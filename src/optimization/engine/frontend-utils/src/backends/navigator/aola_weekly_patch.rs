//! Verified external replacements for the five versioned Aola main movies.
use super::{aola_display_patch, aola_game_ui, fetch};
use ruffle_core::backend::navigator::{NavigationMethod, Request, SuccessResponse};
use ruffle_core::mypro_observer::{
    clear_patch_version_mismatch, compare_patch_versions, observe_patch_version_mismatch,
};
use ruffle_core::swf::{extensions::ReadSwfExt, read::Reader};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::Url;

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_MOVIE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Resource {
    Battle,
    Backpack,
    Notice,
    Control,
    Start,
}

impl Resource {
    const ALL: [Self; 5] = [Self::Battle, Self::Backpack, Self::Notice, Self::Control, Self::Start];

    fn name(self) -> &'static str {
        match self {
            Self::Battle => "battle",
            Self::Backpack => "backpack",
            Self::Notice => "notice",
            Self::Control => "control",
            Self::Start => "start",
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Battle => "/play/pet/petfightnew",
            Self::Backpack => "/play/pet/newpetpackage",
            Self::Notice => "/play/messagesend/messagesendservice",
            Self::Control => "/play/control/controlv2",
            Self::Start => "/play/start",
        }
    }

    fn requested<'a>(request: &Request, url: &'a Url) -> Option<(Self, &'a str)> {
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str() != Some("aola.100bt.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || request.method() != NavigationMethod::Get
            || request.body().is_some()
            || request.headers().keys().any(|key| key.eq_ignore_ascii_case("range"))
        {
            return None;
        }
        let (path, version) = url.path().strip_suffix(".swf")?.split_once('~')?;
        let resource = Self::ALL.into_iter().find(|resource| resource.path() == path)?;
        valid_version(version).then_some((resource, version))
    }
}

#[derive(Default)]
pub(super) struct Manifest {
    directory: PathBuf,
    entries: HashMap<Resource, HashMap<String, Entry>>,
}

struct Entry {
    official_hash: String,
    candidate_file: String,
    candidate_hash: String,
}

impl Manifest {
    pub(super) fn load(data_root: &Path) -> Self {
        let result = Self::read(data_root);
        match result {
            Ok(manifest) => manifest,
            Err(error) => {
                // A standalone installation without a weekly manifest still has
                // the embedded recipes and the same version drift observation.
                if error.kind() != io::ErrorKind::NotFound {
                    tracing::warn!(target: "aola_weekly_patch", "Couldn't load weekly patch manifest: {error}");
                }
                Self::default()
            }
        }
    }

    fn read(data_root: &Path) -> io::Result<Self> {
        let data_root = data_root.canonicalize()?;
        let directory = data_root.join("patches");
        let metadata = fs::symlink_metadata(&directory)?;
        if !metadata.is_dir() || is_link_or_reparse(&metadata) {
            return Err(invalid("patches must be a real directory"));
        }
        let directory = directory.canonicalize()?;
        if directory.parent() != Some(data_root.as_path()) {
            return Err(invalid("patches must be inside the data directory"));
        }
        let bytes = read_local(&directory, "manifest.json", MAX_MANIFEST_BYTES)?;
        let json: Value = serde_json::from_slice(&bytes)?;
        if json["schemaVersion"].as_u64() != Some(1)
            || !json["generatedAt"].as_str().is_some_and(|date| !date.trim().is_empty())
        {
            return Err(invalid("unsupported weekly patch manifest header"));
        }
        let entries = json["entries"].as_array().ok_or_else(|| invalid("missing entries"))?;
        let mut manifest = Self { directory, entries: HashMap::new() };
        for entry in entries {
            let resource = Resource::ALL.into_iter()
                .find(|resource| entry["resource"].as_str() == Some(resource.name())
                    && entry["path"].as_str() == Some(resource.path()))
                .ok_or_else(|| invalid("unknown resource or path in weekly patch manifest"))?;
            let version = entry["version"].as_str().filter(|version| valid_version(version))
                .ok_or_else(|| invalid("invalid official version in weekly patch manifest"))?;
            let candidate_file = entry["candidateFile"].as_str().filter(|name| valid_candidate_name(name))
                .ok_or_else(|| invalid("invalid candidate filename in weekly patch manifest"))?;
            let replace = Entry {
                official_hash: read_hash(&entry["officialSha256"])?,
                candidate_file: candidate_file.to_owned(),
                candidate_hash: read_hash(&entry["candidateSha256"])?,
            };
            if manifest.entries.entry(resource).or_default().insert(version.to_owned(), replace).is_some() {
                return Err(invalid("duplicate resource version in weekly patch manifest"));
            }
        }
        Ok(manifest)
    }

