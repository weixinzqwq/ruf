//! Serve adapted Aola resources without putting them in the official URL cache.

use super::aola_morph_transform::{self, MorphKind};
use super::fetch::{self, MovieBodyFilter, Response, ResponseBody};
use super::spawn_tokio;
use crate::backends::aoqi_cache::{AdaptedKey, AoqiCache, numeric_version};
use quick_xml::{Reader, events::Event};
use ruffle_core::backend::navigator::{NavigationMethod, Request, SuccessResponse};
use ruffle_core::mypro_observer;
use ruffle_core::swf::{self, Tag};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use url::Url;

const OFFICIAL_HOST: &str = "aola.100bt.com";
const BACKPACK: &str = "peticon/movie/type0/";
const BATTLE: &str = "fightassets/bitmap/";
const FIGHT_HEAD_ICON: &str = "peticon/petfighticon/peticon";
const LEGACY_HEAD_ICON: &str = "peticon/peticon";
const POSTER: &str = "pet/petskin/background/";
const VERSION_CLASS: &[u8] = b"mmo.versiondata.VersionData_versionClass";
// VersManager.getVersion uses this SWF version when setVersionXml has no row.
const NATIVE_DEFAULT_VERSION: &str = "1210";
const MAX_CATALOG_BYTES: u32 = 16 * 1024 * 1024;
const MAX_TARGET_BYTES: usize = 128 * 1024 * 1024;

type FetchError = Box<dyn std::error::Error + Send + Sync>;
type Versions = HashMap<String, Option<String>>;
pub(super) type Catalog = Arc<RwLock<CatalogContents>>;

#[derive(Default)]
pub(super) struct CatalogContents {
    base: Option<Versions>,
    startup: Versions,
}

impl CatalogContents {
    fn target_version(&self, name: &str) -> Option<String> {
        let base = self.base.as_ref()?;
        self.startup.get(name).or_else(|| base.get(name)).cloned()
            .unwrap_or_else(|| Some(NATIVE_DEFAULT_VERSION.to_owned()))
    }
}

#[derive(Clone, Copy)]
#[derive(PartialEq, Eq)]
enum CatalogSource { Base, Startup }

fn catalog_source(url: &Url) -> Option<CatalogSource> {
    let path = url.path().strip_prefix("/play/")?;
    if path == "start.xml" { return Some(CatalogSource::Startup); }
    let (file, versioned) = path.split_once('~')?;
    let (version, extension) = versioned.split_once('.')?;
    if !numeric_version(version) { return None; }
    match (file, extension) {
        ("versiondata", "swf") => Some(CatalogSource::Base),
        ("start", "xml") => Some(CatalogSource::Startup),
        _ => None,
    }
}

#[derive(Clone, Copy)]
pub(super) struct Selection {
    source: u32,
    target: u32,
    kind: MorphKind,
}

pub(super) fn try_match(request: &Request, url: &Url) -> Option<Selection> {
    let (source, kind) = parse_versioned_asset(url)?;
    if !AoqiCache::accepts_request(request, url) {
        return None;
    }
    let rules = mypro_observer::morph_rule_snapshot();
    let target = match kind {
        MorphKind::Backpack { .. } if rules.enabled => rules.backpack_map.get(&source),
        MorphKind::Battle { face, .. } if face == 2 || rules.enemy_enabled => {
            rules.battle_map.get(&source)
        }
        MorphKind::HeadIcon { .. } => rules.head_icon_map.get(&source),
        // The backpack drives the native poster request with its visual skin ID.
        _ => None,
    };
    let target = target.copied().filter(|&id| id != 0 && id != source)?;
    Some(Selection { source, target, kind })
}

pub(super) fn parse_versioned_asset(url: &Url) -> Option<(u32, MorphKind)> {
    if !official(url) { return None; }
    let path = url.path().strip_prefix("/play/")?.strip_suffix(".swf")?;
    let (name, version) = path.rsplit_once('~')?;
    if !numeric_version(version) { return None; }
    parse_asset(name)
}

fn official(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.host_str() == Some(OFFICIAL_HOST)
        && url.port().is_none() && url.username().is_empty() && url.password().is_none()
        && url.fragment().is_none()
}

fn id(text: &str) -> Option<u32> {
    if text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
    text.parse().ok()
}

