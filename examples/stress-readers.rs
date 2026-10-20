//! Bounded, process-level stored-query contention experiment.
//! See scripts/concurrent-readers.md for the workload and evidence format.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy)]
struct Limits {
    queries: usize,
    builds: usize,
    seconds: u64,
}

struct ChildGuard(Child);

struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Capture files keep a child from blocking on a full output pipe. All bytes are
/// recorded as arrays if they are not UTF-8, rather than replaced in diagnostics.
fn bytes(bytes: Vec<u8>) -> Value {
    match String::from_utf8(bytes) {
        Ok(text) => json!(text),
        Err(error) => json!({"bytes": error.into_bytes()}),
    }
}

fn command(binary: &Path, root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(binary);
    command
        .current_dir(root)
        .args(args)
        .env_remove("INCDIR")
        .env_remove("INCLUDEDIRS")
        .env_remove("RIPSCOPE_BLOCK_SIZE")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    command
}

fn capture(mut command: Command, root: &Path, deadline: Instant) -> Result<Value> {
    let stdout = tempfile::NamedTempFile::new_in(root)?;
    let stderr = tempfile::NamedTempFile::new_in(root)?;
    command.stdout(stdout.reopen()?).stderr(stderr.reopen()?);
    let started = Instant::now();
    let mut child = ChildGuard(command.spawn()?);
    let mut stopped = None;
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            stopped = Some("deadline");
        } else if stdout.as_file().metadata()?.len() + stderr.as_file().metadata()?.len() > 1 << 20
        {
            stopped = Some("output_limit");
        }
        if stopped.is_some() {
            child.0.kill()?;
            break child.0.wait()?;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    Ok(json!({
        "status": status.code(), "success": status.success() && stopped.is_none(),
        "stopped": stopped, "milliseconds": started.elapsed().as_secs_f64() * 1000.0,
        "stdout": bytes(fs::read(stdout.path())?), "stderr": bytes(fs::read(stderr.path())?),
    }))
}

fn record(file: &mut File, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn expected(version: usize) -> String {
    format!("a.c <global> 1 int Epoch_{version};\na.c <global> 2 int Epoch_{version}_tail;\n")
}

fn source(root: &Path, version: usize) -> Result<()> {
    fs::write(
        root.join("a.c"),
        format!("int Epoch_{version};\nint Epoch_{version}_tail;\n"),
    )?;
    Ok(())
}

fn answer_ok(result: &Value) -> bool {
    result["success"] == true
        && result["stderr"] == ""
        && (result["stdout"] == expected(0) || result["stdout"] == expected(1))
}

fn percentile(values: &[f64], percent: usize) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(sorted[((sorted.len() - 1) * percent) / 100])
}

