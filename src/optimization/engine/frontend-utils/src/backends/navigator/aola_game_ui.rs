//! Opt-in local presentation assets for the versioned start and control-bar movies.
use super::fetch::{self, Response, ResponseBody};
use ruffle_core::backend::navigator::{
    ErrorResponse, NavigationMethod, OwnedFuture, Request, SuccessResponse,
};
use ruffle_core::loader::Error;
use std::path::{Path, PathBuf};
use url::Url;

#[derive(Clone, Copy)]
pub(super) enum Asset {
    Start,
    Control,
    Skin(u8),
}

impl Asset {
    pub(super) fn requested(request: &Request, url: &Url) -> Option<Self> {
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str() != Some("aola.100bt.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || request.method() != NavigationMethod::Get
            || request.body().is_some()
            || request
                .headers()
                .keys()
                .any(|key| key.eq_ignore_ascii_case("range"))
        {
            return None;
        }
        match url.path() {
            "/play/start~2107300001.swf" => Some(Self::Start),
            "/play/control/controlv2~2026082731219998.swf" => Some(Self::Control),
            path => {
                let id = path
                    .strip_prefix("/play/aola-helper/theme/skin")?
                    .strip_suffix(".swf")?;
                let number = id.parse::<u8>().ok()?;
                ((1..=10).contains(&number) && id == number.to_string())
                    .then_some(Self::Skin(number))
            }
        }
    }
}

fn fws(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(b"FWS\x12")
        && bytes.get(4..8) == Some((bytes.len() as u32).to_le_bytes().as_slice())
}

fn matches_profile(official: &[u8], mut candidate: &[u8], mut profile: &[u8]) -> bool {
    // Each record is (official offset, count). The reserved offset is followed by literal bytes.
    let mut decoded = Vec::new();
    if lzma_rs::lzma_decompress(&mut profile, &mut decoded).is_err() {
        return false;
    }
    let mut edits = decoded.as_slice();
    while !edits.is_empty() {
        let Some((pair, tail)) = edits.split_at_checked(8) else {
            return false;
        };
        let at = u32::from_le_bytes(pair[..4].try_into().unwrap());
        let count = u32::from_le_bytes(pair[4..].try_into().unwrap()) as usize;
        edits = tail;
        let bytes = if at == u32::MAX {
            let Some((literal, tail)) = edits.split_at_checked(count) else {
                return false;
            };
            edits = tail;
            literal
        } else {
            let Some(slice) = official.get(at as usize..).and_then(|s| s.get(..count)) else {
                return false;
            };
            slice
        };
        let Some(rest) = candidate.strip_prefix(bytes) else {
            return false;
        };
        candidate = rest;
    }
    candidate.is_empty()
}

pub(super) fn leaf(
    root: &Path,
    id: u8,
    url: String,
) -> OwnedFuture<Box<dyn SuccessResponse>, ErrorResponse> {
    let path = root.join(format!("skin{id}.swf"));
    Box::pin(async move {
        let bytes = tokio::task::spawn_blocking(move || std::fs::read(path))
            .await
            .ok()
            .and_then(Result::ok)
            .filter(|b| fws(b));
        let Some(bytes) = bytes else {
            return Err(ErrorResponse {
                url,
                error: Error::FetchError("Local theme unavailable".into()),
            });
        };
        Ok(Box::new(Response {
            url,
            response_body: ResponseBody::File(Ok(bytes)),
            status: 200,
            text_encoding: None,
            redirected: false,
        }) as Box<dyn SuccessResponse>)
    })
}

pub(super) struct Selection {
    pub root: PathBuf,
    pub url: String,
    pub asset: Asset,
}

impl Selection {
    pub(super) fn wrap(self, response: Box<dyn SuccessResponse>) -> Box<dyn SuccessResponse> {
        if response.status() == 200 && !response.redirected() && response.url() == self.url {
            fetch::filter_movie(response, self)
        } else {
            response
        }
    }

    fn audit(&self, result: &'static str) {
        let (source, version) = match self.asset {
            Asset::Start => ("start", "2107300001"),
            Asset::Control => ("control", "2026082731219998"),
            Asset::Skin(_) => return,
        };
        tracing::info!(target:"aola_game_ui",source,version,result);
    }