fn parse_asset(path: &str) -> Option<(u32, MorphKind)> {
    if let Some(path) = path.strip_prefix(BACKPACK) {
        let (directory, filename) = path.split_once('/')?;
        let (stem, index) = filename.rsplit_once('_')?;
        if directory != stem { return None; }
        let source = id(stem.strip_prefix("petmovie")?)?;
        let index = match index { "0" => 0, "1" => 1, "2" => 2, "3" => 3, _ => return None };
        Some((source, MorphKind::Backpack { index }))
    } else if let Some(path) = path.strip_prefix(BATTLE) {
        let (directory, filename) = path.split_once('/')?;
        let (stem_face, part) = filename.rsplit_once('_')?;
        let (stem, face) = stem_face.rsplit_once('_')?;
        if directory != stem { return None; }
        let source = id(stem.strip_prefix("pet")?)?;
        let face = match face { "1" => 1, "2" => 2, _ => return None };
        let part = match part { "0" => 0, "1" => 1, "2" => 2, _ => return None };
        Some((source, MorphKind::Battle { face, part }))
    } else if let Some(source) = path.strip_prefix(FIGHT_HEAD_ICON) {
        Some((id(source)?, MorphKind::HeadIcon { legacy: false }))
    } else if let Some(source) = path.strip_prefix(LEGACY_HEAD_ICON) {
        Some((id(source)?, MorphKind::HeadIcon { legacy: true }))
    } else {
        let source = id(path.strip_prefix(POSTER)?.strip_prefix("petskinbackground")?)?;
        Some((source, MorphKind::Poster))
    }
}

fn asset_name(id: u32, kind: MorphKind) -> String {
    match kind {
        MorphKind::Backpack { index } => format!("{BACKPACK}petmovie{id}/petmovie{id}_{index}"),
        MorphKind::Battle { face, part } => format!("{BATTLE}pet{id}/pet{id}_{face}_{part}"),
        MorphKind::HeadIcon { legacy: false } => format!("{FIGHT_HEAD_ICON}{id}"),
        MorphKind::HeadIcon { legacy: true } => format!("{LEGACY_HEAD_ICON}{id}"),
        MorphKind::Poster => format!("{POSTER}petskinbackground{id}"),
    }
}

impl Selection {
    pub(super) async fn replace(
        self, original_url: &Url, cache: &AoqiCache, catalog: &Catalog,
        anonymous_client: Option<&reqwest::Client>,
    ) -> Option<Box<dyn SuccessResponse>> {
        let name = asset_name(self.target, self.kind);
        let version = catalog.read().ok().and_then(|catalog| catalog.target_version(&name));
        let result = match &version {
            None => Err(io::Error::new(io::ErrorKind::NotFound, "Target resource version missing").into()),
            Some(version) => self.load(original_url, cache, anonymous_client, &name, version).await,
        };
        match result {
            Ok((body, size, phase)) => {
                self.audit(version.as_deref().unwrap_or_default(), phase, "", size);
                Some(Box::new(Response {
                    url: original_url.to_string(), response_body: body,
                    text_encoding: None, status: 200, redirected: false,
                }))
            }
            Err(error) => {
                self.audit(version.as_deref().unwrap_or_default(), "official-fallback", &error.to_string(), 0);
                None
            }
        }
    }

    async fn load(
        self, original_url: &Url, cache: &AoqiCache, anonymous_client: Option<&reqwest::Client>,
        name: &str, version: &str,
    ) -> Result<(ResponseBody, u64, &'static str), FetchError> {
        let part = match self.kind {
            MorphKind::Backpack { index } => format!("backpack-{index}"),
            MorphKind::Battle { face, part } => format!("battle-{face}-{part}"),
            MorphKind::HeadIcon { legacy: false } => "head-icon".to_owned(),
            MorphKind::HeadIcon { legacy: true } => "head-icon-legacy".to_owned(),
            MorphKind::Poster => "poster".to_owned(),
        };
        let key = AdaptedKey::new(self.source, self.target, &part, version)
            .ok_or_else(|| io::Error::other("Invalid adapted cache key"))?;
        let owned_cache = cache.clone();
        let (key, hit) = spawn_tokio(async move {
            tokio::task::spawn_blocking(move || {
                let hit = owned_cache.open_adapted(&key)?;
                Ok::<_, io::Error>((key, hit))
            }).await
        }).await??;
        if let Some((file, length)) = hit {
            return Ok((ResponseBody::disk(file, length), length, "adapted-cache"));
        }

        let mut url = original_url.clone();
        url.set_path(&format!("/play/{name}~{version}.swf"));
        url.set_query(None);
        let target = target_bytes(cache, anonymous_client, url).await?;
        let adapted = aola_morph_transform::transform(target, self.source, self.target, self.kind).await?;
        let owned_cache = cache.clone();
        let adapted = spawn_tokio(async move {
            tokio::task::spawn_blocking(move || {
                let stored = owned_cache.store_adapted(&key, &adapted)?;
                if !stored {
                    tracing::debug!(target: "aola_morph_http", "Adapted cache publication skipped");
                }
                Ok::<_, io::Error>(adapted)
            }).await
        }).await??;
        let length = adapted.len() as u64;
        Ok((ResponseBody::File(Ok(adapted)), length, "transformed"))
    }

