//! Opt-in local metrics. Never contains requests, projections, or authentication material.
use serde_json::{json, Value};
use std::{
    io::{BufWriter, Write},
    ops::{Deref, DerefMut},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, OnceLock,
    },
    time::{Duration, Instant},
};

const CAPACITY: usize = 8192;
static TRACE: OnceLock<Arc<Trace>> = OnceLock::new();
struct Trace {
    sender: mpsc::SyncSender<Option<Value>>,
    dropped: AtomicU64,
    accepting: AtomicBool,
    gate: std::sync::RwLock<()>,
    started: Instant,
}
pub(super) struct Writer {
    trace: Arc<Trace>,
    thread: Option<std::thread::JoinHandle<std::io::Result<()>>>,
}
impl Writer {
    fn start(path: &Path, capacity: usize) -> anyhow::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let trace = Arc::new(Trace {
            sender,
            dropped: AtomicU64::new(0),
            accepting: AtomicBool::new(true),
            gate: std::sync::RwLock::new(()),
            started: Instant::now(),
        });
        let state = trace.clone();
        let thread = std::thread::spawn(move || {
            let mut file = BufWriter::new(file);
            let mut written = 0u64;
            loop {
                match receiver.recv_timeout(Duration::from_secs(1)) {
                    Ok(Some(value)) => {
                        serde_json::to_writer(&mut file, &value)?;
                        file.write_all(b"\n")?;
                        written += 1;
                    }
                    Ok(None) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => file.flush()?,
                }
                // Continuous traffic must not prevent periodic persistence.
                if written.is_multiple_of(128) {
                    file.flush()?;
                }
            }
            serde_json::to_writer(
                &mut file,
                &json!({"schema_version": 1, "kind": "trace_end", "written": written,
                "dropped": state.dropped.load(Ordering::Relaxed)}),
            )?;
            file.write_all(b"\n")?;
            file.flush()
        });
        Ok(Self {
            trace,
            thread: Some(thread),
        })
    }
    pub fn finish(mut self) -> anyhow::Result<()> {
        // Wait for any in-flight enqueue before putting the end marker on the channel.
        let gate = self.trace.gate.write().unwrap();
        self.trace.accepting.store(false, Ordering::Relaxed);
        let sent = self.trace.sender.send(None);
        drop(gate);
        let result = self
            .thread
            .take()
            .unwrap()
            .join()
            .map_err(|_| anyhow::anyhow!("performance writer panicked"))?;
        result?;
        sent.map_err(|_| anyhow::anyhow!("performance writer disconnected"))?;
        Ok(())
    }
}
impl Trace {
    fn record(&self, make: impl FnOnce() -> Value) {
        let _gate = self.gate.read().unwrap();
        if !self.accepting.load(Ordering::Relaxed) {
            return;
        }
        let mut value = make();
        value["schema_version"] = json!(1);
        value["elapsed_ms"] = json!(self.started.elapsed().as_secs_f64() * 1000.0);
        if self.sender.try_send(Some(value)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
pub(super) fn start() -> anyhow::Result<Option<Writer>> {
    let Some(directory) =
        std::env::var_os("PERFORMANCE_TRACE_DIR").filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    std::fs::create_dir_all(&directory)?;
    let writer = Writer::start(&Path::new(&directory).join("server.jsonl"), CAPACITY)?;
    TRACE
        .set(writer.trace.clone())
        .map_err(|_| anyhow::anyhow!("performance trace already initialized"))?;
    record(
        || json!({"kind": "trace_start", "capacity": CAPACITY, "pid": std::process::id(), "unix_ms": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64() * 1000.0}),
    );
    Ok(Some(writer))
}
pub(super) fn record(make: impl FnOnce() -> Value) {
    if let Some(trace) = TRACE.get() {
        trace.record(make);
    }
}
pub(super) fn enabled() -> bool {
    TRACE.get().is_some()
}

/// Instruments every acquisition of the shared games lock, including early returns.
/// Caller locations are static source locations, never user-controlled strings.
pub(super) struct TimedRwLock<T>(tokio::sync::RwLock<T>);
impl<T> TimedRwLock<T> {
    pub fn new(value: T) -> Self {
        Self(tokio::sync::RwLock::new(value))
    }
    #[track_caller]
    pub fn read(
        &self,
    ) -> impl std::future::Future<Output = Guard<tokio::sync::RwLockReadGuard<'_, T>>> {
        let caller = std::panic::Location::caller();
        async move {
            let started = enabled().then(Instant::now);
            let inner = self.0.read().await;
            Guard::new(inner, started, "read", caller)
        }
    }
    #[track_caller]
    pub fn write(
        &self,
    ) -> impl std::future::Future<Output = Guard<tokio::sync::RwLockWriteGuard<'_, T>>> {
        let caller = std::panic::Location::caller();
        async move {
            let started = enabled().then(Instant::now);
            let inner = self.0.write().await;
            Guard::new(inner, started, "write", caller)
        }
    }
}
pub(super) struct Guard<T> {
    inner: T,
    acquired: Option<Instant>,
    wait_ms: f64,
    mode: &'static str,
    caller: &'static std::panic::Location<'static>,
}
impl<T> Guard<T> {
    fn new(
        inner: T,
        started: Option<Instant>,
        mode: &'static str,
        caller: &'static std::panic::Location<'static>,
    ) -> Self {
        Self {
            inner,
            acquired: started.map(|_| Instant::now()),
            wait_ms: started.map_or(0.0, |s| s.elapsed().as_secs_f64() * 1000.0),
            mode,
            caller,
        }
    }
}
impl<T: Deref> Deref for Guard<T> {
    type Target = T::Target;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl<T: DerefMut> DerefMut for Guard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
impl<T> Drop for Guard<T> {
    fn drop(&mut self) {
        if let Some(acquired) = self.acquired {
            record(|| {
                json!({"kind": "game_lock", "mode": self.mode, "caller": format!("{}:{}", self.caller.file(), self.caller.line()),
                "wait_ms": self.wait_ms, "hold_ms": acquired.elapsed().as_secs_f64() * 1000.0})
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_is_counted_without_blocking() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let trace = Trace {
            sender,
            dropped: AtomicU64::new(0),
            accepting: AtomicBool::new(true),
            gate: std::sync::RwLock::new(()),
            started: Instant::now(),
        };
        trace.record(|| json!({"kind": "test"}));
        trace.record(|| json!({"kind": "test"}));
        assert_eq!(trace.dropped.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn writer_flushes_complete_versioned_trace_and_refuses_overwrite() {
        let path =
            std::env::temp_dir().join(format!("waw-performance-{}.jsonl", uuid::Uuid::new_v4()));
        let writer = Writer::start(&path, 8).unwrap();
        writer
            .trace
            .record(|| json!({"kind": "test", "duration_ms": 1.0}));
        assert!(Writer::start(&path, 8).is_err());
        writer.finish().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let records: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["schema_version"], 1);
        assert_eq!(records[1]["written"], 1);
        assert_eq!(records[1]["dropped"], 0);
    }
}