    pub(super) fn requested(
        self: &Arc<Self>, request: &Request, url: &Url,
        display: &[Arc<aola_display_patch::DisplayPatch>], game_ui_enabled: bool,
        backpack_trial: bool,
    ) -> Option<Selection> {
        let (resource, version) = Resource::requested(request, url)?;
        if resource == Resource::Backpack && backpack_trial {
            return None;
        }
        let path = resource.path();
        let versions = self.entries.get(&resource);
        let candidate_exists = versions.is_some_and(|versions| versions.contains_key(version));
        let game_ui = game_ui_enabled.then(|| aola_game_ui::Asset::version_for_path(path)).flatten();
        let local_version = display.iter().filter_map(|patch| patch.recipe.version_for_path(path))
            .chain(game_ui)
            // If the exact external entry fails validation, it does not cover this response.
            .chain(versions.into_iter().flat_map(|versions| versions.keys().map(String::as_str))
                .filter(|other| *other != version))
            .max_by(|left, right| compare_patch_versions(left, right));
        if !candidate_exists && !local_version.is_some_and(|local| compare_patch_versions(version, local).is_gt()) {
            return None;
        }
        Some(Selection {
            manifest: self.clone(), resource, version: version.to_owned(),
            local_version: local_version.map(str::to_owned), url: url.to_string(),
        })
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn valid_version(version: &str) -> bool {
    !version.is_empty() && version.len() <= 32 && version.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_candidate_name(name: &str) -> bool {
    name.ends_with(".swf") && name.len() <= 128
        && name.as_bytes().first().is_some_and(|byte| byte.is_ascii_alphanumeric())
        && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn read_hash(value: &Value) -> io::Result<String> {
    value.as_str()
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| invalid("invalid SHA256 in weekly patch manifest"))
}

fn matches_hash(bytes: &[u8], expected: &str) -> bool {
    format!("{:x}", Sha256::digest(bytes)) == expected
}

fn is_link_or_reparse(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn read_local(directory: &Path, name: &str, limit: usize) -> io::Result<Vec<u8>> {
    let path = directory.join(name);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || is_link_or_reparse(&metadata) || metadata.len() > limit as u64 {
        return Err(invalid("expected bounded regular weekly patch file"));
    }
    let path = path.canonicalize()?;
    if path.parent() != Some(directory) {
        return Err(invalid("weekly patch file resolves outside patches"));
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid("weekly patch file too large"));
    }
    Ok(bytes)
}

fn valid_fws(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"FWS") || bytes.get(3).is_none_or(|version| *version == 0)
        || bytes.get(4..8) != Some((bytes.len() as u32).to_le_bytes().as_slice())
    {
        return false;
    }
    let mut reader = Reader::new(&bytes[8..], bytes[3]);
    if reader.read_rectangle().is_err() || reader.read_slice(4).is_err() {
        return false;
    }
    while let Ok((tag, length)) = reader.read_tag_code_and_length() {
        if tag == 0 {
            return length == 0 && reader.as_slice().is_empty();
        }
        if reader.read_slice(length).is_err() {
            return false;
        }
    }
    false
}

pub(super) struct Selection {
    manifest: Arc<Manifest>,
    resource: Resource,
    version: String,
    local_version: Option<String>,
    url: String,
}

impl Selection {
    pub(super) fn wrap(
        self, response: Box<dyn SuccessResponse>, source: &'static str,
        display: Option<Arc<aola_display_patch::DisplayPatch>>, game_ui: Option<aola_game_ui::Selection>,
    ) -> Box<dyn SuccessResponse> {
        if response.status() == 200 && !response.redirected() && response.url() == self.url {
            fetch::filter_movie(response, WeeklyResponse { selection: self, source, display, game_ui })
        } else {
            response
        }
    }

    fn external(&self, official: &[u8]) -> Option<Vec<u8>> {
        let entry = self.manifest.entries.get(&self.resource)?.get(&self.version)?;
        if !matches_hash(official, &entry.official_hash) {
            self.audit("official-hash-mismatch");
            return None;
        }
        let candidate = match read_local(&self.manifest.directory, &entry.candidate_file, MAX_MOVIE_BYTES) {
            Ok(candidate) => candidate,
            Err(_) => {
                self.audit("candidate-unavailable");
                return None;
            }
        };
        if !valid_fws(&candidate) || !matches_hash(&candidate, &entry.candidate_hash) {
            self.audit("candidate-invalid");
            return None;
        }
        self.audit("external-hit");
        Some(candidate)
    }

    fn audit(&self, result: &'static str) {
        tracing::info!(target: "aola_weekly_patch", resource = self.resource.name(), version = self.version, result);
    }

    fn observe_result(&self, applied: bool) {
        if applied {
            clear_patch_version_mismatch(self.resource.name(), &self.version);
        } else if self.local_version.as_deref().is_none_or(|local| compare_patch_versions(&self.version, local).is_gt()) {
            observe_patch_version_mismatch(self.resource.name(), &self.version, self.local_version.as_deref());
        }
    }
}

struct WeeklyResponse {
    selection: Selection,
    source: &'static str,
    display: Option<Arc<aola_display_patch::DisplayPatch>>,
    game_ui: Option<aola_game_ui::Selection>,
}

impl fetch::MovieBodyFilter for WeeklyResponse {
    async fn complete(self, bytes: Vec<u8>) -> Vec<u8> {
        let official = Arc::new(bytes);
        let input = official.clone();
        let resolved = tokio::task::spawn_blocking(move || {
            let selection = self.selection.external(&input)
                .or_else(|| self.display.and_then(|patch| patch.select(self.source, &self.selection.version, &input)))
                .or_else(|| self.game_ui.and_then(|game_ui| game_ui.candidate(&input)));
            self.selection.observe_result(selection.is_some());
            selection
        }).await.unwrap_or_else(|error| {
            tracing::warn!(target: "aola_weekly_patch", "Weekly patch worker unavailable: {error}");
            None
        });
        resolved.unwrap_or_else(|| Arc::unwrap_or_clone(official))
    }

    fn fallback(&self, reason: &'static str) {
        self.selection.audit(reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::fetch::{Response, ResponseBody};
    use serde_json::json;

    fn movie(color: u8) -> Vec<u8> {
        // Tiny FWS with a one-bit RECT, one frame, SetBackgroundColor, ShowFrame and End.
        let mut bytes = b"FWS\x12\x17\0\0\0\x08\0\0\x18\x01\0\x43\x02\x01\0\0\x40\0\0\0".to_vec();
        bytes[16] = color;
        bytes
    }

    fn entry(resource: Resource, version: &str, file: &str, original: &[u8], candidate: &[u8]) -> Value {
        json!({
            "resource": resource.name(), "path": resource.path(), "version": version,
            "officialSha256": format!("{:x}", Sha256::digest(original)),
            "candidateFile": file,
            "candidateSha256": format!("{:x}", Sha256::digest(candidate)),
        })
    }

    fn write_manifest(root: &Path, entries: &[Value]) -> PathBuf {
        let patches = root.join("patches");
        fs::create_dir_all(&patches).unwrap();
        fs::write(patches.join("manifest.json"), json!({
            "schemaVersion": 1, "generatedAt": "2026-11-27T10:00:00Z", "entries": entries,
        }).to_string()).unwrap();
        patches
    }

    fn request(resource: Resource, version: &str) -> (Request, Url) {
        let address = format!("https://aola.100bt.com{}~{version}.swf", resource.path());
        (Request::get(address.clone()), Url::parse(&address).unwrap())
    }

    #[test]
    fn manifests_require_canonical_resources_versions_and_basenames() {
        let root = tempfile::tempdir().unwrap();
        let valid = entry(Resource::Battle, "2026112700000000", "battle.fws.swf", &movie(1), &movie(2));
        for invalid in ["../battle.swf", "dir/battle.swf", r"dir\battle.swf", "C:battle.swf", ".battle.swf", "battle.swf "] {
            let mut bad = valid.clone();
            bad["candidateFile"] = json!(invalid);
            write_manifest(root.path(), &[bad]);
            assert!(Manifest::read(root.path()).is_err(), "{invalid}");
        }
        let mut wrong_resource = valid.clone();
        wrong_resource["resource"] = json!("backpack");
        write_manifest(root.path(), &[wrong_resource]);
        assert!(Manifest::read(root.path()).is_err());
        write_manifest(root.path(), &[valid.clone(), valid]);
        assert!(Manifest::read(root.path()).is_err());
    }

    #[test]
    fn versions_are_exact_for_lookup_and_date_ordered_for_drift() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(root.path(), &[
            entry(Resource::Backpack, "2026111953323155", "prior.swf", &movie(1), &movie(2)),
            entry(Resource::Backpack, "261127000000000", "latest.swf", &movie(1), &movie(3)),
        ]);
        let manifest = Arc::new(Manifest::read(root.path()).unwrap());
        let (get, url) = request(Resource::Backpack, "261127000000000");
        let selection = manifest.requested(&get, &url, &[], false, false).unwrap();
        assert_eq!(selection.local_version.as_deref(), Some("2026111953323155"));
        assert!(manifest.requested(&get, &url, &[], false, true).is_none());
        let (get, url) = request(Resource::Backpack, "261128000000000");
        let selection = manifest.requested(&get, &url, &[], false, false).unwrap();
        assert_eq!(selection.local_version.as_deref(), Some("261127000000000"));
        let (get, url) = request(Resource::Backpack, "261110000000000");
        assert!(manifest.requested(&get, &url, &[], false, false).is_none());
    }

    #[test]
    fn only_five_exact_official_complete_gets_qualify() {
        for resource in Resource::ALL {
            let (get, url) = request(resource, "261127000000000");
            assert_eq!(Resource::requested(&get, &url), Some((resource, "261127000000000")));
            assert!(Resource::requested(&Request::post(url.to_string(), None), &url).is_none());
            let mut range = get;
            range.set_headers([("rAnGe".into(), "bytes=0-99".into())].into_iter().collect());
            assert!(Resource::requested(&range, &url).is_none());
            for bad in [
                url.as_str().replace("aola.100bt.com", "aola.100bt.com.example"),
                url.as_str().replace("aola.100bt.com", "user@aola.100bt.com"),
                url.as_str().replace("aola.100bt.com", "aola.100bt.com:8080"),
                url.as_str().replace("~", "/"),
                url.as_str().replace("261127000000000", "latest"),
                url.as_str().replace(".swf", ".swf/other"),
                format!("{url}#movie"),
            ] {
                assert!(Resource::requested(&Request::get(bad.clone()), &Url::parse(&bad).unwrap()).is_none());
            }
        }
    }

    #[tokio::test]
    async fn verified_bodies_are_replaced_once_for_network_and_cache() {
        let data = tempfile::tempdir().unwrap();
        let original = movie(1);
        let candidate = movie(2);
        let file = "battle.fws.swf";
        let patches = write_manifest(data.path(), &[
            entry(Resource::Battle, "261127000000000", file, &original, &candidate),
        ]);
        fs::write(patches.join(file), &candidate).unwrap();
        let manifest = Arc::new(Manifest::read(data.path()).unwrap());
        let (get, url) = request(Resource::Battle, "261127000000000");
        let response = |body: &[u8], source: &str, status, redirected| Box::new(Response {
            url: url.to_string(), text_encoding: None, status, redirected,
            response_body: if source == "network" {
                let network = http::Response::builder().status(status).body(body.to_vec()).unwrap();
                ResponseBody::Network(Arc::new(std::sync::Mutex::new(Some(reqwest::Response::from(network)))))
            } else {
                ResponseBody::File(Ok(body.to_vec()))
            },
        }) as Box<dyn SuccessResponse>;
        let wrap = |body: &[u8], source: &'static str, status, redirected| {
            manifest.requested(&get, &url, &[], false, false).unwrap()
                .wrap(response(body, source, status, redirected), source, None, None)
        };
        for source in ["cache", "network"] {
            let substituted = wrap(&original, source, 200, false);
            assert_eq!(substituted.url(), url.as_str());
            assert_eq!(substituted.body().await.unwrap(), candidate);
            let wrong_original = movie(3);
            assert_eq!(wrap(&wrong_original, source, 200, false).body().await.unwrap(), wrong_original);
            assert_eq!(wrap(&original, source, 206, false).body().await.unwrap(), original);
            assert_eq!(wrap(&original, source, 200, true).body().await.unwrap(), original);
            let mut streamed = wrap(&original, source, 200, false);
            assert_eq!(streamed.next_chunk().await.unwrap().unwrap(), original);
            assert!(streamed.next_chunk().await.unwrap().is_none());
        }
        fs::write(patches.join(file), movie(4)).unwrap();
        assert_eq!(wrap(&original, "cache", 200, false).body().await.unwrap(), original);
        fs::remove_file(patches.join(file)).unwrap();
        assert_eq!(wrap(&original, "cache", 200, false).body().await.unwrap(), original);
    }

    #[test]
    fn malformed_fws_never_qualifies_even_if_its_hash_matches() {
        let root = tempfile::tempdir().unwrap();
        let original = movie(1);
        for changed in [0, 3, 4, 14, 21] {
            let mut malformed = movie(2);
            malformed[changed] ^= if changed == 3 { 18 } else { 0x20 };
            assert!(!valid_fws(&malformed));
            let file = "control.fws.swf";
            let patches = write_manifest(root.path(), &[
                entry(Resource::Control, "261127000000000", file, &original, &malformed),
            ]);
            fs::write(patches.join(file), malformed).unwrap();
            let manifest = Arc::new(Manifest::read(root.path()).unwrap());
            let (get, url) = request(Resource::Control, "261127000000000");
            assert!(manifest.requested(&get, &url, &[], false, false).unwrap().external(&original).is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_candidate_is_rejected_even_with_the_expected_contents() {
        let root = tempfile::tempdir().unwrap();
        let original = movie(1);
        let candidate = movie(2);
        let file = "notice.fws.swf";
        let patches = write_manifest(root.path(), &[
            entry(Resource::Notice, "261127000000000", file, &original, &candidate),
        ]);
        let outside = root.path().join("outside.swf");
        fs::write(&outside, &candidate).unwrap();
        std::os::unix::fs::symlink(&outside, patches.join(file)).unwrap();
        let manifest = Arc::new(Manifest::read(root.path()).unwrap());
        let (get, url) = request(Resource::Notice, "261127000000000");
        assert!(manifest.requested(&get, &url, &[], false, false).unwrap().external(&original).is_none());
    }
}