    fn audit(self, version: &str, result: &str, reason: &str, bytes: u64) {
        let (kind, face, part) = match self.kind {
            MorphKind::Backpack { index } => ("backpack", 0, index),
            MorphKind::Battle { face, part } => ("battle", face, part),
            MorphKind::HeadIcon { legacy: false } => ("head-icon", 0, 0),
            MorphKind::HeadIcon { legacy: true } => ("head-icon-legacy", 0, 0),
            MorphKind::Poster => ("poster", 0, 0),
        };
        tracing::info!(target: "aola_morph_http", source = u64::from(self.source),
            target = u64::from(self.target), kind, face = u64::from(face), part = u64::from(part),
            version, result, reason, bytes);
    }
}

async fn target_bytes(
    cache: &AoqiCache, anonymous_client: Option<&reqwest::Client>, url: Url,
) -> Result<Vec<u8>, FetchError> {
    let owned_cache = cache.clone();
    let cached_url = url.clone();
    let hit = spawn_tokio(async move {
        tokio::task::spawn_blocking(move || owned_cache.lookup(&cached_url)).await
    }).await??;
    if let Some(hit) = hit {
        let response_url = Url::parse(&hit.response_url)?;
        if response_url.path() == url.path() {
            return Ok(hit.bytes);
        }
    }
    let client = anonymous_client.cloned().ok_or_else(|| io::Error::other("Network unavailable"))?;
    let cache = cache.clone();
    spawn_tokio(async move {
        let mut response = client.get(url.clone()).timeout(Duration::from_secs(90)).send().await?;
        let status = response.status();
        if status != reqwest::StatusCode::OK {
            return Err(io::Error::other(format!("Target HTTP {status}")).into());
        }
        if response.url().path() != url.path() {
            return Err(io::Error::other("Target response has unexpected resource identity").into());
        }
        let policy = AoqiCache::response_policy(&url, response.url(), status.as_u16(), response.headers())
            .ok_or_else(|| io::Error::other("Target response is not a cacheable public resource"))?;
        let size = response.content_length().unwrap_or(0);
        if size > MAX_TARGET_BYTES as u64 { return Err(io::Error::other("Target body exceeds size limit").into()); }
        let mut bytes = Vec::with_capacity(size as usize);
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > MAX_TARGET_BYTES {
                return Err(io::Error::other("Target body exceeds size limit").into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(tokio::task::spawn_blocking(move || {
            cache.store(&url, &policy, &bytes)?;
            Ok::<_, io::Error>(bytes)
        }).await??)
    }).await
}

pub(super) fn is_catalog_request(request: &Request, url: &Url) -> bool {
    catalog_source(url).is_some() && official(url) && url.query().is_none()
        && request.method() == NavigationMethod::Get
}

pub(super) fn capture_catalog(
    catalog: Catalog, url: Url, response: Box<dyn SuccessResponse>,
) -> Box<dyn SuccessResponse> {
    if response.status() == 200 && let Some(source) = catalog_source(&url)
        && Url::parse(&response.url()).ok().is_some_and(|final_url| {
            official(&final_url) && (final_url == url
                || source == CatalogSource::Startup && final_url.path() == "/play/start.xml" && final_url.query().is_none())
        }) {
        fetch::filter_movie(response, CatalogResponse { catalog, url, source })
    } else {
        response
    }
}

struct CatalogResponse { catalog: Catalog, url: Url, source: CatalogSource }

impl MovieBodyFilter for CatalogResponse {
    async fn complete(self, bytes: Vec<u8>) -> Vec<u8> {
        let catalog = self.catalog;
        let url = self.url;
        let source = self.source;
        let bytes = Arc::new(bytes);
        let worker_bytes = bytes.clone();
        let parsed = spawn_tokio(async move {
            tokio::task::spawn_blocking(move || match source {
                CatalogSource::Base => decode_catalog(&worker_bytes),
                CatalogSource::Startup => parse_versions(&worker_bytes, source),
            }).await
        }).await.map_err(FetchError::from).and_then(|parsed| parsed);
        match parsed {
            Ok(entries) => {
                tracing::info!(target: "aola_morph_http", result = "catalog-ready", catalog_url = url.as_str(), entries = entries.len() as u64);
                if let Ok(mut slot) = catalog.write() {
                    match source {
                        CatalogSource::Base => slot.base = Some(entries),
                        CatalogSource::Startup => slot.startup = entries,
                    }
                }
            }
            Err(error) => tracing::warn!(target: "aola_morph_http", result = "catalog-failed", catalog_url = url.as_str(), reason = error.to_string().as_str()),
        }
        Arc::into_inner(bytes).expect("catalog worker released response bytes")
    }

    fn fallback(&self, reason: &'static str) {
        tracing::warn!(target: "aola_morph_http", result = "catalog-failed", catalog_url = self.url.as_str(), reason);
    }
}

fn decode_catalog(bytes: &[u8]) -> Result<Versions, FetchError> {
    let len = bytes.get(4..8).map(|len| u32::from_le_bytes(len.try_into().unwrap()));
    if !len.is_some_and(|len| (8..=MAX_CATALOG_BYTES).contains(&len)) {
        return Err(io::Error::other("Invalid version catalog SWF length").into());
    }
    let buffer = swf::decompress_swf(bytes)?;
    let movie = swf::parse_swf(&buffer)?;
    let id = movie.tags.iter().find_map(|tag| match tag {
        Tag::SymbolClass(classes) => classes.iter().find(|binding| binding.class_name.as_bytes() == VERSION_CLASS).map(|binding| binding.id),
        _ => None,
    }).ok_or_else(|| io::Error::other("Version catalog symbol absent"))?;
    let xml = movie.tags.iter().find_map(|tag| match tag {
        Tag::DefineBinaryData(data) if data.id == id => Some(data.data),
        _ => None,
    }).ok_or_else(|| io::Error::other("Version catalog XML absent"))?;
    parse_versions(xml, CatalogSource::Base)
}

fn parse_versions(xml: &[u8], source: CatalogSource) -> Result<Versions, FetchError> {
    if xml.len() > MAX_CATALOG_BYTES as usize { return Err(io::Error::other("Version XML exceeds size limit").into()); }
    let section = match source { CatalogSource::Base => b"version".as_slice(), CatalogSource::Startup => b"u".as_slice() };
    let mut reader = Reader::from_reader(xml);
    let mut in_versions = false;
    let mut entries = Versions::new();
    loop {
        match reader.read_event()? {
            Event::Start(tag) if tag.name().as_ref() == section => in_versions = true,
            Event::Empty(tag) if tag.name().as_ref() == section => return Ok(entries),
            Event::Empty(tag) | Event::Start(tag) if in_versions && tag.name().as_ref() == b"f" => {
                let name = tag.try_get_attribute("n")?;
                let version = tag.try_get_attribute("v")?;
                if let (Some(name), Some(version)) = (name, version) {
                    let name = std::str::from_utf8(&name.value)?;
                    let version = std::str::from_utf8(&version.value)?;
                    if parse_asset(name).is_some() {
                        let regional = source == CatalogSource::Startup
                            && tag.try_get_attribute("rn")?.is_some_and(|name| !name.value.is_empty());
                        let version = (!regional && numeric_version(version)).then(|| version.to_owned());
                        entries.insert(name.to_owned(), version);
                    }
                }
            }
            Event::End(tag) if tag.name().as_ref() == section => return Ok(entries),
            Event::Eof if !in_versions && source == CatalogSource::Startup => return Ok(entries),
            Event::Eof => return Err(io::Error::other("Incomplete version catalog XML").into()),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cold_target_fetch_is_anonymous_and_cached() {
        use crate::backends::navigator::tests::new_test_backend;
        use reqwest::cookie::CookieStore;
        use ruffle_core::swf::Header;
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
            time::timeout,
        };

        let target = Url::parse("http://aola.100bt.com/play/fightassets/bitmap/pet12345/pet12345_2_1~72.swf").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let cache = AoqiCache::new(temporary.path().to_owned()).unwrap();
        let mut backend = new_test_backend(false);
        backend.proxy = Some(reqwest::Proxy::all(format!("http://{address}")).unwrap());
        let backend = backend.with_aola_cache(cache.root().to_owned()).unwrap();
        let login_cookie = "session=synthetic-morph-login";
        backend.cookie_store.add_cookie_str(
            &format!("{login_cookie}; Domain=.100bt.com; Path=/"),
            &target,
        );
        assert_eq!(backend.cookie_store.cookies(&target).unwrap(), login_cookie);
        assert!(cache.lookup(&target).unwrap().is_none());

        let mut bytes = Vec::new();
        swf::write_swf(&Header::default_with_swf_version(18), &[], &mut bytes).unwrap();
        let reply = bytes.clone();
        let expected_request = format!("GET {target} HTTP/1.1");
        let proxy = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                assert!(request.len() < 8192);
                request.push(connection.read_u8().await.unwrap());
            }
            let request = std::str::from_utf8(&request).unwrap();
            assert_eq!(request.lines().next(), Some(expected_request.as_str()));
            assert!(!request.lines().any(|line| {
                line.split_once(':').is_some_and(|(name, _)| name.eq_ignore_ascii_case("cookie"))
            }));
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nCache-Control: private, no-cache\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            connection.write_all(headers.as_bytes()).await.unwrap();
            connection.write_all(&reply).await.unwrap();
        });

        let response = timeout(
            Duration::from_secs(5),
            target_bytes(&cache, backend.morph_client.as_ref(), target.clone()),
        ).await.unwrap().unwrap();
        proxy.await.unwrap();
        assert_eq!(response, bytes);
        assert_eq!(cache.lookup(&target).unwrap().unwrap().bytes, bytes);
    }

    #[test]
    fn only_exact_official_morph_paths_match() {
        let url = |path: &str| Url::parse(&format!("https://{OFFICIAL_HOST}/play/{path}")).unwrap();
        assert_eq!(parse_versioned_asset(&url("peticon/movie/type0/petmovie5336/petmovie5336_3~1210.swf")),
            Some((5336, MorphKind::Backpack { index: 3 })));
        assert_eq!(parse_versioned_asset(&url("fightassets/bitmap/pet5336/pet5336_2_0~1210.swf")),
            Some((5336, MorphKind::Battle { face: 2, part: 0 })));
        assert_eq!(parse_versioned_asset(&url("peticon/petfighticon/peticon5336~1210.swf")),
            Some((5336, MorphKind::HeadIcon { legacy: false })));
        assert_eq!(parse_versioned_asset(&url("peticon/peticon4167~1210.swf")),
            Some((4167, MorphKind::HeadIcon { legacy: true })));
        assert_eq!(parse_versioned_asset(&url("pet/petskin/background/petskinbackground1029~12.swf")),
            Some((1029, MorphKind::Poster)));
        for path in [
            "peticon/movie/type0/petmovie1/petmovie2_0~1210.swf",
            "peticon/movie/type1/petmovie1/petmovie1_0~1210.swf",
            "fightassets/bitmap/pet5336/pet5336_3_0~1210.swf",
            "fightassets/bitmap/pet5336/pet5336_1_3~1210.swf",
            "peticon/petfighticon/peticon5336_0~1210.swf",
            "peticon/peticon4167.swf",
            "peticon/peticon04167~1210.swf",
            "pet/petskin/background/petskinbackground1029~bad.swf",
        ] { assert!(parse_versioned_asset(&url(path)).is_none(), "{path}"); }
        let other = Url::parse("https://local.example/play/fightassets/bitmap/pet53/pet53_2_0~1210.swf").unwrap();
        assert!(parse_versioned_asset(&other).is_none());
    }

    #[test]
    fn uses_native_default_and_startup_overrides_only_after_base_catalog() {
        let target = "fightassets/bitmap/pet12345/pet12345_2_1";
        let unlisted = "fightassets/bitmap/pet12345/pet12345_2_2";
        let base = format!(r#"<version><f n="{target}" v="72" /></version>"#);
        let startup = format!(r#"<r><v>123</v><u><f n="{target}" v="83" /></u><m><f n="{target}" v="94" /></m></r>"#);
        let mut catalog = CatalogContents::default();
        assert!(catalog.target_version(target).is_none());
        catalog.base = Some(parse_versions(base.as_bytes(), CatalogSource::Base).unwrap());
        assert_eq!(catalog.target_version(target).as_deref(), Some("72"));
        assert_eq!(catalog.target_version(unlisted).as_deref(), Some("1210"));
        catalog.startup = parse_versions(startup.as_bytes(), CatalogSource::Startup).unwrap();
        assert_eq!(catalog.target_version(target).as_deref(), Some("83"));
        let regional = format!(r#"<r><u><f n="{target}" v="83" rn="variant" z="1;2" /></u></r>"#);
        catalog.startup = parse_versions(regional.as_bytes(), CatalogSource::Startup).unwrap();
        assert!(catalog.target_version(target).is_none());
    }
}
