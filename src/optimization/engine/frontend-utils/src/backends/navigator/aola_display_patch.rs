//! Apply display-only edits to complete responses for exact official resource versions.
use super::{aola_battle_edits, aola_lifecycle::decode_movie, aola_notice_edits, fetch};
use ruffle_core::backend::navigator::{NavigationMethod, Request, SuccessResponse};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use url::Url;

pub(super) struct Recipe {
    path: &'static str,
    version: &'static str,
    original_size: usize,
    movie_size: usize,
    edits: &'static [(usize, &'static [u8], &'static [u8])],
    battle: bool,
}

pub(super) static NOTICE: Recipe = Recipe {
    path: "/play/messagesend/messagesendservice",
    version: "2026062579184482",
    original_size: 221_474,
    movie_size: 0x4a41f,
    edits: aola_notice_edits::EDITS,
    battle: false,
};

pub(super) static BATTLE: Recipe = Recipe {
    path: "/play/pet/petfightnew",
    version: "261112849126071",
    original_size: 5_377_898,
    movie_size: 6_573_705,
    edits: aola_battle_edits::EDITS,
    battle: true,
};

impl Recipe {
    pub(super) fn audit(&self, source: &'static str, result: &'static str) {
        let version = self.version;
        if self.battle {
            tracing::info!(target: "aola_battle", source, result, version);
        } else {
            tracing::info!(target: "aola_notice", source, result, version);
        }
    }

    fn official_version<'a>(&self, url: &'a Url) -> Option<&'a str> {
        if !matches!(url.scheme(), "http" | "https") || url.host_str() != Some("aola.100bt.com")
            || url.port().is_some() || !url.username().is_empty() || url.password().is_some()
            || url.fragment().is_some() {
            return None;
        }
        let suffix = url.path().strip_prefix(self.path)?.strip_suffix(".swf")?;
        if suffix.is_empty() { return Some(""); }
        let version = suffix.strip_prefix('~')?;
        (!version.is_empty() && version.len() <= 32 && version.bytes().all(|b| b.is_ascii_digit())).then_some(version)
    }

    pub(super) fn accepts(&self, request: &Request, url: &Url) -> bool {
        let Some(version) = self.official_version(url) else { return false };
        if version != self.version {
            self.audit("request", "version-mismatch");
            return false;
        }
        if request.method() != NavigationMethod::Get || request.body().is_some()
            || request.headers().keys().any(|name| name.eq_ignore_ascii_case("range")) {
            self.audit("request", "unsupported-request");
            return false;
        }
        true
    }

    fn build(&self, original: &[u8]) -> io::Result<Vec<u8>> {
        if original.len() != self.original_size {
            return Err(io::Error::other("unexpected display SWF size"));
        }
        let mut movie = Vec::with_capacity(self.movie_size);
        movie.extend_from_slice(b"FWS");
        movie.extend_from_slice(&original[3..8]);
        movie.extend(decode_movie(original, self.movie_size)?);
        let mut candidate = Vec::with_capacity(movie.len() + 512);
        let mut cursor = 0;
        for &(offset, removed, added) in self.edits {
            if offset < cursor || movie.get(offset..offset + removed.len()) != Some(removed) {
                return Err(io::Error::other("unrecognized display SWF body"));
            }
            candidate.extend_from_slice(&movie[cursor..offset]);
            candidate.extend_from_slice(added);
            cursor = offset + removed.len();
        }
        candidate.extend_from_slice(&movie[cursor..]);
        if candidate.get(4..8) != Some((candidate.len() as u32).to_le_bytes().as_slice()) {
            return Err(io::Error::other("inconsistent display candidate length"));
        }
        Ok(candidate)
    }
}

pub(super) struct DisplayPatch {
    pub(super) recipe: &'static Recipe,
    root: PathBuf,
    original: OnceLock<io::Result<Vec<u8>>>,
    candidate: OnceLock<io::Result<Vec<u8>>>,
}

impl DisplayPatch {
    pub(super) fn new(recipe: &'static Recipe, root: &Path) -> Self {
        Self { recipe, root: root.to_owned(), original: OnceLock::new(), candidate: OnceLock::new() }
    }

