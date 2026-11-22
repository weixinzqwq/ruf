use super::{hash, is_link, no_link, public_resource_url, valid_payload, MAGIC, MAX_METADATA, MAX_RESOURCE};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use std::time::{Duration, Instant, SystemTime};
use url::Url;

pub(super) const LOCK_NAME: &str = ".aola-budget.lock";
const SCAN_INTERVAL: Duration = Duration::from_secs(30);

pub(super) fn configured_limit(value: Option<&str>) -> Option<u64> {
    let mib = value?.parse::<u64>().ok()?;
    mib.checked_mul(1024 * 1024).filter(|&bytes| bytes > 0)
}

pub(super) struct DiskBudget {
    lock_path: PathBuf,
    writes: mpsc::SyncSender<()>,
}

#[derive(Default)]
struct ActiveBudgets {
    directories: HashMap<PathBuf, Weak<DiskBudget>>,
    // Keep the current directory across the short gap between player generations.
    latest: Option<Arc<DiskBudget>>,
}

impl DiskBudget {
    pub(super) fn start(entries: &Path, limit: u64) -> io::Result<Arc<Self>> {
        static ACTIVE: OnceLock<Mutex<ActiveBudgets>> = OnceLock::new();
        let mut active = ACTIVE.get_or_init(Default::default).lock().unwrap();
        let lock_path = entries.join(LOCK_NAME);
        let created = match OpenOptions::new().create_new(true).write(true).open(&lock_path) {
            Ok(_) => true,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                no_link(&lock_path)?;
                false
            }
            Err(err) => return Err(err),
        };
        if !fs::symlink_metadata(&lock_path)?.is_file() {
            return Err(io::Error::other("Invalid static cache budget lock"));
        }
        if let Some(budget) = active.directories.get(entries).and_then(Weak::upgrade) {
            if created { let _ = budget.writes.try_send(()); }
            active.latest = Some(budget.clone());
            return Ok(budget);
        }
        active.directories.retain(|_, value| value.strong_count() > 0);
        let (writes, receiver) = mpsc::sync_channel(1);
        let directory = entries.to_owned();
        std::thread::Builder::new().name("aola-cache-budget".into()).spawn(move || {
            // Scan at startup. After writes, coalesce checks across a 30 s window.
            // An idle or read-only directory requires no periodic work.
            loop {
                let retry_busy = match trim(&directory, limit) {
                    Ok(()) => false,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => true,
                    Err(error) => {
                        tracing::warn!("Static cache budget check failed: {error}");
                        false
                    }
                };
                let next = Instant::now() + SCAN_INTERVAL;
                if !retry_busy && receiver.recv().is_err() { break; }
                while let Some(delay) = next.checked_duration_since(Instant::now()) {
                    match receiver.recv_timeout(delay) {
                        Ok(()) => continue,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        })?;
        let budget = Arc::new(Self { lock_path, writes });
        active.directories.insert(entries.to_owned(), Arc::downgrade(&budget));
        active.latest = Some(budget.clone());
        Ok(budget)
    }

    pub(super) fn publish(&self, temporary: &Path, destination: &Path) -> io::Result<bool> {
        // Share the publication lock between writers. A sweep can then recheck
        // and delete its oldest entries without racing an atomic replacement.
        no_link(&self.lock_path)?;
        let gate = File::open(&self.lock_path)?;
        match gate.try_lock_shared() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(false),
            Err(TryLockError::Error(err)) => return Err(err),
        }
        fs::rename(temporary, destination)?;
        drop(gate);
        let _ = self.writes.try_send(());
        Ok(true)
    }
}

fn entry_name(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "cache") &&
        path.file_stem().and_then(|s| s.to_str()).is_some_and(hex64)
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn recognized(path: &Path, length: u64) -> io::Result<bool> {
    // Inspect only the envelope and the resource signature.
    if length > (12 + MAX_METADATA + MAX_RESOURCE) as u64 { return Ok(false); }
    let mut file = File::open(path)?;
    let mut prefix = [0; 12];
    file.read_exact(&mut prefix)?;
    if &prefix[..8] != MAGIC { return Ok(false); }
    let meta_size = u32::from_le_bytes(prefix[8..].try_into().unwrap()) as usize;
    if meta_size > MAX_METADATA { return Ok(false); }
    let mut bytes = vec![0; meta_size];
    file.read_exact(&mut bytes)?;
    let Ok(meta) = serde_json::from_slice::<Value>(&bytes) else { return Ok(false); };
    let Some(size) = meta["size"].as_u64().filter(|&s| s <= MAX_RESOURCE as u64) else { return Ok(false); };
    let Some(url) = meta["url"].as_str().and_then(|s| Url::parse(s).ok()) else { return Ok(false); };
    if !public_resource_url(&url) || meta["status"].as_u64() != Some(200)
        || meta["expiresAt"].as_u64().is_none()
        || !meta["sha256"].as_str().is_some_and(hex64)
        || !meta.get("contentType").is_some_and(|v| v.is_null() || v.is_string())
        || !meta["responseUrl"].as_str().and_then(|s| Url::parse(s).ok()).as_ref().is_some_and(public_resource_url)
        || length != 12 + meta_size as u64 + size
        || path.file_stem().and_then(|s| s.to_str()) != Some(hash(url.as_str().as_bytes()).as_str()) {
        return Ok(false);
    }
    let count = size.min(prefix.len() as u64) as usize;
    file.read_exact(&mut prefix[..count])?;
    Ok(valid_payload(&url, &prefix[..count]))
}

fn visit(entries: &Path, mut entry: impl FnMut(PathBuf, fs::Metadata)) -> io::Result<()> {
    no_link(entries.parent().ok_or_else(|| io::Error::other("Missing cache root"))?)?;
    no_link(entries)?;
    for item in fs::read_dir(entries)? {
        let path = item?.path();
        if !entry_name(&path) { continue; }
        if let Ok(metadata) = fs::symlink_metadata(&path) && metadata.is_file() && !is_link(&metadata) {
            entry(path, metadata);
        }
    }
    Ok(())
}

fn trim(entries: &Path, limit: u64) -> io::Result<()> {
    // Below budget only filenames and filesystem metadata are inspected.
    let mut upper_bound = 0u64;
    visit(entries, |_, m| upper_bound = upper_bound.saturating_add(m.len()))?;
    if upper_bound <= limit { return Ok(()); }

    let lock_path = entries.join(LOCK_NAME);
    no_link(&lock_path)?;
    let gate = File::open(lock_path)?;
    match gate.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Err(io::ErrorKind::WouldBlock.into()),
        Err(TryLockError::Error(err)) => return Err(err),
    }
    // Another process may have reclaimed before this lock. Recount only entries
    // belonging to this cache; legacy and malformed files do not consume its budget.
    let mut owned = Vec::<(SystemTime, PathBuf, u64)>::new();
    let mut total = 0u64;
    visit(entries, |path, metadata| {
        if let Ok(modified) = metadata.modified()
            && recognized(&path, metadata.len()).unwrap_or(false) {
            total = total.saturating_add(metadata.len());
            owned.push((modified, path, metadata.len()));
        }
    })?;
    if total <= limit { return Ok(()); }
    owned.sort_unstable();
    let before = total;
    let target = limit / 5 * 4 + limit % 5 * 4 / 5;
    for (modified, path, size) in owned {
        if total <= target { break; }
        no_link(entries.parent().unwrap())?;
        no_link(entries)?;
        if no_link(&path).is_ok()
            && let Ok(current) = fs::symlink_metadata(&path)
            && current.len() == size && current.modified().ok() == Some(modified)
            && fs::remove_file(path).is_ok() {
            total -= size;
        }
    }
    tracing::info!(target: "aola_cache_budget", before, after=total, limit, "Trimmed public resource cache");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::AoqiCache;
    use reqwest::header::HeaderMap;

