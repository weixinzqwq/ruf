//! Versioned disk cache for public Aoqi binary resources. Legacy files are left
//! untouched: their key/index format has not been established by available evidence.

use reqwest::header::HeaderMap;
use ruffle_core::backend::navigator::{NavigationMethod, OwnedFuture, Request, SuccessResponse};
use ruffle_core::loader::Error;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

mod budget;

const MAGIC: &[u8; 8] = b"AOQIC001";
const MAX_RESOURCE: usize = 128 * 1024 * 1024;
const MAX_METADATA: usize = 16 * 1024;

#[derive(Clone)]
pub struct AoqiCache {
    root: PathBuf,
    entries: PathBuf,
    adapted: PathBuf,
    budget: Option<Arc<budget::DiskBudget>>,
}

pub struct CachedResource {
    pub bytes: Vec<u8>,
    pub response_url: String,
    pub content_type: Option<String>,
}

pub(super) struct AdaptedKey(String);

impl AdaptedKey {
    pub(super) fn new(source: u32, target: u32, part: &str, version: &str) -> Option<Self> {
        let name = format!("v1-{source}-{target}-{part}-{version}.swf");
        let path = Path::new(&name);
        (path.file_name() == Some(path.as_os_str()) && adapted_entry_name(path)).then_some(Self(name))
    }
}