    fn candidate(&self, received: &[u8]) -> Option<Vec<u8>> {
        let (original, patched, packed_len, movie_len, profile): (_, _, _, _, &[u8]) =
            match self.asset {
                Asset::Start => (
                    "start-official.swf",
                    "start-ui.swf",
                    2495,
                    3929,
                    include_bytes!("aola_game_ui_start.edits.lzma"),
                ),
                Asset::Control => (
                    "control-official.swf",
                    "control-ui.swf",
                    1798707,
                    4961288,
                    include_bytes!("aola_game_ui_control.edits.lzma"),
                ),
                Asset::Skin(_) => return None,
            };
        if received.len() != packed_len
            || std::fs::read(self.root.join(original)).as_deref().ok() != Some(received)
        {
            self.audit("source-mismatch");
            return None;
        }
        let candidate = (|| {
            let mut fws_source = b"FWS".to_vec();
            fws_source.extend_from_slice(&received[3..8]);
            fws_source.extend(super::aola_lifecycle::decode_movie(received, movie_len).ok()?);
            let result = std::fs::read(self.root.join(patched)).ok()?;
            matches_profile(&fws_source, &result, profile).then_some(result)
        })();
        self.audit(if candidate.is_some() {
            "hit"
        } else {
            "invalid-candidate"
        });
        candidate
    }
}

impl fetch::MovieBodyFilter for Selection {
    async fn complete(self, bytes: Vec<u8>) -> Vec<u8> {
        let original = std::sync::Arc::new(bytes);
        let input = original.clone();
        let selected = tokio::task::spawn_blocking(move || self.candidate(&input))
            .await
            .ok()
            .flatten();
        selected.unwrap_or_else(|| std::sync::Arc::unwrap_or_clone(original))
    }
    fn fallback(&self, reason: &'static str) {
        self.audit(reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn aola_game_ui_release_candidates_reject_non_marker_corruption() {
        let Some(bundle) = std::env::var_os("AOLA_GAME_UI_TEST_BUNDLE").map(PathBuf::from) else {
            return;
        };
        let scratch = tempfile::tempdir().unwrap();
        for (asset, name) in [(Asset::Start, "start"), (Asset::Control, "control")] {
            let original = format!("{name}-official.swf");
            let candidate = format!("{name}-ui.swf");
            let bytes = std::fs::read(bundle.join(&original)).unwrap();
            std::fs::write(scratch.path().join(&original), &bytes).unwrap();
            let mut file = std::fs::read(bundle.join(&candidate)).unwrap();
            let local = scratch.path().join(&candidate);
            let url = match asset {
                Asset::Start => "https://aola.100bt.com/play/start~2107300001.swf",
                _ => "https://aola.100bt.com/play/control/controlv2~2026082731219998.swf",
            };
            let select = || Selection {
                root: scratch.path().to_owned(),
                url: url.into(),
                asset,
            };
            let response = || {
                Box::new(Response {
                    url: url.into(),
                    response_body: ResponseBody::File(Ok(bytes.clone())),
                    text_encoding: None,
                    status: 200,
                    redirected: false,
                }) as Box<dyn SuccessResponse>
            };
            std::fs::write(&local, &file).unwrap();
            assert_eq!(select().wrap(response()).body().await.unwrap(), file);
            let i = file.len() - 3;
            file[i] ^= 1;
            std::fs::write(&local, &file).unwrap();
            assert!(fws(&file));
            assert_eq!(select().wrap(response()).body().await.unwrap(), bytes);
        }
    }
    #[test]
    fn aola_game_ui_limits_routes_to_official_version_and_ten_named_skins() {
        for (url, allowed) in [
            (
                "https://aola.100bt.com/play/start~2107300001.swf?ts=1",
                true,
            ),
            ("https://aola.100bt.com/play/start~1.swf", false),
            (
                "https://aola.100bt.com/play/control/controlv2~2026082731219998.swf",
                true,
            ),
            (
                "https://aola.100bt.com/play/aola-helper/theme/skin10.swf",
                true,
            ),
            (
                "https://aola.100bt.com/play/aola-helper/theme/skin01.swf",
                false,
            ),
            (
                "https://aola.100bt.com/play/aola-helper/theme/skin11.swf",
                false,
            ),
            (
                "https://other.example/play/aola-helper/theme/skin1.swf",
                false,
            ),
        ] {
            let address = Url::parse(url).unwrap();
            assert_eq!(
                Asset::requested(&Request::get(url.into()), &address).is_some(),
                allowed
            );
            assert!(Asset::requested(&Request::post(url.into(), None), &address).is_none());
        }
    }
}