    fn store(cache: &AoqiCache, id: u8) -> (Url, PathBuf) {
        let url = Url::parse(&format!("https://aola.100bt.com/play/test~{id}.swf")).unwrap();
        let policy = AoqiCache::response_policy(&url, &url, 200, &HeaderMap::new()).unwrap();
        assert!(cache.store(&url, &policy, b"FWS\x09\x08\0\0\0").unwrap());
        let file = cache.entry_path(&url);
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1700000000 + u64::from(id));
        OpenOptions::new().write(true).open(&file).unwrap().set_modified(mtime).unwrap();
        (url, file)
    }

    #[test]
    fn aola_disk_budget_is_opt_in_and_overflow_safe() {
        for value in [None, Some("0"), Some("-1"), Some("x"), Some("18446744073709551615")] {
            assert_eq!(configured_limit(value), None);
        }
        assert_eq!(configured_limit(Some("4096")), Some(4 * 1024u64.pow(3)));
    }

    #[test]
    fn aola_disk_budget_trims_oldest_owned_entries_only() {
        let temp = tempfile::tempdir().unwrap();
        let cache = AoqiCache::new(temp.path().join("cache")).unwrap();
        let mut files = Vec::new();
        for id in 0..5 { files.push(store(&cache, id).1); }
        let size = fs::metadata(&files[0]).unwrap().len();
        let partial = files[0].with_extension("partial-test");
        let renamed = cache.entries.join(format!("{}.cache", "a".repeat(64)));
        let unknown = cache.entries.join(format!("{}.cache", "b".repeat(64)));
        let other = cache.root.join("webview").join(files[0].file_name().unwrap());
        fs::create_dir(other.parent().unwrap()).unwrap();
        for file in [&partial, &renamed, &other] { fs::copy(&files[0], file).unwrap(); }
        fs::write(&unknown, vec![0; size as usize]).unwrap();
        File::create(cache.entries.join(LOCK_NAME)).unwrap();
        // Retain the correct URL-derived filename so this symlink would pass
        // the cache envelope checks if the scanner were to follow it.
        let (_, link) = store(&cache, 6);
        let outside = temp.path().join("outside.bin");
        fs::rename(&link, &outside).unwrap();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(windows)]
        let linked = match std::os::windows::fs::symlink_file(&outside, &link) {
            Ok(()) => true,
            Err(e) if e.raw_os_error() == Some(1314) || e.kind() == io::ErrorKind::PermissionDenied => false,
            Err(e) => panic!("Cannot prepare cache symlink fixture: {e}"),
        };
        #[cfg(not(any(windows, unix)))]
        let linked = false;
        // The unrecognized files alone must never trigger eviction.
        trim(&cache.entries, 5 * size).unwrap();
        assert!(files.iter().all(|p| p.exists()));
        let _budget = DiskBudget::start(&cache.entries, 4 * size).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while files[1].exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!files[0].exists() && !files[1].exists());
        assert!(files[2..].iter().all(|p| p.exists()));
        assert!([partial, renamed, unknown, other].iter().all(|p| p.exists()));
        assert!(outside.exists());
        if linked { assert!(link.symlink_metadata().unwrap().is_symlink()); }
    }

    #[test]
    fn aola_disk_budget_cross_instance_gate_keeps_published_files_whole() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache = AoqiCache::new(temp.path().join("cache")).unwrap();
        let budget = DiskBudget::start(&cache.entries, u64::MAX).unwrap();
        let second = DiskBudget::start(&cache.entries, u64::MAX).unwrap();
        assert!(Arc::ptr_eq(&budget, &second));
        cache.budget = Some(budget.clone());
        let (url, file) = store(&cache, 1);
        let gate = File::open(&budget.lock_path).unwrap();
        gate.lock_shared().unwrap();
        assert_eq!(trim(&cache.entries, 1).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        drop(gate);
        let gate = File::open(&budget.lock_path).unwrap();
        gate.lock().unwrap();
        let policy = AoqiCache::response_policy(&url, &url, 200, &HeaderMap::new()).unwrap();
        assert!(!cache.store(&url, &policy, b"CWS\x09\x08\0\0\0").unwrap());
        assert_eq!(cache.lookup(&url).unwrap().unwrap().bytes[0], b'F');
        assert_eq!(fs::read_dir(&cache.entries).unwrap().count(), 2);
        drop(gate);
        assert!(cache.store(&url, &policy, b"CWS\x09\x08\0\0\0").unwrap());
        assert_eq!(cache.lookup(&url).unwrap().unwrap().bytes[0], b'C');
        trim(&cache.entries, 1).unwrap();
        assert!(!file.exists());
    }
}
