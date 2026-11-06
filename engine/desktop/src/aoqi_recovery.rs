//! Independent startup-recovery implementation. The native recovery functions are
//! available as pseudocode, but their complete state format was not recovered.
//! Recovery never deletes account, settings, WebView, shader or game-cache data.

use crate::aoqi_storage::{display_path, Storage};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const STATE_PATH: &str = ".host-state/startup.json";
const LOCK_PATH: &str = ".host-state/instance.lock";

pub struct StartupRecovery {
    storage: Storage,
    // An OS-owned lock is released even on a process crash. The persistent state
    // is a separate, atomically replaced file, so a torn write cannot mean clean.
    _lock: File,
    state: Value,
    pub safe_mode: bool,
    pub previous_unclean: bool,
    pub consecutive_failures: u64,
}

impl StartupRecovery {
    pub fn begin(root: &Path, requested_safe_mode: bool) -> Result<Self, String> {
        let storage = Storage::new(root.to_owned())?;
        storage.call("storage.makeFolder", &[json!(".host-state")])?;
        let lock_path = storage.resolve(LOCK_PATH)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // A second host must not overwrite the first host's startup marker.
            options.share_mode(0);
        }
        let lock = options.open(&lock_path).map_err(|e| {
            format!("Cannot exclusively open startup lock (another host may use this data directory): {e}")
        })?;
        #[cfg(not(windows))]
        lock.try_lock().map_err(|e| format!("Another host may use this data directory: {e}"))?;

        let previous_bytes = storage.call("storage.read", &[json!(STATE_PATH)])?;
        let (previous, corrupt) = match previous_bytes {
            Value::Null => (Value::Null, false),
            Value::String(encoded) => match serde_json::from_slice::<Value>(&STANDARD.decode(&encoded).map_err(|e| e.to_string())?) {
                Ok(value) if value.is_object() => (value, false),
                _ => {
                    let backup = format!(".host-state/startup.corrupt-{}-{}.json", std::process::id(), timestamp_nanos());
                    storage.call("storage.write", &[json!(backup), json!(encoded)])?;
                    (Value::Null, true)
                }
            },
            _ => unreachable!(),
        };
        let previous_unclean = corrupt || (!previous.is_null()
            && !matches!(previous["phase"].as_str(), Some("clean" | "restart-requested")));
        let consecutive_failures = if previous_unclean {
            previous["consecutiveFailures"].as_u64().unwrap_or(0).saturating_add(1)
        } else { 0 };
        let safe_mode = requested_safe_mode || previous_unclean
            || previous["nextSafeMode"].as_bool().unwrap_or(false);
        let state = json!({
            "schema":1,
            "phase":"starting",
            "processId":std::process::id(),
            "startedAt":timestamp_nanos(),
            "safeMode":safe_mode,
            "nextSafeMode":false,
            "previousUnclean":previous_unclean,
            "consecutiveFailures":consecutive_failures,
        });
        storage.write_json(STATE_PATH, &state)?;
        Ok(Self {
            storage,
            _lock: lock,
            state,
            safe_mode,
            previous_unclean,
            consecutive_failures,
        })
    }

    /// Call only after the window, renderer, bridge and host movie are initialized.
    pub fn mark_ready(&mut self) -> Result<(), String> {
        self.set_phase("ready")
    }

    /// The event loop calls this on a normal close, never from Drop/panic cleanup.
    pub fn finish_clean(&mut self) -> Result<(), String> {
        self.state["consecutiveFailures"] = json!(0);
        self.set_phase("clean")
    }

    /// Persist a one-shot safe-mode request before the caller starts a replacement
    /// process. Release this instance's lock by dropping it before starting again.
    pub fn prepare_restart(&mut self, safe_mode: bool) -> Result<(), String> {
        self.state["nextSafeMode"] = json!(safe_mode);
        self.set_phase("restart-requested")
    }

    pub fn summary(&self) -> Value {
        json!({
            "safeMode":self.safe_mode,
            "previousUnclean":self.previous_unclean,
            "consecutiveFailures":self.consecutive_failures,
            "phase":self.state["phase"],
            "statePath":display_path(&self.storage.root().join(STATE_PATH)),
        })
    }

    fn set_phase(&mut self, phase: &str) -> Result<(), String> {
        self.state["phase"] = json!(phase);
        self.state["updatedAt"] = json!(timestamp_nanos());
        self.storage.write_json(STATE_PATH, &self.state)
    }
}

