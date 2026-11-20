//! Optional JSONL logging. The caller supplies already sanitized packet data.
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ROWS: usize = 256;
const MAX_BYTES: usize = 8 * 1024 * 1024;

pub struct PacketLog {
    path: PathBuf,
    sender: Option<SyncSender<Box<str>>>,
    worker: Option<JoinHandle<()>>,
    bytes: Arc<AtomicUsize>,
    error: Arc<Mutex<Option<String>>>,
    dropped: u64,
}

impl PacketLog {
    pub fn start(root: &Path) -> io::Result<Self> {
        let directory = root.join("logs");
        fs::create_dir_all(&directory)?;
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(io::Error::other)?.as_nanos();
        let path = directory.join(format!("aola-packets-{timestamp}-{}.jsonl", std::process::id()));
        let mut file = File::options().write(true).create_new(true).open(&path)?;
        let (sender, receiver) = mpsc::sync_channel::<Box<str>>(MAX_ROWS);
        let bytes = Arc::new(AtomicUsize::new(0));
        let error = Arc::new(Mutex::new(None));
        let pending_bytes = bytes.clone();
        let worker_error = error.clone();
        let worker = thread::Builder::new().name("aola-packet-log".into()).spawn(move || {
            for line in receiver {
                let result = writeln!(file, "{line}");
                pending_bytes.fetch_sub(line.len() + 1, Ordering::Relaxed);
                if let Err(error) = result {
                    *worker_error.lock().unwrap() = Some(format!("日志写入失败：{error}"));
                    return;
                }
            }
            if let Err(error) = file.flush() {
                *worker_error.lock().unwrap() = Some(format!("日志写入失败：{error}"));
            }
        })?;
        Ok(Self { path, sender: Some(sender), worker: Some(worker), bytes, error, dropped: 0 })
    }

    /// This never waits for disk I/O. The cap counts encoded bytes plus newlines.
    pub fn record(&mut self, row: serde_json::Value) -> Result<(), String> {
        if let Some(error) = self.error.lock().unwrap().clone() {
            return Err(error);
        }
        let sender = self.sender.as_ref().ok_or("日志已经停止")?;
        let line = serde_json::to_string(&row).map_err(|error| format!("日志编码失败：{error}"))?.into_boxed_str();
        let size = line.len() + 1;
        if self.bytes.try_update(Ordering::Relaxed, Ordering::Relaxed, |pending| {
            pending.checked_add(size).filter(|total| *total <= MAX_BYTES)
        }).is_err() {
            self.dropped += 1;
            return Err(format!("日志队列已满，累计丢弃 {} 条", self.dropped));
        }
        match sender.try_send(line) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.bytes.fetch_sub(size, Ordering::Relaxed);
                match error {
                    TrySendError::Full(_) => {
                        self.dropped += 1;
                        Err(format!("日志队列已满，累计丢弃 {} 条", self.dropped))
                    }
                    TrySendError::Disconnected(_) => Err(self.error.lock().unwrap().clone().unwrap_or_else(|| "日志写入线程已停止".into())),
                }
            }
        }
    }

    pub fn path(&self) -> &Path { &self.path }
    pub fn dropped(&self) -> u64 { self.dropped }

    /// Closing the sender drains retained rows before the worker exits.
    pub fn stop(&mut self) -> Result<(), String> {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                return Err("日志写入线程异常结束".into());
            }
        }
        match self.error.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for PacketLog {
    fn drop(&mut self) { let _ = self.stop(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aola_log_drains_on_stop_and_rejects_oversized_rows() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("aola-log-test-{stamp}-{}", std::process::id()));
        let mut logger = PacketLog::start(&root).unwrap();
        let path = logger.path().to_owned();
        logger.record(json!({"command":"test", "message":"中文"})).unwrap();
        assert!(logger.record(json!({"oversized":"x".repeat(MAX_BYTES)})).is_err());
        assert_eq!(logger.dropped(), 1);
        logger.record(json!({"command":"last"})).unwrap();
        logger.stop().unwrap();
        let content = fs::read_to_string(path).unwrap();
        let rows: Vec<serde_json::Value> = content.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(rows, vec![json!({"command":"test", "message":"中文"}), json!({"command":"last"})]);
        assert!(logger.record(json!({"after":"stop"})).is_err());
        assert_eq!(logger.bytes.load(Ordering::Relaxed), 0);
        drop(logger);
        assert!(root.canonicalize().unwrap().starts_with(std::env::temp_dir().canonicalize().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }
}