impl AoqiCache {
    /// root is the same directory used by migration (normally storage_root/cache).
    pub fn new(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&root)?;
        no_link(&root)?;
        let root = fs::canonicalize(root)?;
        let entries = root.join("v1");
        fs::create_dir_all(&entries)?;
        no_link(&entries)?;
        let adapted = root.join("morph-adapted");
        fs::create_dir_all(&adapted)?;
        no_link(&adapted)?;
        Ok(Self { root, entries, adapted, budget: None })
    }

    pub(super) fn for_aola(root: PathBuf) -> io::Result<Self> {
        let mut cache = Self::new(root)?;
        if let Some(limit) = budget::configured_limit(std::env::var("AOLA_DISK_CACHE_BUDGET_MB").ok().as_deref()) {
            match budget::DiskBudget::start(&cache.entries, &cache.adapted, limit) {
                Ok(budget) => cache.budget = Some(budget),
                Err(error) => tracing::warn!("Static cache budget unavailable: {error}"),
            }
        }
        Ok(cache)
    }

    pub fn root(&self) -> &Path { &self.root }

    /// No credentials, arbitrary query strings, account endpoints or POST bodies
    /// can enter the cache key or cache metadata.
    pub fn accepts_request(request: &Request, url: &Url) -> bool {
        if request.method() != NavigationMethod::Get
            || request.body().as_ref().is_some_and(|(bytes, _)| !bytes.is_empty())
            || !public_resource_url(url) { return false; }
        request.headers().iter().all(|(name, value)| {
            let name = name.to_ascii_lowercase();
            matches!(name.as_str(), "accept" | "accept-language" | "user-agent" | "referer" | "origin" | "cache-control" | "pragma" | "content-type")
                && !(matches!(name.as_str(), "cache-control" | "pragma") && value.to_ascii_lowercase().contains("no-cache"))
        })
    }

    pub fn lookup(&self, url: &Url) -> io::Result<Option<CachedResource>> {
        if !public_resource_url(url) { return Ok(None); }
        no_link(&self.root)?;
        no_link(&self.entries)?;
        let path = self.entry_path(url);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            Ok(_) => no_link(&path)?,
        }
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let length = file.metadata()?.len();
        if length > (MAX_RESOURCE + MAX_METADATA + 12) as u64 { return Ok(None); }
        let mut header = [0_u8; 12];
        if file.read_exact(&mut header).is_err() || &header[..8] != MAGIC { return Ok(None); }
        let metadata_length = u32::from_le_bytes(header[8..].try_into().unwrap()) as usize;
        if metadata_length > MAX_METADATA { return Ok(None); }
        let mut metadata = vec![0; metadata_length];
        if file.read_exact(&mut metadata).is_err() { return Ok(None); }
        let Ok(metadata) = serde_json::from_slice::<Value>(&metadata) else { return Ok(None); };
        if metadata["url"].as_str() != Some(url.as_str())
            || metadata["expiresAt"].as_u64().unwrap_or(0) <= now()
            || metadata["status"].as_u64() != Some(200) { return Ok(None); }
        let Some(response_url) = metadata["responseUrl"].as_str() else { return Ok(None); };
        if !Url::parse(response_url).ok().as_ref().is_some_and(public_resource_url) { return Ok(None); }
        let mut bytes = Vec::new();
        file.take(MAX_RESOURCE as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > MAX_RESOURCE || !valid_payload(url, &bytes)
            || metadata["size"].as_u64() != Some(bytes.len() as u64)
            || metadata["sha256"].as_str() != Some(hash(&bytes).as_str()) { return Ok(None); }
        touch(&path);
        Ok(Some(CachedResource {
            bytes, response_url: response_url.to_owned(),
            content_type: metadata["contentType"].as_str().map(str::to_owned),
        }))
    }

    pub fn response_policy(url: &Url, response_url: &Url, status: u16, headers: &HeaderMap) -> Option<CachePolicy> {
        if status != 200 || !public_resource_url(url) || !public_resource_url(response_url)
            || headers.contains_key("set-cookie") || headers.contains_key("content-range") { return None; }
        let control = headers.get_all("cache-control").iter().filter_map(|v| v.to_str().ok()).collect::<Vec<_>>().join(",").to_ascii_lowercase();
        let immutable_morph = versioned_morph_path(url.path()) && response_url.path() == url.path();
        let allows_storage = control.split(',').all(|value| {
            let directive = value.split('=').next().unwrap_or("").trim();
            directive != "no-store" && (immutable_morph || !matches!(directive, "no-cache" | "private"))
        });
        if !allows_storage { return None; }
        let vary = headers.get_all("vary").iter().filter_map(|v| v.to_str().ok()).collect::<Vec<_>>().join(",").to_ascii_lowercase();
        if vary.split(',').any(|v| matches!(v.trim(), "*" | "cookie" | "authorization")) { return None; }
        let content_type = headers.get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
        if content_type.as_deref().is_some_and(|v| {
            let v = v.to_ascii_lowercase();
            v.starts_with("text/") || v.starts_with("application/json") || v.starts_with("application/xhtml")
        }) { return None; }
        let versioned = url.query().is_some() || url.path().rsplit('/').next().is_some_and(|name| {
            name.rsplit_once('~').is_some_and(|(_, suffix)| suffix.split('.').next().is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())))
        });
        let max_age = control.split(',').find_map(|part| part.trim().strip_prefix("max-age=").and_then(|s| s.trim_matches('"').parse::<u64>().ok()))
            .filter(|&age| age > 0 || !immutable_morph)
            .unwrap_or(if versioned { 7 * 86400 } else { 300 }).min(365 * 86400);
        if max_age == 0 { return None; }
        Some(CachePolicy { response_url: response_url.to_string(), content_type, expires_at: now().saturating_add(max_age) })
    }

    pub fn store(&self, url: &Url, policy: &CachePolicy, bytes: &[u8]) -> io::Result<bool> {
        if bytes.len() > MAX_RESOURCE || !public_resource_url(url) || !valid_payload(url, bytes)
            || !Url::parse(&policy.response_url).ok().as_ref().is_some_and(public_resource_url) { return Ok(false); }
        no_link(&self.root)?;
        no_link(&self.entries)?;
        let path = self.entry_path(url);
        if path.exists() { no_link(&path)?; }
        let metadata = serde_json::to_vec(&json!({
            "url":url.as_str(),"responseUrl":policy.response_url,"contentType":policy.content_type,
            "status":200,"size":bytes.len(),"sha256":hash(bytes),"expiresAt":policy.expires_at,
        }))?;
        if metadata.len() > MAX_METADATA { return Ok(false); }
        self.write_entry(&path, |file| {
            file.write_all(MAGIC)?;
            file.write_all(&(metadata.len() as u32).to_le_bytes())?;
            file.write_all(&metadata)?;
            file.write_all(bytes)
        })
    }

    pub(super) fn open_adapted(&self, key: &AdaptedKey) -> io::Result<Option<(File, u64)>> {
        no_link(&self.root)?;
        no_link(&self.adapted)?;
        let path = self.adapted.join(&key.0);
        match no_link(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            Ok(()) => {}
        }
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let length = file.metadata()?.len();
        if !adapted_header(&mut file, length)? {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid adapted SWF"));
        }
        use std::io::Seek;
        file.rewind()?;
        touch(&path);
        Ok(Some((file, length)))
    }

    pub(super) fn store_adapted(&self, key: &AdaptedKey, bytes: &[u8]) -> io::Result<bool> {
        if !adapted_header(&mut &*bytes, bytes.len() as u64)? { return Ok(false); }
        no_link(&self.root)?;
        no_link(&self.adapted)?;
        self.write_entry(&self.adapted.join(&key.0), |file| file.write_all(bytes))
    }

    fn write_entry(&self, path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<bool> {
        if path.exists() { no_link(path)?; }
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let temporary = path.with_extension(format!("partial-{}-{nonce}", std::process::id()));
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        let result = (|| {
            write(&mut file)?;
            file.sync_all()?;
            drop(file);
            if let Some(budget) = &self.budget {
                budget.publish(&temporary, path)
            } else {
                fs::rename(&temporary, path).map(|_| true)
            }
        })();
        if !matches!(result, Ok(true)) { let _ = fs::remove_file(temporary); }
        result
    }

    /// Read actual disk contents, including files copied by cache migration. Never
    /// claims unknown legacy files can be used as cache hits.
    pub fn stats(&self) -> io::Result<Value> {
        no_link(&self.root)?;
        let mut files = 0_u64;
        let mut bytes = 0_u64;
        let mut adapted_files = 0_u64;
        let mut adapted_bytes = 0_u64;
        let mut legacy_files = 0_u64;
        let mut legacy_bytes = 0_u64;
        let mut pending = vec![self.root.clone()];
        while let Some(directory) = pending.pop() {
            no_link(&directory)?;
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                let path = entry.path();
                if directory == self.entries && entry.file_name() == budget::LOCK_NAME { continue; }
                no_link(&path)?;
                let metadata = entry.metadata()?;
                if metadata.is_dir() { pending.push(path); }
                else if metadata.is_file() {
                    if path.parent() == Some(self.entries.as_path()) && path.extension().is_some_and(|ext| ext == "cache") {
                        files += 1; bytes += metadata.len();
                    } else if directory == self.adapted && adapted_entry_name(&path) {
                        adapted_files += 1; adapted_bytes += metadata.len();
                    } else { legacy_files += 1; legacy_bytes += metadata.len(); }
                }
            }
        }
        Ok(json!({"root":self.root.to_string_lossy(),"format":"aoqi-static-cache-v1","entries":files,"diskBytes":bytes,
            "morphAdaptedEntries":adapted_files,"morphAdaptedDiskBytes":adapted_bytes,
            "legacyFiles":legacy_files,"legacyBytes":legacy_bytes,"legacyReadable":false}))
    }

    fn entry_path(&self, url: &Url) -> PathBuf { self.entries.join(format!("{}.cache", hash(url.as_str().as_bytes()))) }
}