fn timestamp_nanos() -> String {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("aoqi-recovery-test-{}-{}", std::process::id(), NEXT_TEST.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn clean_start_and_exit_does_not_trigger_recovery() {
        let temp = Temp::new();
        let mut first = StartupRecovery::begin(&temp.0, false).unwrap();
        assert!(!first.safe_mode);
        assert!(!first.previous_unclean);
        first.mark_ready().unwrap();
        first.finish_clean().unwrap();
        drop(first);
        let next = StartupRecovery::begin(&temp.0, false).unwrap();
        assert!(!next.safe_mode);
        assert_eq!(next.consecutive_failures, 0);
    }

    #[test]
    fn unclean_exit_enters_safe_mode_without_deleting_user_files() {
        let temp = Temp::new();
        fs::write(temp.0.join("accounts.json"), b"user-owned-test-fixture").unwrap();
        fs::create_dir(temp.0.join("cache")).unwrap();
        fs::write(temp.0.join("cache/resource"), b"retained-cache").unwrap();
        let mut first = StartupRecovery::begin(&temp.0, false).unwrap();
        first.mark_ready().unwrap();
        drop(first); // Simulate an exit that never wrote a clean marker.
        let second = StartupRecovery::begin(&temp.0, false).unwrap();
        assert!(second.safe_mode && second.previous_unclean);
        assert_eq!(second.consecutive_failures, 1);
        assert_eq!(fs::read(temp.0.join("accounts.json")).unwrap(), b"user-owned-test-fixture");
        assert_eq!(fs::read(temp.0.join("cache/resource")).unwrap(), b"retained-cache");
        drop(second);
        let third = StartupRecovery::begin(&temp.0, false).unwrap();
        assert_eq!(third.consecutive_failures, 2);
    }

    #[test]
    fn explicit_restart_safe_mode_is_consumed_once() {
        let temp = Temp::new();
        let mut first = StartupRecovery::begin(&temp.0, false).unwrap();
        first.prepare_restart(true).unwrap();
        first.finish_clean().unwrap();
        drop(first);
        let mut second = StartupRecovery::begin(&temp.0, false).unwrap();
        assert!(second.safe_mode);
        assert!(!second.previous_unclean);
        second.finish_clean().unwrap();
        drop(second);
        let third = StartupRecovery::begin(&temp.0, false).unwrap();
        assert!(!third.safe_mode);
    }

    #[test]
    fn concurrent_host_cannot_overwrite_marker() {
        let temp = Temp::new();
        let _first = StartupRecovery::begin(&temp.0, false).unwrap();
        let state = fs::read(temp.0.join(STATE_PATH)).unwrap();
        assert!(StartupRecovery::begin(&temp.0, false).is_err());
        assert_eq!(fs::read(temp.0.join(STATE_PATH)).unwrap(), state);
    }

    #[test]
    fn corrupt_marker_is_preserved_and_recovers_conservatively() {
        let temp = Temp::new();
        fs::create_dir(temp.0.join(".host-state")).unwrap();
        fs::write(temp.0.join(STATE_PATH), b"{unfinished").unwrap();
        let recovery = StartupRecovery::begin(&temp.0, false).unwrap();
        assert!(recovery.safe_mode && recovery.previous_unclean);
        let backup = fs::read_dir(temp.0.join(".host-state")).unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.file_name().unwrap().to_string_lossy().starts_with("startup.corrupt-")).unwrap();
        assert_eq!(fs::read(backup).unwrap(), b"{unfinished");
    }
}