fn scenario(binary: &Path, directory: &Path, forced: bool, limits: Limits) -> Result<Value> {
    fs::create_dir(directory)?;
    let query_args = ["-dL", "-f", "i.db", "-0", "^Epoch_[01].*$"];
    source(directory, 0)?;
    let mut setup_log = File::create(directory.join("setup.jsonl"))?;
    let setup = capture(
        command(binary, directory, &["-bk", "-f", "i.db", "a.c"]),
        directory,
        Instant::now() + Duration::from_secs(15),
    )?;
    record(&mut setup_log, &setup)?;
    if setup["success"] != true || setup["stderr"] != "" {
        return Err(io::Error::other("initial publication failed; see setup.jsonl").into());
    }
    let initial = capture(
        command(binary, directory, &query_args),
        directory,
        Instant::now() + Duration::from_secs(5),
    )?;
    record(&mut setup_log, &initial)?;
    if !answer_ok(&initial) || initial["stdout"] != expected(0) {
        return Err(io::Error::other("initial answer differs from the labelled fixture").into());
    }
    // Check the published revision without keeping a connection that would change
    // the last-close contention being investigated.
    let revision = |root: &Path| -> Result<i64> {
        let connection = rusqlite::Connection::open(root.join("i.db"))?;
        Ok(
            connection.query_row("SELECT value FROM meta WHERE key = 'revision'", [], |row| {
                row.get(0)
            })?,
        )
    };
    let initial_revision = revision(directory)?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(limits.seconds);
    let stop = Arc::new(AtomicBool::new(false));
    let writing = Arc::new(AtomicBool::new(false));
    let reader_stop = stop.clone();
    let reader_writing = writing.clone();
    let query_root = directory.to_owned();
    let query_binary = binary.to_owned();
    let mut log = File::create(directory.join("builds.jsonl"))?;
    let reader = std::thread::spawn(move || -> Result<Value> {
        let _stop_on_error = StopOnDrop(reader_stop.clone());
        let mut log = File::create(query_root.join("queries.jsonl"))?;
        let mut count = 0;
        let mut overlap = 0;
        let mut failures = 0;
        let mut latency = Vec::new();
        while count < limits.queries
            && !reader_stop.load(Ordering::SeqCst)
            && Instant::now() < deadline
        {
            let active_before = reader_writing.load(Ordering::SeqCst);
            let mut result = capture(
                command(&query_binary, &query_root, &query_args),
                &query_root,
                deadline.min(Instant::now() + Duration::from_secs(5)),
            )?;
            let active_after = reader_writing.load(Ordering::SeqCst);
            let valid = answer_ok(&result);
            count += 1;
            overlap += usize::from(active_before || active_after);
            failures += usize::from(!valid);
            latency.push(result["milliseconds"].as_f64().expect("elapsed time"));
            result["number"] = json!(count);
            result["answer_ok"] = json!(valid);
            result["writer_process_active_before"] = json!(active_before);
            result["writer_process_active_after"] = json!(active_after);
            record(&mut log, &result)?;
            if !valid {
                reader_stop.store(true, Ordering::SeqCst);
                break;
            }
        }
        reader_stop.store(true, Ordering::SeqCst);
        Ok(json!({
            "count": count, "failures": failures, "writer_process_overlap": overlap,
            "latency_ms": {"p50": percentile(&latency, 50), "p95": percentile(&latency, 95),
                "p99": percentile(&latency, 99), "max": percentile(&latency, 100)},
        }))
    });
    let mut count = 0;
    let mut failures = 0;
    let writer_result = (|| -> Result<()> {
        while count < limits.builds && !stop.load(Ordering::SeqCst) && Instant::now() < deadline {
            source(directory, (count + 1) % 2)?;
            writing.store(true, Ordering::SeqCst);
            let mut result = capture(
                command(
                    binary,
                    directory,
                    &[if forced { "-buk" } else { "-bk" }, "-f", "i.db", "a.c"],
                ),
                directory,
                deadline.min(Instant::now() + Duration::from_secs(15)),
            )?;
            writing.store(false, Ordering::SeqCst);
            count += 1;
            result["number"] = json!(count);
            result["source_version"] = json!(count % 2);
            let valid = result["success"] == true && result["stderr"] == "";
            failures += usize::from(!valid);
            record(&mut log, &result)?;
            if !valid {
                break;
            }
        }
        Ok(())
    })();
    stop.store(true, Ordering::SeqCst);
    writing.store(false, Ordering::SeqCst);
    let queries = reader
        .join()
        .map_err(|_| io::Error::other("query thread panicked"))??;
    writer_result?;
    let final_revision = revision(directory)?;
    let summary = json!({
        "scenario": if forced { "forced" } else { "ordinary" },
        "query_args": query_args, "build_option": if forced { "-buk" } else { "-bk" },
        "limits": {"queries": limits.queries, "builds": limits.builds, "seconds": limits.seconds,
            "query_timeout_seconds": 5, "build_timeout_seconds": 15},
        "queries": queries, "builds": {"count": count, "failures": failures},
        "revision_before": initial_revision, "revision_after": final_revision,
        "publications": final_revision - initial_revision,
        "seconds": started.elapsed().as_secs_f64(),
        "stop": if queries["failures"] != 0 || failures != 0 { "failure" }
            else if count == limits.builds { "build_limit" }
            else if queries["count"] == limits.queries { "query_limit" } else { "time_limit" },
    });
    fs::write(
        directory.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    Ok(summary)
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: stress-readers BINARY NEW_EVIDENCE_DIRECTORY [QUERIES BUILDS SECONDS]";
    let binary = fs::canonicalize(args.next().ok_or_else(|| io::Error::other(usage))?)?;
    let evidence = std::path::absolute(PathBuf::from(
        args.next().ok_or_else(|| io::Error::other(usage))?,
    ))?;
    let remaining = args.collect::<Vec<_>>();
    let limits = match remaining.len() {
        0 => Limits {
            queries: 10_000,
            builds: 1_000,
            seconds: 180,
        },
        3 => Limits {
            queries: remaining[0].to_string_lossy().parse()?,
            builds: remaining[1].to_string_lossy().parse()?,
            seconds: remaining[2].to_string_lossy().parse()?,
        },
        _ => return Err(io::Error::other(usage).into()),
    };
    if limits.queries == 0 || limits.builds == 0 || limits.seconds == 0 {
        return Err(io::Error::other("all bounds must be positive").into());
    }
    fs::create_dir(&evidence)?;
    let version = capture(
        command(&binary, &evidence, &["-V"]),
        &evidence,
        Instant::now() + Duration::from_secs(5),
    )?;
    let metadata = json!({
        "started_unix_seconds": SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs(),
        "os": std::env::consts::OS, "architecture": std::env::consts::ARCH,
        "binary": binary, "binary_sha256": format!("{:x}", Sha256::digest(fs::read(&binary)?)),
        "binary_version": version, "harness_sqlite_version": rusqlite::version(),
        "harness_sqlite_version_number": rusqlite::version_number(),
        "harness_version": env!("CARGO_PKG_VERSION"),
        "sqlite_identity_note": "Build the binary and example together to identify the binary's bundled SQLite version.",
        "expected_answers": [expected(0), expected(1)],
    });
    fs::write(
        evidence.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    println!(
        "Each scenario stops at {} queries, {} index builds or {} seconds; queries have a 5 s deadline and index builds 15 s.",
        limits.queries, limits.builds, limits.seconds
    );
    let mut failures = false;
    for forced in [false, true] {
        let summary = scenario(
            &binary,
            &evidence.join(if forced { "forced" } else { "ordinary" }),
            forced,
            limits,
        )?;
        println!("{}", serde_json::to_string(&summary)?);
        failures |= summary["queries"]["failures"] != 0 || summary["builds"]["failures"] != 0;
    }
    println!("Evidence: {}", evidence.display());
    if failures {
        Err(
            io::Error::other("contention experiment captured a failure; inspect the evidence")
                .into(),
        )
    } else {
        Ok(())
    }
}