#[derive(Clone)]
pub struct CachePolicy {
    response_url: String,
    content_type: Option<String>,
    expires_at: u64,
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }
fn touch(path: &Path) {
    if let Ok(file) = OpenOptions::new().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

fn versioned_morph_path(path: &str) -> bool {
    let Some((name, version)) = path.strip_suffix(".swf").and_then(|path| path.rsplit_once('~')) else { return false; };
    numeric_version(version) && (
        ["/play/fightassets/bitmap/", "/play/peticon/movie/", "/play/pet/petskin/background/"]
            .iter().any(|prefix| name.starts_with(prefix))
        || ["/play/peticon/petfighticon/peticon", "/play/peticon/peticon"]
            .into_iter().filter_map(|prefix| name.strip_prefix(prefix))
            .any(|id| !id.is_empty() && !id.starts_with('0') && id.bytes().all(|byte| byte.is_ascii_digit()))
    )
}

pub(super) fn numeric_version(version: &str) -> bool {
    !version.is_empty() && version.len() <= 24 && version.bytes().all(|byte| byte.is_ascii_digit())
}

fn adapted_entry_name(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false; };
    let Some(name) = name.strip_prefix("v1-").and_then(|name| name.strip_suffix(".swf")) else { return false; };
    let mut fields = name.splitn(3, '-');
    let source = fields.next().and_then(|id| id.parse::<u32>().ok());
    let target = fields.next().and_then(|id| id.parse::<u32>().ok());
    let Some((part, version)) = fields.next().and_then(|rest| rest.rsplit_once('-')) else { return false; };
    source.is_some_and(|id| id > 0) && target.is_some_and(|id| id > 0)
        && source != target && numeric_version(version)
        && matches!(part, "poster" | "head-icon" | "head-icon-legacy" | "backpack-0" | "backpack-1" | "backpack-2" | "backpack-3"
            | "battle-1-0" | "battle-1-1" | "battle-1-2" | "battle-2-0" | "battle-2-1" | "battle-2-2")
}