    fn original(&self) -> io::Result<&[u8]> {
        self.original.get_or_init(|| {
            let mut bytes = Vec::with_capacity(self.recipe.original_size);
            std::fs::File::open(self.root.join("original.swf"))?
                .take(self.recipe.original_size as u64 + 1).read_to_end(&mut bytes)?;
            if bytes.len() != self.recipe.original_size {
                return Err(io::Error::other("unexpected display SWF size"));
            }
            Ok(bytes)
        }).as_deref().map_err(|error| io::Error::new(error.kind(), error.to_string()))
    }

    pub(super) fn candidate(&self) -> io::Result<&[u8]> {
        self.candidate.get_or_init(|| self.recipe.build(self.original()?))
            .as_deref().map_err(|error| io::Error::new(error.kind(), error.to_string()))
    }

    fn select(&self, source: &'static str, bytes: &[u8]) -> Option<Vec<u8>> {
        let Ok(original) = self.original() else {
            self.recipe.audit(source, "invalid-reference");
            return None;
        };
        if original != bytes {
            self.recipe.audit(source, "source-mismatch");
            return None;
        }
        let Ok(candidate) = self.candidate() else {
            self.recipe.audit(source, "invalid-reference");
            return None;
        };
        self.recipe.audit(source, "hit");
        Some(candidate.to_vec())
    }
}

pub(super) fn wrap(
    patch: Option<Arc<DisplayPatch>>, source: &'static str, response: Box<dyn SuccessResponse>,
) -> Box<dyn SuccessResponse> {
    let Some(patch) = patch else { return response };
    let recipe = patch.recipe;
    let same = Url::parse(&response.url()).ok().is_some_and(|url| recipe.official_version(&url) == Some(recipe.version));
    if response.status() != 200 || response.redirected() || !same {
        recipe.audit(source, "unexpected-response");
        return response;
    }
    fetch::filter_movie(response, DisplaySelection { patch, source })
}

struct DisplaySelection {
    patch: Arc<DisplayPatch>,
    source: &'static str,
}

impl fetch::MovieBodyFilter for DisplaySelection {
    async fn complete(self, bytes: Vec<u8>) -> Vec<u8> {
        if !self.patch.recipe.battle {
            return self.patch.select(self.source, &bytes).unwrap_or(bytes);
        }
        if bytes.len() != self.patch.recipe.original_size {
            self.fallback("source-mismatch");
            return bytes;
        }
        // Read, compare and decompress on the IO worker. Preserve the received
        // source even if the worker fails, and share a single build across loads.
        let official = Arc::new(bytes);
        let input = official.clone();
        let recipe = self.patch.recipe;
        let source = self.source;
        let result = tokio::task::spawn_blocking(move || self.patch.select(source, &input)).await;
        result.unwrap_or_else(|_| {
            recipe.audit(source, "worker-failed");
            None
        }).unwrap_or_else(|| Arc::unwrap_or_clone(official))
    }

    fn fallback(&self, reason: &'static str) { self.patch.recipe.audit(self.source, reason); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::fetch::{Response, ResponseBody};
    use futures_lite::future::block_on;

    fn url() -> String { format!("https://aola.100bt.com{}~{}.swf", NOTICE.path, NOTICE.version) }

    #[test]
    fn aola_notice_accepts_only_the_versioned_official_get() {
        for recipe in [&NOTICE, &BATTLE] {
            let official = format!("https://aola.100bt.com{}~{}.swf", recipe.path, recipe.version);
            let url = Url::parse(&official).unwrap();
            let get = Request::get(official.clone());
            assert!(recipe.accepts(&get, &url));
            for other in [official.replace("100bt.com", "100bt.com.test"), official.replace("100bt.com", "100bt.com:8000"),
                official.replace("//aola", "//user@aola"), official.replace(recipe.version, "123"),
                official.replace("~", "/"), official.replace(".swf", ".swf/other")] {
                assert!(!recipe.accepts(&get, &Url::parse(&other).unwrap()));
            }
            assert!(!recipe.accepts(&Request::post(official, None), &url));
            let mut range = get;
            range.set_headers([("Range".into(), "bytes=0-99".into())].into_iter().collect());
            assert!(!recipe.accepts(&range, &url));
        }
    }

    #[tokio::test]
    async fn aola_battle_builds_only_for_verified_bytes_and_shares_one_candidate() {
        // ZWS envelope containing the LZMA-compressed text "synthetic battle body".
        let original = vec![90, 87, 83, 18, 29, 0, 0, 0, 31, 0, 0, 0, 93, 0, 0, 128,
            0, 0, 57, 158, 74, 1, 1, 252, 162, 252, 212, 28, 54, 95, 148, 55, 100,
            64, 237, 34, 58, 211, 81, 92, 200, 157, 149, 255, 242, 199, 48, 0];
        static EXAMPLE: Recipe = Recipe {
            path: "/test", version: "1", original_size: 48, movie_size: 29,
            edits: &[(28, b"y", b"!")], battle: true,
        };
        let expected = b"FWS\x12\x1d\0\0\0synthetic battle bod!";
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("original.swf"), &original).unwrap();
        let patch = Arc::new(DisplayPatch::new(&EXAMPLE, temp.path()));
        assert!(patch.original.get().is_none());

        let selection = || DisplaySelection { patch: patch.clone(), source: "network" };
        let mut changed = original.clone();
        *changed.last_mut().unwrap() ^= 1;
        use fetch::MovieBodyFilter;
        assert!(selection().complete(changed.clone()).await == changed);
        assert!(patch.candidate.get().is_none());
        let (first, second) = futures_lite::future::zip(
            selection().complete(original.clone()), selection().complete(original)
        ).await;
        assert_eq!(first, expected);
        assert_eq!(second, expected);
        assert!(patch.candidate.get().is_some());
    }

