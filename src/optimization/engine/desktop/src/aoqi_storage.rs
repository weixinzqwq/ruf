//! Reconstructed storage bridge. The AS contract is recovered; directory listing and
//! migration policy below are independent implementations, not recovered Rust source.

use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&root).map_err(error)?;
        reject_link(&root)?;
        Ok(Self { root: fs::canonicalize(root).map_err(error)? })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Accept absolute paths produced by Storage.as, but keep all access inside root.
    pub fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let input = Path::new(path);
        validate_path(input)?;
        if input.has_root() && !input.is_absolute() {
            return Err("A rooted path must include its drive".into());
        }
        let requested = if input.is_absolute() { input.to_owned() } else { self.root.join(input) };
        let resolved = resolve_ancestor(&requested)?;
        if !resolved.starts_with(&self.root) {
            return Err("Path is outside the application storage root".into());
        }
        let mut current = self.root.clone();
        reject_link(&current)?;
        for component in resolved.strip_prefix(&self.root).map_err(error)?.components() {
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(_) => reject_link(&current)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(error(e)),
            }
        }
        // Check the original spelling as well: canonicalization must not hide a link.
        let mut current = PathBuf::new();
        for component in requested.components() {
            current.push(component);
            if let Ok(canonical) = fs::canonicalize(&current) {
                if canonical.starts_with(&self.root) {
                    reject_link(&current)?;
                }
            }
        }
        Ok(resolved)
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Result<Value, String> {
        if name == "storage.root" {
            return Ok(json!(display_path(&self.root)));
        }
        if name == "storage.resource" || name == "storage.resourceText" {
            return match aoqi_assets::get(argument(args, 0)?) {
                None => Ok(Value::Null),
                Some(asset) if name == "storage.resource" => Ok(json!(STANDARD.encode(asset.bytes))),
                Some(asset) => Ok(json!(std::str::from_utf8(asset.bytes).map_err(error)?)),
            };
        }
        let path = self.resolve(argument(args, 0)?)?;
        match name {
            "storage.read" | "storage.readText" => {
                let bytes = match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
                    Err(e) => return Err(error(e)),
                };
                if name == "storage.read" { Ok(json!(STANDARD.encode(bytes))) }
                else { Ok(json!(String::from_utf8(bytes).map_err(error)?)) }
            }
            "storage.write" => {
                let bytes = STANDARD.decode(argument(args, 1)?).map_err(error)?;
                self.write(&path, &bytes)?;
                Ok(json!(true))
            }
            "storage.writeText" => {
                let text = argument(args, 1)?;
                let append = match args.get(2) {
                    Some(Value::Bool(value)) => *value,
                    None => false,
                    _ => return Err("append must be a boolean".into()),
                };
                let mut bytes = if append {
                    match fs::read(&path) {
                        Ok(bytes) => bytes,
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                        Err(e) => return Err(error(e)),
                    }
                } else { Vec::new() };
                bytes.extend_from_slice(text.as_bytes());
                self.write(&path, &bytes)?;
                Ok(json!(true))
            }
            "storage.info" => match fs::metadata(&path) {
                Ok(metadata) => Ok(json!({
                    "exists": true,
                    "isDirectory": metadata.is_dir(),
                    "isFile": metadata.is_file(),
                    "size": metadata.len(),
                    "modified": metadata.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|t| t.as_millis() as u64),
                    "name": path.file_name().map(|v| v.to_string_lossy().into_owned()),
                    "path": display_path(&path),
                })),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({"exists":false})),
                Err(e) => Err(error(e)),
            },
            "storage.list" => {
                if !path.exists() { return Ok(json!([])); }
                let mut names = Vec::new();
                for entry in fs::read_dir(path).map_err(error)? {
                    let entry = entry.map_err(error)?;
                    reject_link(&entry.path())?;
                    names.push(entry.file_name().into_string().map_err(|_| "Non-Unicode filename")?);
                }
                names.sort();
                Ok(json!(names))
            }
            "storage.makeFolder" => {
                fs::create_dir_all(&path).map_err(error)?;
                Ok(json!(true))
            }
            "storage.remove" => {
                self.require_child(&path)?;
                if path.is_dir() {
                    inventory(&path)?; // Reject any reparse point before deleting anything.
                    fs::remove_dir_all(path).map_err(error)?;
                } else if path.exists() {
                    fs::remove_file(path).map_err(error)?;
                }
                Ok(json!(true))
            }
            "storage.copy" | "storage.move" => {
                self.require_child(&path)?;
                let destination = self.resolve(argument(args, 1)?)?;
                self.require_child(&destination)?;
                if destination == path || destination.starts_with(&path) || path.starts_with(&destination) {
                    return Err("Source and destination must be separate paths".into());
                }
                if destination.exists() { return Err("Destination already exists".into()); }
                if path.is_dir() {
                    inventory(&path)?;
                    if name == "storage.copy" { migrate_cache(&path, &destination)?; }
                    else { fs::rename(path, destination).map_err(error)?; }
                } else if name == "storage.copy" {
                    let bytes = fs::read(path).map_err(error)?;
                    self.write(&destination, &bytes)?;
                } else {
                    fs::rename(path, destination).map_err(error)?;
                }
                Ok(json!(true))
            }
            // The event loop executes this action; this return value does not claim
            // Explorer has opened. No shell command is constructed from a path.
            "storage.reveal" => Ok(json!({"action":"reveal", "path":display_path(&path)})),
            _ => Err(format!("Unknown storage method: {name}")),
        }
    }

    pub fn read_json(&self, path: &str) -> Result<Option<Value>, String> {
        match self.call("storage.readText", &[json!(path)])? {
            Value::Null => Ok(None),
            Value::String(text) => serde_json::from_str(&text).map(Some).map_err(error),
            _ => unreachable!(),
        }
    }

    pub fn write_json(&self, path: &str, value: &Value) -> Result<(), String> {
        let path = self.resolve(path)?;
        self.write(&path, &serde_json::to_vec_pretty(value).map_err(error)?)
    }

    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.require_child(path)?;
        if let Some(parent) = path.parent() { fs::create_dir_all(parent).map_err(error)?; }
        self.resolve(&display_path(path))?;
        atomic_write(path, bytes)
    }

    fn require_child(&self, path: &Path) -> Result<(), String> {
        if path == self.root || !path.starts_with(&self.root) {
            Err("The application storage root itself cannot be replaced or removed".into())
        } else { Ok(()) }
    }
}

