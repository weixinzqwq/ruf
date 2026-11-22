//! One opt-in SWF experiment. The roundtrip must decode to the complete original movie.
use super::{aola_skin_recipe, aola_swf_capture};
use lzma_rs::decompress::{Options, UnpackedSize};
use ruffle_core::backend::navigator::{NavigationMethod, Request, SuccessResponse};
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use url::Url;

const VERSION: &str = "2026111953323155";
const PREFIX: &str = "/play/pet/newpetpackage~";
const BACKPACK: &str = "pet/newpetpackage";
const UTIL: &str = "library/util";
const UTIL_VERSION: &str = "2026090340574444";
const ORIGINAL_SIZE: usize = 1_525_161;
const MOVIE_SIZE: usize = 2_197_545;
const PATCHED_MOVIE_SIZE: usize = 2_197_794;
const MAX_PACKED_SIZE: usize = 4 * 1024 * 1024;

fn resource_version(url: &Url) -> Option<(&'static str, &str)> {
    if !matches!(url.scheme(), "http" | "https") || url.host_str() != Some("aola.100bt.com")
        || url.port().is_some() || !url.username().is_empty() || url.password().is_some()
        || url.fragment().is_some() {
        return None;
    }
    let path = url.path().strip_suffix(".swf")?;
    let (resource, version) = if let Some(version) = path.strip_prefix(PREFIX) {
        (BACKPACK, version)
    } else {
        (UTIL, path.strip_prefix("/play/library/util~")?)
    };
    (!version.is_empty() && version.len() <= 24 && version.bytes().all(|c| c.is_ascii_digit())).then_some((resource, version))
}

pub(super) fn audit(mode: &str, resource: &str, version: &str, source: &str, result: &str, bytes: usize) {
    tracing::info!(target: "aola_swf_lifecycle", mode, resource, version, source, result, bytes = bytes as u64);
}

#[derive(Clone)]
pub(super) enum Trial {
    Capture(PathBuf),
    Fixed {
        mode: &'static str, original: Arc<[u8]>, output: Arc<[u8]>, util: Arc<[u8]>,
        run: Arc<Mutex<DependencyRun>>,
    },
}

#[derive(Default)]
pub(super) struct DependencyRun {
    pending: usize,
    verified: bool,
    invalid: bool,
}