    #[test]
    fn aola_notice_replaces_only_the_matching_complete_response() {
        fn response(input: &[u8], status: u16, redirected: bool) -> Box<dyn SuccessResponse> {
            Box::new(Response { url: url(), response_body: ResponseBody::File(Ok(input.to_vec())),
                text_encoding: None, status, redirected })
        }
        let patch = Arc::new(DisplayPatch::new(&NOTICE, Path::new(".")));
        patch.original.set(Ok(b"official".to_vec())).unwrap();
        patch.candidate.set(Ok(b"candidate".to_vec())).unwrap();
        for source in ["network", "cache"] {
            for (input, status, redirected, output) in [
                (&b"official"[..], 200, false, &b"candidate"[..]), (b"new-data", 200, false, b"new-data"),
                (b"official", 206, false, b"official"), (b"official", 200, true, b"official"),
            ] {
                let actual = wrap(Some(patch.clone()), source, response(input, status, redirected));
                assert_eq!(block_on(actual.body()).unwrap(), output);
            }
            let mut streamed = wrap(Some(patch.clone()), source, response(b"official", 200, false));
            assert_eq!(block_on(streamed.next_chunk()).unwrap().unwrap(), b"official");
            assert!(block_on(streamed.body()).unwrap().is_empty());
            assert_eq!(block_on(wrap(None, source, response(b"official", 200, false)).body()).unwrap(), b"official");
        }
        let mut foreign = response(b"official", 200, false);
        foreign.set_url(url().replace("aola.100bt.com", "different.test"));
        assert_eq!(block_on(wrap(Some(patch.clone()), "network", foreign).body()).unwrap(), b"official");
        let http = http::Response::builder().status(200).body(b"official".to_vec()).unwrap();
        let network = Box::new(Response { url: url(), status: 200, redirected: false, text_encoding: None,
            response_body: ResponseBody::Network(Arc::new(std::sync::Mutex::new(Some(reqwest::Response::from(http))))) });
        assert_eq!(block_on(wrap(Some(patch), "network", network).body()).unwrap(), b"candidate");
        assert!(NOTICE.build(&vec![0; NOTICE.original_size]).is_err());
    }

    #[test]
    fn aola_notice_rejects_bad_bundles_and_compares_the_configured_fixture() {
        let temp = tempfile::tempdir().unwrap();
        assert!(DisplayPatch::new(&NOTICE, temp.path()).candidate().is_err());
        std::fs::write(temp.path().join("original.swf"), b"ZWS").unwrap();
        assert!(DisplayPatch::new(&NOTICE, temp.path()).candidate().is_err());

        // The offline packager can supply an independent reconstruction of the
        // real movie for byte-for-byte comparison in addition to the failure cases.
        if let Some(dir) = std::env::var_os("AOLA_TEST_NOTICE_FILES") {
            let root = Path::new(&dir);
            let expected = std::fs::read(root.join("messagesendservice-notice.swf")).unwrap();
            let patch = DisplayPatch::new(&NOTICE, root);
            assert_eq!(patch.candidate().unwrap(), expected.as_slice());
        }
    }
}