fn adapted_header(reader: &mut impl Read, length: u64) -> io::Result<bool> {
    if !(8..=MAX_RESOURCE as u64).contains(&length) { return Ok(false); }
    let mut prefix = [0; 8];
    reader.read_exact(&mut prefix)?;
    let declared = u64::from(u32::from_le_bytes(prefix[4..].try_into().unwrap()));
    Ok(&prefix[..3] == b"FWS" && prefix[3] != 0 && declared == length)
}

fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn no_link(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if is_link(&metadata) { Err(io::Error::other("Cache directory contains a link or reparse point")) }
    else { Ok(()) }
}
fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    let reparse = { use std::os::windows::fs::MetadataExt; metadata.file_attributes() & 0x400 != 0 };
    #[cfg(not(windows))]
    let reparse = false;
    metadata.file_type().is_symlink() || reparse
}

fn public_resource_url(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some()
        || url.fragment().is_some() || url.path().contains('%') || url.as_str().len() > 8192 { return false; }
    let Some(host) = url.host_str() else { return false; };
    if !(host == "100bt.com" || host.ends_with(".100bt.com")) { return false; }
    let lower = format!("{host}{}", url.path()).to_ascii_lowercase();
    if ["account", "login", "auth", "passport", "token", "session", "password", "credential", "payment", "oauth", "/api/"].iter().any(|word| lower.contains(word)) { return false; }
    if !matches!(extension(url), "swf" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "mp3" | "ogg" | "wav" | "flv") { return false; }
    url.query_pairs().all(|(key, value)| {
        matches!(key.as_ref(), "v" | "ver" | "version" | "hash" | "rev") && !value.is_empty() && value.len() <= 128
            && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    })
}

fn extension(url: &Url) -> &str { url.path().rsplit('.').next().unwrap_or("") }
fn valid_payload(url: &Url, bytes: &[u8]) -> bool {
    match extension(url) {
        "swf" => bytes.len() >= 8 && matches!(&bytes[..3], b"FWS" | b"CWS" | b"ZWS"),
        "png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => bytes.starts_with(b"\xff\xd8\xff"),
        "gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        "bmp" => bytes.starts_with(b"BM"),
        "mp3" => bytes.starts_with(b"ID3") || (bytes.len() >= 2 && bytes[0] == 255 && bytes[1] & 0xe0 == 0xe0),
        "ogg" => bytes.starts_with(b"OggS"),
        "wav" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE",
        "flv" => bytes.starts_with(b"FLV"),
        _ => false,
    }
}