fn read_limited(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take((MAX_PACKED_SIZE + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PACKED_SIZE { return Err(io::Error::other("oversized SWF")); }
    Ok(bytes)
}

// Unlike a player interpreting a potentially incomplete movie, this comparison requires
// the full declared movie including its stage header and every byte of every tag.
pub(super) fn decode_movie(bytes: &[u8], size: usize) -> io::Result<Vec<u8>> {
    let invalid = || io::Error::other("invalid roundtrip movie");
    if bytes.len() < 17 || &bytes[..4] != b"ZWS\x12"
        || bytes[4..8] != (size as u32).to_le_bytes()
        || bytes[8..12] != ((bytes.len() - 17) as u32).to_le_bytes() {
        return Err(invalid());
    }
    let mut body = vec![0; size - 8];
    let mut writer = Cursor::new(&mut body[..]);
    lzma_rs::lzma_decompress_with_options(&mut &bytes[12..], &mut writer, &Options {
        unpacked_size: UnpackedSize::UseProvided(None), memlimit: Some(32 * 1024 * 1024),
        allow_incomplete: false,
    }).map_err(|_| invalid())?;
    if writer.position() != (size - 8) as u64 { return Err(invalid()); }
    Ok(body)
}

impl Trial {
    pub(super) fn load(mode: &str, root: &Path, capture: &Path) -> io::Result<Self> {
        if mode == "capture" {
            return Ok(Self::Capture(capture.to_owned()));
        }
        let original: Arc<[u8]> = read_limited(&root.join("newpetpackage-original.swf"))?.into();
        if original.len() != ORIGINAL_SIZE { return Err(io::Error::other("invalid reference size")); }
        let movie = decode_movie(&original, MOVIE_SIZE)?;
        let (mode, output) = match mode {
            "original" => ("original", original.clone()),
            "roundtrip" => {
                let control = read_limited(&root.join("newpetpackage-roundtrip.swf"))?;
                if movie != decode_movie(&control, MOVIE_SIZE)? {
                    return Err(io::Error::other("control differs from original movie"));
                }
                ("roundtrip", control.into())
            }
            "lifecycle" => {
                let patched = read_limited(&root.join("newpetpackage-lifecycle.swf"))?;
                if patched.len() != 1_525_329 || !aola_skin_recipe::matches_skin_candidate(
                    &movie, &decode_movie(&patched, PATCHED_MOVIE_SIZE)?,
                ) { return Err(io::Error::other("candidate differs from approved skin changes")); }
                ("lifecycle", patched.into())
            }
            _ => return Err(io::Error::other("invalid experiment mode")),
        };
        let util = read_limited(&root.join("library_util~2026090340574444.swf"))?;
        if util.len() != 158_135 || !util.starts_with(b"CWS\x12") || util[4..8] != 335735u32.to_le_bytes() {
            return Err(io::Error::other("invalid utility reference"));
        }
        Ok(Self::Fixed { mode, original, output, util: util.into(), run: Arc::new(Mutex::default()) })
    }

    pub(super) fn mode(&self) -> &'static str {
        match self {
            Self::Capture(_) => "capture",
            Self::Fixed { mode, .. } => mode,
        }
    }

    pub(super) fn request_resource<'a>(&self, request: &Request, url: &'a Url) -> Option<(&'static str, &'a str)> {
        let Some((resource, version)) = resource_version(url) else {
            if matches!(self, Self::Fixed { .. }) && let Some(path) = url.path().strip_suffix(".swf") {
                let base = path.split_once('~').map_or(path, |(base, _)| base);
                let resource = match base {
                    "/play/library/util" => UTIL,
                    "/play/pet/newpetpackage" => BACKPACK,
                    _ => return None,
                };
                self.reject(resource, "", "request", "unexpected-identity");
            }
            return None;
        };
        if request.method() != NavigationMethod::Get || request.body().is_some() {
            self.reject(resource, version, "request", "unsupported-request");
            return None;
        }
        let expected = if resource == UTIL { UTIL_VERSION } else { VERSION };
        if !matches!(self, Self::Capture(_)) && version != expected {
            self.reject(resource, version, "request", "version-mismatch");
            return None;
        }
        if resource == UTIL && let Self::Fixed { run, .. } = self {
            run.lock().unwrap().pending += 1;
        }
        Some((resource, version))
    }

    pub(super) fn reject(&self, resource: &str, version: &str, source: &str, reason: &str) {
        if let Self::Fixed { run, .. } = self { run.lock().unwrap().invalid = true; }
        audit(self.mode(), resource, version, source, reason, 0);
    }

    fn select(&self, resource: &str, version: &str, source: &str, bytes: Vec<u8>) -> Vec<u8> {
        let mode = self.mode();
        match self {
            Self::Capture(dir) => {
                let result = match aola_swf_capture::write(dir, resource, version, &bytes) {
                    Ok(()) => "captured",
                    Err(_) => "capture-failed",
                };
                audit(mode, resource, version, source, result, bytes.len());
                bytes
            }
            Self::Fixed { original, util, output, run, .. } => {
                let mut state = run.lock().unwrap();
                let result = if resource == UTIL {
                    state.pending = state.pending.saturating_sub(1);
                    if bytes.as_slice() != util.as_ref() { "dependency-mismatch" }
                    else if state.invalid { "dependency-unconfirmed" }
                    else {
                        state.verified = true;
                        audit(mode, resource, version, source, "dependency-confirmed", bytes.len());
                        return bytes;
                    }
                } else if bytes.as_slice() != original.as_ref() { "source-mismatch" }
                else if state.invalid || state.pending > 0 || !state.verified { "dependency-unconfirmed" }
                else {
                    audit(mode, resource, version, source, "hit", output.len());
                    return output.to_vec();
                };
                state.invalid = true;
                audit(mode, resource, version, source, result, bytes.len());
                bytes
            }
        }
    }
}

