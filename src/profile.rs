//! Opt-in Session diagnostics. Normal builds discard every hook at compile time.
macro_rules! profile_span {
    ($name:expr) => {
        #[cfg(feature = "session-profile")]
        let _profile_span = crate::profile::Span::new($name);
    };
}

macro_rules! profile_count {
    ($name:expr, $count:expr) => {
        #[cfg(feature = "session-profile")]
        crate::profile::count($name, $count as u64);
    };
}

#[cfg(feature = "session-profile")]
mod enabled {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::fs::{File, OpenOptions};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    #[derive(Default)]
    struct Metric {
        ns: u128,
        calls: u64,
        count: u64,
    }

    thread_local! {
        static METRICS: RefCell<BTreeMap<&'static str, Metric>> = const { RefCell::new(BTreeMap::new()) };
    }
    // A Session serves one request at a time and joins all walkers before the next.
    static REQUEST: AtomicU64 = AtomicU64::new(0);

    fn logger() -> Option<&'static Mutex<File>> {
        static LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();
        LOG.get_or_init(|| {
            std::env::var_os("RIPSCOPE_PROFILE").map(|path| {
                Mutex::new(
                    OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .expect("cannot open RIPSCOPE_PROFILE diagnostic file"),
                )
            })
        })
        .as_ref()
    }

    pub(crate) struct Span {
        name: &'static str,
        started: Instant,
    }

    impl Span {
        pub(crate) fn new(name: &'static str) -> Option<Self> {
            logger().map(|_| Self {
                name,
                started: Instant::now(),
            })
        }
    }

    impl Drop for Span {
        fn drop(&mut self) {
            let ns = self.started.elapsed().as_nanos();
            METRICS.with_borrow_mut(|metrics| {
                let metric = metrics.entry(self.name).or_default();
                metric.ns += ns;
                metric.calls += 1;
            });
        }
    }

    pub(crate) fn count(name: &'static str, count: u64) {
        if logger().is_some() {
            METRICS.with_borrow_mut(|metrics| metrics.entry(name).or_default().count += count);
        }
    }

    /// Write after work, once per request/thread, avoiding protocol streams and row logs.
    pub(crate) fn flush() {
        let Some(log) = logger() else { return };
        let metrics = METRICS.with_borrow_mut(std::mem::take);
        if metrics.is_empty() {
            return;
        }
        let mut buffer = Vec::new();
        for (phase, metric) in metrics {
            let row = serde_json::json!({
                "pid": std::process::id(), "request": REQUEST.load(Ordering::Relaxed),
                "thread": format!("{:?}", std::thread::current().id()),
                "phase": phase, "ns": metric.ns, "calls": metric.calls, "count": metric.count,
            });
            writeln!(buffer, "{row}").expect("diagnostic buffer");
        }
        log.lock()
            .expect("diagnostic log lock")
            .write_all(&buffer)
            .expect("cannot write RIPSCOPE_PROFILE diagnostics");
    }

    pub(crate) struct Operation(Option<Span>);

    impl Operation {
        pub(crate) fn start(name: &'static str, request: bool) -> Self {
            if request && logger().is_some() {
                REQUEST.fetch_add(1, Ordering::Relaxed);
            }
            Self(Span::new(name))
        }
    }

    impl Drop for Operation {
        fn drop(&mut self) {
            drop(self.0.take());
            flush();
        }
    }
}

#[cfg(feature = "session-profile")]
pub(crate) use enabled::*;