fn argument(args: &[Value], index: usize) -> Result<&str, String> {
    args.get(index).and_then(Value::as_str).ok_or_else(|| format!("Argument {index} must be a string"))
}

fn error(error: impl std::fmt::Display) -> String { error.to_string() }

pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") { format!(r"\\{unc}") }
    else { text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned() }
}

fn validate_path(path: &Path) -> Result<(), String> {
    for component in path.components() {
        match component {
            Component::ParentDir => return Err("Parent traversal is not allowed".into()),
            Component::Prefix(_) if !path.is_absolute() => return Err("Drive-relative paths are not allowed".into()),
            Component::Normal(name) => {
                let name = name.to_str().ok_or("Non-Unicode path")?;
                if name.contains('\0') || name.contains(':') || name.contains('\\') || name.ends_with(['.', ' ']) {
                    return Err("Invalid storage path component".into());
                }
                let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
                if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
                    || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9')) {
                    return Err("Windows device names are not storage paths".into());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn resolve_ancestor(path: &Path) -> Result<PathBuf, String> {
    let mut ancestor = path.to_owned();
    let mut missing = Vec::new();
    while !ancestor.try_exists().map_err(error)? {
        missing.push(ancestor.file_name().ok_or("Path has no existing ancestor")?.to_owned());
        if !ancestor.pop() { return Err("Path has no existing ancestor".into()); }
    }
    let mut resolved = fs::canonicalize(ancestor).map_err(error)?;
    for component in missing.into_iter().rev() { resolved.push(component); }
    Ok(resolved)
}

fn reject_link(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if metadata.file_type().is_symlink() || reparse {
        Err(format!("Links and reparse points are not allowed: {}", display_path(path)))
    } else { Ok(()) }
}

fn unique_sibling(path: &Path, label: &str) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("Path has no parent")?;
    let name = path.file_name().ok_or("Path has no filename")?.to_string_lossy();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_err(error)?.as_nanos();
    Ok(parent.join(format!(".{name}.{label}-{}-{nonce}-{}", std::process::id(), NEXT_FILE.fetch_add(1, Ordering::Relaxed))))
}

/// Same-volume write/flush/replace, matching the recovered Windows flags (1 | 8).
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = unique_sibling(path, "write")?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(error)?;
    let result = (|| {
        file.write_all(bytes).map_err(error)?;
        file.sync_all().map_err(error)?;
        drop(file);
        replace_file(&temporary, path)
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().chain(Some(0)).collect();
    // Both paths are owned, NUL-terminated buffers alive throughout the call.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
        Err(error(std::io::Error::last_os_error()))
    } else { Ok(()) }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(error)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    relative: PathBuf,
    directory: bool,
    size: u64,
    sha256: String,
}

fn digest(path: &Path) -> Result<(u64, String), String> {
    let mut file = File::open(path).map_err(error)?;
    let mut hash = Sha256::new();
    let mut total = 0;
    let mut bytes = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes).map_err(error)?;
        if count == 0 { break; }
        total += count as u64;
        hash.update(&bytes[..count]);
    }
    Ok((total, format!("{:x}", hash.finalize())))
}

fn inventory(root: &Path) -> Result<Vec<Entry>, String> {
    reject_link(root)?;
    if !root.is_dir() { return Err("Cache source must be a directory".into()); }
    let mut result = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(error)? {
            let path = entry.map_err(error)?.path();
            reject_link(&path)?;
            let relative = path.strip_prefix(root).map_err(error)?.to_owned();
            validate_path(&relative)?;
            let metadata = fs::metadata(&path).map_err(error)?;
            if metadata.is_dir() {
                pending.push(path);
                result.push(Entry { relative, directory: true, size: 0, sha256: String::new() });
            } else if metadata.is_file() {
                let (size, sha256) = digest(&path)?;
                result.push(Entry { relative, directory: false, size, sha256 });
            } else { return Err("Unsupported cache entry type".into()); }
        }
    }
    result.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(result)
}