pub(super) fn wrap(
    selection: Option<(Trial, &'static str, String)>, source: &'static str, response: Box<dyn SuccessResponse>,
) -> Box<dyn SuccessResponse> {
    let Some((trial, resource, version)) = selection else { return response };
    let same_resource = Url::parse(&response.url()).ok().is_some_and(|u| resource_version(&u) == Some((resource, version.as_str())));
    if response.status() != 200 || response.redirected() || !same_resource {
        trial.reject(resource, &version, source, "unexpected-response");
        return response;
    }
    super::fetch::filter_movie(response, TrialSelection { trial, resource, version, source })
}

struct TrialSelection {
    trial: Trial,
    resource: &'static str,
    version: String,
    source: &'static str,
}

impl super::fetch::MovieBodyFilter for TrialSelection {
    async fn complete(self, bytes: Vec<u8>) -> Vec<u8> {
        self.trial.select(self.resource, &self.version, self.source, bytes)
    }

    fn fallback(&self, reason: &'static str) {
        self.trial.reject(self.resource, &self.version, self.source, reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::fetch::{Response, ResponseBody};
    use futures_lite::future::block_on;

    fn fixed(mode: &'static str, original: &[u8], output: &[u8]) -> Trial {
        Trial::Fixed { mode, original: original.into(), output: output.into(), util: Arc::from(&b"utility"[..]),
            run: Arc::new(Mutex::default()) }
    }

    fn control() -> Trial {
        fixed("roundtrip", b"original", b"control")
    }

    fn version(trial: &Trial, url: &str, request: &Request) -> Option<String> {
        trial.request_resource(request, &Url::parse(url).unwrap()).map(|(_, version)| version.to_owned())
    }

    #[test]
    fn aola_lifecycle_matches_one_resource_and_safe_request() {
        let trial = control();
        let url = format!("https://aola.100bt.com{PREFIX}{VERSION}.swf");
        let get = Request::get(url.clone());
        assert_eq!(version(&trial, &url, &get).as_deref(), Some(VERSION));
        for bad in [url.replace(VERSION, "123"), url.replace("/play/", "/other/"),
            url.replace("100bt.com", "100bt.com.invalid"), url.replace("100bt.com", "100bt.com:8443"),
            url.replace("//aola", "//user@aola"), url.replace("https:", "file:"),
            url.replace(".swf", ".swf/extra")] {
            assert!(version(&trial, &bad, &get).is_none());
        }
        assert!(version(&trial, &url, &Request::post(url.clone(), None)).is_none());
        let mut with_body = get;
        with_body.set_body((vec![], String::new()));
        assert!(version(&trial, &url, &with_body).is_none());
    }

    #[test]
    fn aola_lifecycle_explicit_official_overrides_legacy() {
        use super::super::AolaSwfMode::*;
        use super::super::AolaSwfMode;
        assert_eq!(AolaSwfMode::select("official", "optimized"), Official);
        assert_eq!(AolaSwfMode::select("", "baseline"), Legacy { optimized: false });
        assert_eq!(AolaSwfMode::select("roundtrip", "optimized"), Lifecycle { mode: "roundtrip", conflict: true });
        assert_eq!(AolaSwfMode::select("original", ""), Lifecycle { mode: "original", conflict: false });
    }

    fn exercise(trial: Option<Trial>, input: &[u8], status: u16, redirected: bool, origin: &'static str) -> Vec<u8> {
        let response = Box::new(Response { url: format!("https://aola.100bt.com{PREFIX}{VERSION}.swf"),
            response_body: fixture_body(origin, input, status), text_encoding: None, status, redirected });
        let response = wrap(trial.map(|t| (t, BACKPACK, VERSION.into())), origin, response);
        assert_eq!(response.status(), status);
        assert_eq!(response.redirected(), redirected);
        block_on(response.body()).unwrap()
    }

    fn fixture_body(source: &str, bytes: &[u8], status: u16) -> ResponseBody {
        if source == "network" {
            let http_response = http::Response::builder().status(status).body(bytes.to_vec()).unwrap();
            ResponseBody::Network(Arc::new(Mutex::new(Some(reqwest::Response::from(http_response)))))
        } else {
            ResponseBody::File(Ok(bytes.to_vec()))
        }
    }

    #[test]
    fn aola_lifecycle_both_delivery_paths_preserve_rejected_or_disabled_responses() {
        for origin in ["cache", "network"] {
            let trial = control();
            assert_eq!(deliver(&trial, UTIL, UTIL_VERSION, b"utility", origin), b"utility");
            assert_eq!(exercise(Some(trial), b"original", 200, false, origin), b"control");
            assert_eq!(exercise(Some(control()), b"changed!", 200, false, origin), b"changed!");
            assert_eq!(exercise(Some(control()), b"original", 206, false, origin), b"original");
            assert_eq!(exercise(Some(control()), b"original", 200, true, origin), b"original");
            assert_eq!(exercise(None, b"original", 200, false, origin), b"original");
        }
    }

    #[test]
    fn aola_lifecycle_rejects_changed_final_resource_without_reading_it_early() {
        let response = Box::new(Response {
            url: format!("https://different-host.test{PREFIX}{VERSION}.swf"),
            response_body: ResponseBody::File(Ok(b"original".to_vec())),
            text_encoding: None,
            status: 200,
            redirected: false,
        });
        let mut actual = wrap(Some((control(), BACKPACK, VERSION.into())), "cache", response);
        assert_eq!(block_on(actual.next_chunk()).unwrap().as_deref(), Some(&b"original"[..]));
        assert!(block_on(actual.next_chunk()).unwrap().is_none());
    }

    #[test]
    fn aola_lifecycle_mixed_reads_preserve_only_the_unread_bytes() {
        let url = format!("https://aola.100bt.com{PREFIX}{VERSION}.swf");
        let response = Box::new(Response { url, response_body: ResponseBody::File(Ok(b"first".to_vec())),
            text_encoding: None, status: 200, redirected: false });
        // The file response yields its entire content in the first chunk, so its
        // unread suffix is empty. A complete-body trial would substitute this sentinel.
        let trial = fixed("roundtrip", b"", b"bad");
        let mut response = wrap(Some((trial, BACKPACK, VERSION.into())), "cache", response);
        assert_eq!(block_on(response.next_chunk()).unwrap().as_deref(), Some(&b"first"[..]));
        assert!(block_on(response.body()).unwrap().is_empty());
    }

    #[test]
    fn aola_lifecycle_capture_can_observe_new_versions_and_preserves_failed_writes() {
        let dir = tempfile::tempdir().unwrap();
        let capture = Trial::Capture(dir.path().to_owned());
        let url = format!("https://aola.100bt.com{PREFIX}123.swf");
        assert_eq!(version(&capture, &url, &Request::get(url.clone())).as_deref(), Some("123"));
        assert_eq!(capture.select(BACKPACK, "123", "network", b"not a movie".to_vec()), b"not a movie");
        let util = Url::parse("https://aola.100bt.com/play/library/util~2026090340574444.swf").unwrap();
        let request = Request::get(util.to_string());
        assert_eq!(capture.request_resource(&request, &util), Some((UTIL, "2026090340574444")));
        assert_eq!(control().request_resource(&request, &util), Some((UTIL, UTIL_VERSION)));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    fn skin() -> Trial {
        fixed("lifecycle", b"original", b"approved")
    }

    fn deliver(trial: &Trial, resource: &'static str, version: &str, bytes: &[u8], source: &'static str) -> Vec<u8> {
        let url = Url::parse(&format!("https://aola.100bt.com/play/{resource}~{version}.swf")).unwrap();
        let selected = trial.request_resource(&Request::get(url.to_string()), &url)
            .map(|(id, v)| (trial.clone(), id, v.to_owned()));
        let response = Box::new(Response { url: url.to_string(), response_body: fixture_body(source, bytes, 200),
            text_encoding: None, status: 200, redirected: false });
        block_on(wrap(selected, source, response).body()).unwrap()
    }

    #[test]
    fn aola_lifecycle_prior_backpack_version_falls_back_in_all_fixed_modes() {
        let old = "2026102936286521";
        for source in ["cache", "network"] {
            let original = fixed("original", b"original", b"original");
            for trial in [original, control(), skin()] {
                let url = Url::parse(&format!("https://aola.100bt.com/play/{BACKPACK}~{old}.swf")).unwrap();
                assert!(trial.request_resource(&Request::get(url.to_string()), &url).is_none());
                assert_eq!(deliver(&trial, BACKPACK, old, b"original", source), b"original");
                assert_eq!(deliver(&trial, UTIL, UTIL_VERSION, b"utility", source), b"utility");
                assert_eq!(deliver(&trial, BACKPACK, VERSION, b"original", source), b"original");
                let Trial::Fixed { run, .. } = &trial else { unreachable!() };
                assert!(run.lock().unwrap().invalid);
            }
        }
    }

    #[test]
    fn aola_lifecycle_dependency_order_and_conflicts_are_sticky_for_this_run() {
        for source in ["cache", "network"] {
            for (resource, version, bytes) in [(UTIL, "111", &b"utility"[..]), (UTIL, UTIL_VERSION, &b"wrong!!"[..]),
                (BACKPACK, VERSION, &b"original"[..]), (BACKPACK, "111", &b"original"[..])] {
                for trial in [fixed("original", b"original", b"original"), control(), skin()] {
                    assert_eq!(deliver(&trial, resource, version, bytes, source), bytes);
                    assert_eq!(deliver(&trial, UTIL, UTIL_VERSION, b"utility", source), b"utility");
                    assert_eq!(deliver(&trial, BACKPACK, VERSION, b"original", source), b"original");
                    let Trial::Fixed { run, .. } = &trial else { unreachable!() };
                    assert!(run.lock().unwrap().invalid);
                }
            }
            let trial = skin();
            assert_eq!(deliver(&trial, UTIL, UTIL_VERSION, b"utility", source), b"utility");
            assert_eq!(deliver(&trial, BACKPACK, VERSION, b"original", source), b"approved");
            assert_eq!(deliver(&trial, BACKPACK, VERSION, b"modified", source), b"modified");
            assert_eq!(deliver(&trial, BACKPACK, VERSION, b"original", source), b"original");
            let trial = skin();
            let url = Url::parse(&format!("https://aola.100bt.com/play/{UTIL}~{UTIL_VERSION}.swf")).unwrap();
            trial.request_resource(&Request::get(url.to_string()), &url).unwrap();
            // One matching response completed while another dependency read is outstanding.
            assert_eq!(deliver(&trial, UTIL, UTIL_VERSION, b"utility", source), b"utility");
            assert_eq!(deliver(&trial, BACKPACK, VERSION, b"original", source), b"original");
            let trial = skin();
            assert_eq!(deliver(&trial, UTIL, UTIL_VERSION, b"utility", source), b"utility");
            let unexpected = Url::parse("https://other.invalid/play/library/util~111.swf").unwrap();
            assert!(trial.request_resource(&Request::get(unexpected.to_string()), &unexpected).is_none());
            assert_eq!(deliver(&trial, BACKPACK, VERSION, b"original", source), b"original");
        }
    }

    #[test]
    fn aola_lifecycle_rejects_missing_or_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        for mode in ["invalid-mode", "original", "roundtrip"] {
            assert!(Trial::load(mode, dir.path(), dir.path()).is_err());
        }
        std::fs::write(dir.path().join("newpetpackage-original.swf"), vec![0; ORIGINAL_SIZE]).unwrap();
        std::fs::write(dir.path().join("newpetpackage-roundtrip.swf"), b"wrong").unwrap();
        assert!(Trial::load("original", dir.path(), dir.path()).is_err());
        assert!(Trial::load("roundtrip", dir.path(), dir.path()).is_err());
    }

    #[test]
    #[ignore = "requires the archived SWF and a locally rebuilt roundtrip"]
    fn aola_lifecycle_real_control() {
        let root = PathBuf::from(std::env::var_os("AOLA_TEST_SWF_FILES").expect("AOLA_TEST_SWF_FILES"));
        let original = std::fs::read(root.join("newpetpackage-original.swf")).unwrap();
        let control = std::fs::read(root.join("newpetpackage-roundtrip.swf")).unwrap();
        let util = std::fs::read(root.join("library_util~2026090340574444.swf")).unwrap();
        assert_ne!(original, control, "test must select distinct encoded bytes");
        for (mode, output) in [("roundtrip", &control), ("original", &original)] {
            let trial = Trial::load(mode, &root, &root).unwrap();
            assert_eq!(deliver(&trial, UTIL, UTIL_VERSION, &util, "network"), util);
            assert_eq!(&exercise(Some(trial.clone()), &original, 200, false, "network"), output);
            let Trial::Fixed { run, .. } = &trial else { unreachable!() };
            let state = run.lock().unwrap();
            assert!(state.verified && !state.invalid && state.pending == 0);
        }
    }

    #[test]
    #[ignore = "requires the four SWFs produced by the local experiment packager"]
    fn aola_lifecycle_real_candidate_cannot_substitute_modified_content() {
        let root = PathBuf::from(std::env::var_os("AOLA_TEST_SWF_FILES").unwrap());
        let dir = tempfile::tempdir().unwrap();
        for name in ["newpetpackage-original.swf", "library_util~2026090340574444.swf"] {
            std::fs::copy(root.join(name), dir.path().join(name)).unwrap();
        }
        let name = "newpetpackage-lifecycle.swf";
        let mut candidate = std::fs::read(root.join(name)).unwrap();
        assert!(Trial::load("lifecycle", &root, &root).is_ok());
        candidate[10000] ^= 0x04;
        std::fs::write(dir.path().join(name), candidate).unwrap();
        assert!(Trial::load("lifecycle", dir.path(), dir.path()).is_err());
    }
}