/// Preserve streaming semantics: publish a cache entry only after body()/the last
/// next_chunk() succeeds. Abandoned and failed downloads never become cache hits.
pub struct CachingResponse {
    inner: Box<dyn SuccessResponse>,
    capture: Arc<Mutex<Capture>>,
}

struct Capture {
    cache: AoqiCache,
    url: Url,
    policy: CachePolicy,
    bytes: Vec<u8>,
    disabled: bool,
}

impl Capture {
    fn append(&mut self, bytes: &[u8]) {
        if self.disabled { return; }
        if self.bytes.len().saturating_add(bytes.len()) > MAX_RESOURCE {
            self.disabled = true; self.bytes.clear();
        } else { self.bytes.extend_from_slice(bytes); }
    }
    fn finish(&mut self) -> Option<(AoqiCache, Url, CachePolicy, Vec<u8>)> {
        if self.disabled { return None; }
        self.disabled = true;
        Some((self.cache.clone(), self.url.clone(), self.policy.clone(), std::mem::take(&mut self.bytes)))
    }

}

async fn persist_capture(job: Option<(AoqiCache, Url, CachePolicy, Vec<u8>)>) {
    if let Some((cache, url, policy, bytes)) = job {
        match ruffle_core::aoqi_work::blocking(move || cache.store(&url, &policy, &bytes)).await {
            Ok(Ok(_)) => {},
            result => tracing::warn!("Static resource cache write failed: {result:?}"),
        }
    }
}

impl CachingResponse {
    pub fn new(inner: Box<dyn SuccessResponse>, cache: AoqiCache, url: Url, policy: CachePolicy) -> Self {
        Self { inner, capture: Arc::new(Mutex::new(Capture { cache, url, policy, bytes: Vec::new(), disabled: false })) }
    }
}