/// Copy a closed cache into a new, user-selected destination. Never deletes source.
/// The caller switches its persisted cache location only after this returns Ok.
/// A failed copy leaves the owned staging directory for diagnostics/retry, with its
/// path in the error; it never exposes a partial tree at the destination.
pub fn migrate_cache(source: &Path, destination: &Path) -> Result<Value, String> {
    validate_path(source)?;
    validate_path(destination)?;
    if !source.is_absolute() || !destination.is_absolute() {
        return Err("Cache migration requires absolute paths".into());
    }
    reject_link(source)?;
    let source = fs::canonicalize(source).map_err(error)?;
    let destination = resolve_ancestor(destination)?;
    if destination.starts_with(&source) || source.starts_with(&destination) {
        return Err("Cache locations must not contain each other".into());
    }
    if destination.exists() { return Err("The selected cache destination must be a new directory".into()); }
    let parent = destination.parent().ok_or("Cache destination has no parent")?;
    if !parent.is_dir() { return Err("Select a destination inside an existing directory".into()); }
    reject_link(parent)?;
    let entries = inventory(&source)?;
    let staging = unique_sibling(&destination, "incoming")?;
    fs::create_dir(&staging).map_err(error)?;
    let result: Result<Value, String> = (|| {
        for entry in &entries {
            let target = staging.join(&entry.relative);
            if entry.directory {
                fs::create_dir_all(target).map_err(error)?;
            } else {
                fs::create_dir_all(target.parent().ok_or("Missing file parent")?).map_err(error)?;
                let mut input = File::open(source.join(&entry.relative)).map_err(error)?;
                let mut output = OpenOptions::new().write(true).create_new(true).open(&target).map_err(error)?;
                std::io::copy(&mut input, &mut output).map_err(error)?;
                output.sync_all().map_err(error)?;
                drop(output);
                if digest(&target)? != (entry.size, entry.sha256.clone()) {
                    return Err(format!("Cache copy hash mismatch: {}", display_path(&entry.relative)));
                }
            }
        }
        // Includes empty directories, unexpected files and source edits during copy.
        if inventory(&staging)? != entries || inventory(&source)? != entries {
            return Err("Cache changed during migration; close all cache writers and retry".into());
        }
        if destination.exists() { return Err("Cache destination appeared during migration".into()); }
        fs::rename(&staging, &destination).map_err(error)?;
        Ok(json!({
            "source":display_path(&source), "destination":display_path(&destination),
            "files":entries.iter().filter(|e| !e.directory).count(),
            "bytes":entries.iter().map(|e| e.size).sum::<u64>(),
            "sourceRetained":true, "verified":"sha256",
            "completedAt":SystemTime::now().duration_since(UNIX_EPOCH).map_err(error)?.as_secs(),
        }))
    })();
    result.map_err(|message| format!("{message}; source retained; staging: {}", display_path(&staging)))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("aoqi-storage-test-{}-{}", std::process::id(), NEXT_FILE.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }

    #[test]
    fn text_binary_json_append_and_missing_roundtrip() {
        let temp = Temp::new();
        let storage = Storage::new(temp.0.join("data")).unwrap();
        assert_eq!(storage.call("storage.readText", &[json!("missing")]).unwrap(), Value::Null);
        storage.call("storage.writeText", &[json!("nested/log.txt"), json!("奥奇"), json!(false)]).unwrap();
        storage.call("storage.writeText", &[json!("nested/log.txt"), json!("\n追加"), json!(true)]).unwrap();
        assert_eq!(storage.call("storage.readText", &[json!("nested/log.txt")]).unwrap(), json!("奥奇\n追加"));
        storage.call("storage.write", &[json!("bytes"), json!(STANDARD.encode([0, 255, 128]))]).unwrap();
        assert_eq!(storage.call("storage.read", &[json!("bytes")]).unwrap(), json!("AP+A"));
        assert!(storage.call("storage.write", &[json!("bytes"), json!("invalid!")]).is_err());
        assert_eq!(fs::read(storage.root.join("bytes")).unwrap(), [0, 255, 128]);
        storage.write_json("settings.json", &json!({"enabled":true})).unwrap();
        assert_eq!(storage.read_json("settings.json").unwrap(), Some(json!({"enabled":true})));
    }

    #[test]
    fn rejects_escape_devices_and_root_mutation() {
        let temp = Temp::new();
        let storage = Storage::new(temp.0.join("data")).unwrap();
        for path in ["../outside", "sub/../../outside", "file:secret", "NUL", "a/COM1.txt", "trailing."] {
            assert!(storage.resolve(path).is_err(), "{path}");
        }
        assert!(storage.resolve(&display_path(&temp.0.join("outside"))).is_err());
        assert!(storage.call("storage.remove", &[json!(display_path(storage.root()))]).is_err());
        assert!(storage.resolve(&format!("{}/new/file", display_path(storage.root()))).is_ok());
    }

    #[test]
    fn copy_move_list_info_remove_and_resources() {
        let temp = Temp::new();
        let storage = Storage::new(temp.0.clone()).unwrap();
        storage.call("storage.writeText", &[json!("a"), json!("hello")]).unwrap();
        storage.call("storage.copy", &[json!("a"), json!("b")]).unwrap();
        storage.call("storage.move", &[json!("b"), json!("c")]).unwrap();
        assert_eq!(storage.call("storage.list", &[json!("")]).unwrap(), json!(["a", "c"]));
        assert_eq!(storage.call("storage.info", &[json!("c")]).unwrap()["size"], json!(5));
        storage.call("storage.remove", &[json!("a")]).unwrap();
        assert_eq!(storage.call("storage.info", &[json!("a")]).unwrap(), json!({"exists":false}));
        let binary = storage.call("storage.resource", &[json!("host.swf")]).unwrap();
        assert_eq!(STANDARD.decode(binary.as_str().unwrap()).unwrap(), aoqi_assets::get("host.swf").unwrap().bytes);
        assert_eq!(storage.call("storage.resource", &[json!("../host.swf")]).unwrap(), Value::Null);
        assert_eq!(storage.call("storage.reveal", &[json!("c")]).unwrap()["action"], json!("reveal"));
    }

    #[test]
    fn failed_replace_preserves_target_and_cleans_owned_temporary() {
        let temp = Temp::new();
        fs::create_dir(temp.0.join("existing")).unwrap();
        fs::write(temp.0.join("existing/keep"), "retained").unwrap();
        assert!(atomic_write(&temp.0.join("existing"), b"replacement").is_err());
        assert_eq!(fs::read(temp.0.join("existing/keep")).unwrap(), b"retained");
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }

    #[test]
    fn migration_verifies_every_file_retains_source_and_empty_folders() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        let destination = temp.0.join("destination");
        fs::create_dir_all(source.join("empty")).unwrap();
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/cache"), [0, 1, 255]).unwrap();
        let report = migrate_cache(&source, &destination).unwrap();
        assert_eq!(report["files"], 1);
        assert_eq!(report["sourceRetained"], true);
        assert_eq!(inventory(&source).unwrap(), inventory(&destination).unwrap());
        assert!(migrate_cache(&source, &destination).is_err());
        assert!(migrate_cache(&source, &source.join("inside")).is_err());
        assert_eq!(fs::read(source.join("nested/cache")).unwrap(), [0, 1, 255]);
    }

    #[test]
    fn migration_detects_modified_manifest() {
        let temp = Temp::new();
        fs::write(temp.0.join("file"), "first").unwrap();
        let expected = inventory(&temp.0).unwrap();
        fs::write(temp.0.join("file"), "other").unwrap();
        assert_ne!(inventory(&temp.0).unwrap(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_links_inside_root() {
        let temp = Temp::new();
        let storage = Storage::new(temp.0.join("data")).unwrap();
        fs::create_dir(temp.0.join("outside")).unwrap();
        std::os::unix::fs::symlink(temp.0.join("outside"), storage.root.join("link")).unwrap();
        assert!(storage.resolve("link/file").is_err());
        assert!(migrate_cache(storage.root(), &temp.0.join("copy")).is_err());
    }
}
