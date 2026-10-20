//! A loopback release mirror that serves exact archive bytes, as the installers'
//! `RIPSCOPE_RELEASES` setting expects: `/latest` redirects to the version's tag, and
//! each archive has its `.sha256` sidecar.
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub struct Mirror {
    pub url: String,
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    stopped: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Mirror {
    pub fn start(version: &str) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        let files = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (responses, finish, location) = (
            files.clone(),
            stopped.clone(),
            format!("{url}/tag/v{version}"),
        );
        let worker = std::thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut stream) = connection else { break };
                if finish.load(Ordering::SeqCst) {
                    break;
                }
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let mut request = Vec::new();
                let mut byte = [0];
                while request.len() < 16384 && !request.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).ok() != Some(1) {
                        break;
                    }
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap_or("");
                if path == "/latest" {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                } else {
                    let bytes = if path.starts_with("/tag/") {
                        Some(Vec::new())
                    } else {
                        responses.lock().unwrap().get(path).cloned()
                    };
                    let (status, bytes) =
                        bytes.map_or(("404 Not Found", Vec::new()), |data| ("200 OK", data));
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    if !request.starts_with("HEAD ") {
                        let _ = stream.write_all(&bytes);
                    }
                }
            }
        });
        Ok(Self {
            url,
            files,
            stopped,
            worker: Some(worker),
        })
    }
    pub fn archive(&self, version: &str, name: &str, bytes: &[u8], valid_checksum: bool) {
        let path = format!("/download/v{version}/{name}");
        let digest = if valid_checksum {
            hash(bytes)
        } else {
            "0".repeat(64)
        };
        let mut files = self.files.lock().unwrap();
        files.insert(path.clone(), bytes.to_vec());
        files.insert(
            format!("{path}.sha256"),
            format!("{digest}  {name}\n").into_bytes(),
        );
    }
}
impl Drop for Mirror {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