impl SuccessResponse for CachingResponse {
    fn url(&self) -> std::borrow::Cow<'_, str> { self.inner.url() }
    fn set_url(&mut self, url: String) { self.inner.set_url(url); }
    fn body(self: Box<Self>) -> OwnedFuture<Vec<u8>, Error> {
        let Self { inner, capture } = *self;
        Box::pin(async move {
            let result = inner.body().await;
            let job = {
            let mut capture = capture.lock().map_err(|_| Error::FetchError("Cache capture lock poisoned".into()))?;
            match result {
                Ok(ref bytes) => capture.append(bytes),
                Err(_) => { capture.disabled = true; capture.bytes.clear(); }
            }
            capture.finish()
            };
            persist_capture(job).await;
            result
        })
    }
    fn text_encoding(&self) -> Option<&'static ruffle_core::swf::Encoding> { self.inner.text_encoding() }
    fn status(&self) -> u16 { self.inner.status() }
    fn redirected(&self) -> bool { self.inner.redirected() }
    fn expected_length(&self) -> Result<Option<u64>, Error> { self.inner.expected_length() }
    fn next_chunk(&mut self) -> OwnedFuture<Option<Vec<u8>>, Error> {
        let future = self.inner.next_chunk();
        let capture = self.capture.clone();
        Box::pin(async move {
            let result = future.await;
            let job = {
            let mut capture = capture.lock().map_err(|_| Error::FetchError("Cache capture lock poisoned".into()))?;
            match &result {
                Ok(Some(bytes)) => { capture.append(bytes); None },
                Ok(None) => capture.finish(),
                Err(_) => { capture.disabled = true; capture.bytes.clear(); None }
            }
            };
            persist_capture(job).await;
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SWF: &[u8] = b"FWS\x09\x08\0\0\0";
    fn url() -> Url { Url::parse("https://aoqi.100bt.com/play/library/common~123.swf").unwrap() }
    fn policy(url: &Url) -> CachePolicy { AoqiCache::response_policy(url, url, 200, &HeaderMap::new()).unwrap() }

    #[test]
    fn public_binary_roundtrip_and_real_statistics() {
        let temp = tempfile::tempdir().unwrap(); let cache = AoqiCache::new(temp.path().join("cache")).unwrap();
        assert!(cache.store(&url(), &policy(&url()), SWF).unwrap());
        let reopened = AoqiCache::new(cache.root.clone()).unwrap();
        assert_eq!(reopened.lookup(&url()).unwrap().unwrap().bytes, SWF);
        assert_eq!(reopened.stats().unwrap()["entries"], 1);
        assert!(reopened.stats().unwrap()["diskBytes"].as_u64().unwrap() > SWF.len() as u64);
    }

    #[test]
    fn excludes_credentials_login_api_post_and_unknown_queries() {
        for text in ["https://account.100bt.com/avatar.png", "https://aoqi.100bt.com/api/item.png", "https://aoqi.100bt.com/play/a.swf?token=secret", "https://user:pass@aoqi.100bt.com/a.swf", "https://other.example/a.swf", "https://aoqi.100bt.com/login/a.swf"] {
            let url = Url::parse(text).unwrap(); assert!(!AoqiCache::accepts_request(&Request::get(text.into()), &url));
        }
        assert!(!AoqiCache::accepts_request(&Request::post(url().to_string(), None), &url()));
        let mut request = Request::get(url().to_string());
        let mut headers = ruffle_core::indexmap::IndexMap::new(); headers.insert("Authorization".into(),"secret".into()); request.set_headers(headers);
        assert!(!AoqiCache::accepts_request(&request, &url()));
    }

    #[test]
    fn private_partial_cookie_and_failed_responses_are_not_cached() {
        for (key, value) in [("set-cookie","session=secret"),("cache-control","private, max-age=300"),("cache-control","no-store"),("vary","Cookie"),("content-type","text/html")] {
            let mut headers=HeaderMap::new(); headers.insert(reqwest::header::HeaderName::from_bytes(key.as_bytes()).unwrap(), value.parse().unwrap());
            assert!(AoqiCache::response_policy(&url(), &url(),200,&headers).is_none());
        }
        assert!(AoqiCache::response_policy(&url(), &url(),206,&HeaderMap::new()).is_none());
        assert!(AoqiCache::response_policy(&url(), &url(),404,&HeaderMap::new()).is_none());
    }

    #[test]
    fn versioned_morph_leaves_ignore_conservative_cdn_revalidation_headers() {
        let url = Url::parse("https://aola.100bt.com/play/peticon/movie/type0/petmovie6044/petmovie6044_0~1210.swf").unwrap();
        let unversioned = Url::parse(url.as_str().replace("~1210", "").as_str()).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("cache-control", "private, no-cache, max-age=0".parse().unwrap());
        assert!(AoqiCache::response_policy(&url, &url, 200, &headers).is_some());
        assert!(AoqiCache::response_policy(&unversioned, &unversioned, 200, &headers).is_none());
        for name in ["peticon/petfighticon/peticon5336", "peticon/peticon4167"] {
            let url = Url::parse(&format!("https://aola.100bt.com/play/{name}~1210.swf")).unwrap();
            assert!(AoqiCache::response_policy(&url, &url, 200, &headers).is_some());
        }
        let unrelated = Url::parse("https://aola.100bt.com/play/peticon/peticon_skin~1210.swf").unwrap();
        assert!(AoqiCache::response_policy(&unrelated, &unrelated, 200, &headers).is_none());
        headers.append("cache-control", "no-store".parse().unwrap());
        assert!(AoqiCache::response_policy(&url, &url, 200, &headers).is_none());
    }

    #[test]
    fn rejects_wrong_payload_expiry_and_corruption() {
        let temp=tempfile::tempdir().unwrap(); let cache=AoqiCache::new(temp.path().join("cache")).unwrap();
        assert!(!cache.store(&url(),&policy(&url()),b"<html>login</html>").unwrap());
        let mut expired=policy(&url()); expired.expires_at=0; cache.store(&url(),&expired,SWF).unwrap();
        assert!(cache.lookup(&url()).unwrap().is_none());
        cache.store(&url(),&policy(&url()),SWF).unwrap();
        let path=cache.entry_path(&url()); let mut bytes=fs::read(&path).unwrap(); *bytes.last_mut().unwrap()=1; fs::write(path,bytes).unwrap();
        assert!(cache.lookup(&url()).unwrap().is_none());
    }

    #[test]
    fn migrated_v1_is_readable_legacy_is_preserved_and_counted() {
        let temp=tempfile::tempdir().unwrap(); let source=AoqiCache::new(temp.path().join("source")).unwrap();
        source.store(&url(),&policy(&url()),SWF).unwrap();
        let target=AoqiCache::new(temp.path().join("target")).unwrap();
        fs::copy(source.entry_path(&url()),target.entry_path(&url())).unwrap();
        fs::write(target.root.join("legacy-resource.bin"),b"legacy-data").unwrap();
        assert_eq!(target.lookup(&url()).unwrap().unwrap().bytes,SWF);
        assert_eq!(target.stats().unwrap()["legacyFiles"],1);
        assert_eq!(fs::read(target.root.join("legacy-resource.bin")).unwrap(),b"legacy-data");
    }
    struct BytesResponse(std::collections::VecDeque<Vec<u8>>);
    impl SuccessResponse for BytesResponse {
        fn url(&self) -> std::borrow::Cow<'_,str> { std::borrow::Cow::Owned(url().to_string()) }
        fn set_url(&mut self, _: String) {}
        fn body(self: Box<Self>) -> OwnedFuture<Vec<u8>,Error> {
            Box::pin(async move { Ok(self.0.into_iter().flatten().collect()) })
        }
        fn text_encoding(&self) -> Option<&'static ruffle_core::swf::Encoding> { None }
        fn status(&self) -> u16 { 200 }
        fn redirected(&self) -> bool { false }
        fn expected_length(&self) -> Result<Option<u64>,Error> { Ok(None) }
        fn next_chunk(&mut self) -> OwnedFuture<Option<Vec<u8>>,Error> {
            let chunk=self.0.pop_front(); Box::pin(async move { Ok(chunk) })
        }
    }
    #[test]
    fn background_cache_body_and_chunk_completion_preserve_bytes_and_cancellation() {
        let temp=tempfile::tempdir().unwrap();
        for streaming in [false,true] {
            let cache=AoqiCache::new(temp.path().join(if streaming {"chunks"} else {"body"})).unwrap();
            let inner=Box::new(BytesResponse([SWF[..4].to_vec(),SWF[4..].to_vec()].into()));
            let mut response=Box::new(CachingResponse::new(inner,cache.clone(),url(),policy(&url())));
            if streaming {
                assert_eq!(futures_lite::future::block_on(response.next_chunk()).unwrap().unwrap(),SWF[..4]);
                assert!(cache.lookup(&url()).unwrap().is_none());
                assert_eq!(futures_lite::future::block_on(response.next_chunk()).unwrap().unwrap(),SWF[4..]);
                assert!(cache.lookup(&url()).unwrap().is_none());
                assert!(futures_lite::future::block_on(response.next_chunk()).unwrap().is_none());
            } else { assert_eq!(futures_lite::future::block_on(response.body()).unwrap(),SWF); }
            assert_eq!(cache.lookup(&url()).unwrap().unwrap().bytes,SWF);
        }
        let cache=AoqiCache::new(temp.path().join("cancelled")).unwrap();
        let mut response=CachingResponse::new(Box::new(BytesResponse([SWF[..4].to_vec()].into())),cache.clone(),url(),policy(&url()));
        futures_lite::future::block_on(response.next_chunk()).unwrap();
        drop(response);
        assert!(cache.lookup(&url()).unwrap().is_none());
    }

}
