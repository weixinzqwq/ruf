mod fetch;
mod aola_pet_patch;
mod aola_lifecycle;
mod aola_swf_capture;
mod aola_skin_recipe;
mod aola_display_patch;
mod aola_game_ui;
mod aola_backpack_edits;
mod aola_battle_edits;
mod aola_notice_edits;

use crate::backends::navigator::fetch::{Response, ResponseBody};
use crate::backends::aoqi_cache::{AoqiCache, CachingResponse};
use crate::content::PlayingContent;
use async_channel::{Receiver, Sender, TryRecvError};
use async_io::Timer;
use futures_lite::FutureExt;
use reqwest::{Proxy, cookie, header};
use reqwest::cookie::CookieStore;
use ruffle_core::backend::navigator::{
    ErrorResponse, NavigationMethod, NavigatorBackend, OwnedFuture, Request, SocketMode,
    SuccessResponse, async_return, create_fetch_error, get_encoding,
};
use ruffle_core::indexmap::IndexMap;
use ruffle_core::loader::Error;
use ruffle_core::socket::{ConnectionState, SocketAction, SocketHandle};
use ruffle_core::socket_tap::{self, SocketDirection};
use std::collections::HashSet;
use std::fs::File;
use std::future::Future;
use std::io;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::warn;
use url::{ParseError, Url};

pub trait NavigatorInterface: Clone + Send + 'static {
    fn navigate_to_website(&self, url: Url);

    fn open_file(&self, path: &Path) -> impl std::future::Future<Output = io::Result<File>> + Send;

    fn confirm_socket(
        &self,
        host: &str,
        port: u16,
    ) -> impl std::future::Future<Output = bool> + Send;
}

pub trait FutureSpawner<Err> {
    fn spawn(&self, future: OwnedFuture<(), Err>);
}

#[derive(Debug, PartialEq, Eq)]
pub enum AolaSwfMode<'a> {
    Official,
    Legacy { optimized: bool },
    Lifecycle { mode: &'a str, conflict: bool },
}

impl<'a> AolaSwfMode<'a> {
    pub fn select(lifecycle: &'a str, legacy: &str) -> Self {
        let old_mode = matches!(legacy, "baseline" | "optimized");
        match lifecycle {
            "" if old_mode => Self::Legacy { optimized: legacy == "optimized" },
            "" | "official" => Self::Official,
            mode => Self::Lifecycle { mode, conflict: old_mode },
        }
    }
}

/// Implementation of `NavigatorBackend` for non-web environments that can call
/// out to a web browser.
pub struct ExternalNavigatorBackend<F: FutureSpawner<Error>, I: NavigatorInterface> {
    /// Sink for tasks sent to us through `spawn_future`.
    future_spawner: F,

    /// The url to use for all relative fetches.
    base_url: Url,

    // Client to use for network requests
    client: Option<Rc<reqwest::Client>>,
    cookie_store: Arc<cookie::Jar>,

    socket_allowed: HashSet<String>,

    socket_mode: SocketMode,

    upgrade_to_https: bool,

    content: Rc<PlayingContent>,

    interface: I,

    aoqi_cache: Option<AoqiCache>,
    aola_pet_patch: Option<aola_pet_patch::PetPatch>,
    aola_lifecycle: Option<aola_lifecycle::Trial>,
    display_patches: Vec<Arc<aola_display_patch::DisplayPatch>>,
    aola_game_ui: Option<PathBuf>,
}

impl<F: FutureSpawner<Error>, I: NavigatorInterface> ExternalNavigatorBackend<F, I> {
    /// Construct a navigator backend with fetch and async capability.
    #[expect(clippy::too_many_arguments)]
    pub fn new(
        mut base_url: Url,
        referer: Option<Url>,
        cookie: Option<String>,
        future_spawner: F,
        proxy: Option<Url>,
        upgrade_to_https: bool,
        socket_allowed: HashSet<String>,
        socket_mode: SocketMode,
        content: Rc<PlayingContent>,
        interface: I,
    ) -> Self {
        let cookie_store = Arc::new(cookie::Jar::default());
        let mut builder = reqwest::ClientBuilder::new()
            .cookie_provider(cookie_store.clone())
            .user_agent(concat!(
                "Ruffle/",
                env!("CARGO_PKG_VERSION"),
                " (https://ruffle.rs)"
            ));

        if let Some(referer) = referer {
            let mut headers = header::HeaderMap::new();
            headers.insert(header::REFERER, referer.to_string().parse().unwrap());
            builder = builder.default_headers(headers);
        }

        if let Some(cookie) = cookie {
            cookie_store.add_cookie_str(&cookie, &base_url);
        }

        if let Some(proxy) = proxy {
            match Proxy::all(proxy.clone()) {
                Ok(proxy) => {
                    builder = builder.proxy(proxy);
                }
                Err(e) => {
                    tracing::error!("Couldn't configure proxy {proxy}: {e}")
                }
            }
        }

        let client = builder.build().ok().map(Rc::new);

        // Force replace the last segment with empty. //

        if let Ok(mut base_url) = base_url.path_segments_mut() {
            base_url.pop().pop_if_empty().push("");
        }

        Self {
            future_spawner,
            client,
            cookie_store,
            base_url,
            upgrade_to_https,
            socket_allowed,
            socket_mode,
            content,
            interface,
            aoqi_cache: None,
            aola_pet_patch: None,
            aola_lifecycle: None,
            display_patches: Vec::new(),
            aola_game_ui: None,
        }
    }

    /// Opt-in cache for the reconstructed Aoqi host; ordinary Ruffle navigation
    /// remains unchanged. This must use the destination persisted by migration.
    pub fn with_aola_pet_patch(mut self, root: &Path, optimized: bool) -> io::Result<Self> {
        self.aola_pet_patch = Some(aola_pet_patch::PetPatch::load(root, optimized)?);
        Ok(self)
    }

    pub fn with_aoqi_cache(mut self, root: PathBuf) -> io::Result<Self> {
        self.aoqi_cache = Some(AoqiCache::new(root)?);
        Ok(self)
    }

    pub fn with_aola_cache(mut self, root: PathBuf) -> io::Result<Self> {
        self.aoqi_cache = Some(AoqiCache::for_aola(root)?);
        Ok(self)
    }

    pub fn with_aola_notice(mut self, bundle: Option<&Path>) -> Self {
        use aola_display_patch::{DisplayPatch, NOTICE};
        if let Some(patch) = bundle.map(|root| DisplayPatch::new(&NOTICE, root))
            .filter(|p| p.candidate().is_ok()) {
            NOTICE.audit("config", "ready");
            self.display_patches.push(Arc::new(patch));
        } else {
            NOTICE.audit("config", "invalid-reference");
        }
        self
    }

    pub fn with_aola_battle(mut self, bundle: Option<&Path>) -> Self {
        use aola_display_patch::{BATTLE, DisplayPatch};
        if let Some(root) = bundle {
            self.display_patches.push(Arc::new(DisplayPatch::new(&BATTLE, root)));
            BATTLE.audit("config", "enabled");
        } else {
            BATTLE.audit("config", "invalid-reference");
        }
        self
    }

    pub fn with_aola_game_ui(mut self, bundle: PathBuf) -> Self {
        self.aola_game_ui = Some(bundle);
        self
    }

    pub fn with_aola_backpack(mut self, bundle: Option<&Path>) -> Self {
        use aola_display_patch::{ARCHIVED_BACKPACK, DisplayPatch, SUPPLIED_BACKPACK};
        for recipe in [&SUPPLIED_BACKPACK, &ARCHIVED_BACKPACK] {
            if let Some(root) = bundle {
                self.display_patches.push(Arc::new(DisplayPatch::new(recipe, root)));
                recipe.audit("config", "enabled");
            } else {
                recipe.audit("config", "invalid-reference");
            }
        }
        self
    }

    pub fn with_aola_lifecycle(mut self, mode: &str, bundle: &Path, capture: &Path, legacy: bool) -> Self {
        let label = match mode {
            "original" | "roundtrip" | "capture" | "lifecycle" => mode,
            _ => "unknown",
        };
        self.aola_lifecycle = None;
        let had_legacy = self.aola_pet_patch.take().is_some();
        if legacy || had_legacy {
            aola_lifecycle::audit(label, "", "", "config", "conflicting-modes", 0);
        } else {
            match aola_lifecycle::Trial::load(mode, bundle, capture) {
                Ok(trial) => {
                    aola_lifecycle::audit(label, "", "", "config", "ready", 0);
                    self.aola_lifecycle = Some(trial);
                }
                Err(_) => aola_lifecycle::audit(label, "", "", "config", "invalid-configuration", 0),
            }
        }
        self
    }
}

impl<F: FutureSpawner<Error> + 'static, I: NavigatorInterface> NavigatorBackend
    for ExternalNavigatorBackend<F, I>
{
    fn navigate_to_url(
        &self,
        url: &str,
        _target: &str,
        vars_method: Option<(NavigationMethod, IndexMap<String, String>)>,
    ) {
        //TODO: Should we return a result for failed opens? Does Flash care?

        //NOTE: Flash desktop players / projectors ignore the window parameter,
        //      unless it's a `_layer`, and we shouldn't handle that anyway.
        let mut parsed_url = match self.resolve_url(url) {
            Ok(parsed_url) => parsed_url,
            Err(e) => {
                tracing::error!(
                    "Could not parse URL because of {}, the corrupt URL was: {}",
                    e,
                    url
                );
                return;
            }
        };

        let modified_url = match vars_method {
            Some((_, query_pairs)) if !query_pairs.is_empty() => {
                {
                    //lifetime limiter because we don't have NLL yet
                    let mut modifier = parsed_url.query_pairs_mut();

                    for (k, v) in query_pairs.iter() {
                        modifier.append_pair(k, v);
                    }
                }

                parsed_url
            }
            _ => parsed_url,
        };

        if modified_url.scheme() == "javascript" {
            tracing::warn!(
                "SWF tried to run a script on desktop, but javascript calls are not allowed"
            );
            return;
        }

        self.interface.navigate_to_website(modified_url);
    }

    fn fetch(&self, request: Request) -> OwnedFuture<Box<dyn SuccessResponse>, ErrorResponse> {
        // TODO: honor sandbox type (local-with-filesystem, local-with-network, remote, ...)
        let mut processed_url = match self.resolve_url(request.url()) {
            Ok(url) => url,
            Err(e) => {
                return async_return(Err(create_fetch_error(request.url(), e)));
            }
        };

        ruffle_core::mypro_observer::observe("resource", processed_url.as_str(), "");
        let game_ui = self.aola_game_ui.as_ref().and_then(|root| {
            aola_game_ui::Asset::requested(&request, &processed_url)
                .map(|asset| aola_game_ui::Selection {root:root.clone(),url:processed_url.to_string(),asset})
        });
        if let Some(aola_game_ui::Selection { root, url, asset:aola_game_ui::Asset::Skin(id) }) = game_ui.as_ref() {
            return aola_game_ui::leaf(root,*id,url.clone());
        }
        if let Some(patch) = &self.aola_pet_patch {
            if aola_pet_patch::PetPatch::accepts(&request, &processed_url) {
                let response: Box<dyn SuccessResponse> = Box::new(Response {
                    url: processed_url.to_string(), response_body: ResponseBody::File(Ok(patch.bytes())),
                    text_encoding: None, status: 200, redirected: false,
                });
                return async_return(Ok(response));
            }
        }

        let lifecycle = self.aola_lifecycle.as_ref().and_then(|trial| {
            trial.request_resource(&request, &processed_url).map(|(resource, version)| (trial.clone(), resource, version.to_owned()))
        });
        let display_patch = self.display_patches.iter()
            .find(|p| p.recipe.accepts(&request, &processed_url)).cloned();
        let client = self.client.clone();
        let cookie_store = self.cookie_store.clone();
        ruffle_core::aola_panel_diagnostics::resource(processed_url.as_str(), "resource-request", 0);
        let cache = self.aoqi_cache.clone().filter(|_| {
            AoqiCache::accepts_request(&request, &processed_url)
                && self.cookie_store.cookies(&processed_url).is_none()
        });

        let error_trial = lifecycle.clone();
        let fetch: OwnedFuture<Box<dyn SuccessResponse>, ErrorResponse> = match processed_url.scheme() {
            "aoqi" => {
                // Explicit local bundle scheme; remote HTTP requests are never redirected.
                let asset = if processed_url.host_str().is_none() {
                    aoqi_assets::get(processed_url.path().trim_start_matches('/'))
                } else {
                    None
                };
                match asset {
                    Some(asset) => {
                        let response: Box<dyn SuccessResponse> = Box::new(Response {
                            url: processed_url.to_string(),
                            response_body: ResponseBody::File(Ok(asset.bytes.to_vec())),
                            text_encoding: None,
                            status: 200,
                            redirected: false,
                        });
                        async_return(Ok(response))
                    }
                    None => async_return(Err(ErrorResponse {
                        url: processed_url.to_string(),
                        error: Error::FetchError("Unknown embedded resource".to_string()),
                    })),
                }
            }
            "file" => {
                let content = self.content.clone();
                let interface = self.interface.clone();
                Box::pin(async move {
                    // We send the original url (including query parameters)
                    // back to ruffle_core in the `Response`
                    let response_url = processed_url.clone();
                    // Flash supports query parameters with local urls.
                    // SwfMovie takes care of exposing those to ActionScript -
                    // when we actually load a filesystem url, strip them out.
                    processed_url.set_query(None);

                    let contents = content.get_local_file(&processed_url, interface).await;

                    let response: Box<dyn SuccessResponse> = Box::new(Response {
                        url: response_url.to_string(),
                        response_body: ResponseBody::File(contents),
                        text_encoding: None,
                        status: 0,
                        redirected: false,
                    });

                    Ok(response)
                })
            }
            _ => Box::pin(async move {
                if let Some(cache) = &cache {
                    let worker_cache = cache.clone();
                    let worker_url = processed_url.clone();
                    match ruffle_core::aoqi_work::blocking(move || worker_cache.lookup(&worker_url)).await
                        .unwrap_or_else(|error| Err(std::io::Error::other(error))) {
                        Ok(Some(hit)) if Url::parse(&hit.response_url).ok().is_some_and(|url| cookie_store.cookies(&url).is_none()) => {
                            ruffle_core::aola_panel_diagnostics::resource(processed_url.as_str(), "resource-cache", hit.bytes.len() as u64);
                            let text_encoding = hit.content_type.as_deref().and_then(get_encoding);
                            let redirected = hit.response_url != processed_url.as_str();
                            let response: Box<dyn SuccessResponse> = Box::new(Response {
                                url: hit.response_url, response_body: ResponseBody::File(Ok(hit.bytes)),
                                text_encoding, status: 200, redirected,
                            });
                            return Ok(aola_display_patch::wrap(display_patch, "cache", aola_lifecycle::wrap(lifecycle, "cache", response)));
                        }
                        Ok(_) => {}
                        Err(error) => tracing::warn!("Static resource cache read failed: {error}"),
                    }
                }
                let client = client.ok_or_else(|| ErrorResponse {
                    url: processed_url.to_string(),
                    error: Error::FetchError("Network unavailable".to_string()),
                })?;

                let mut request_builder = match request.method() {
                    NavigationMethod::Get => client.get(processed_url.clone()),
                    NavigationMethod::Post => client.post(processed_url.clone()),
                };
                let (body_data, mime) = request.body().clone().unwrap_or_default();
                for (name, val) in request.headers().iter() {
                    request_builder = request_builder.header(name, val);
                }
                request_builder = request_builder.header("Content-Type", &mime);

                request_builder = request_builder.body(body_data);

                let response = spawn_tokio(request_builder.send()).await.map_err(|e| {
                    if let Some(patch) = &display_patch { patch.recipe.audit("network", "fetch-failed"); }
                    ruffle_core::aola_panel_diagnostics::resource(processed_url.as_str(), "resource-network-error", 0);
                    let inner = if e.is_connect() {
                        Error::InvalidDomain(processed_url.to_string())
                    } else {
                        Error::FetchError(e.to_string())
                    };
                    ErrorResponse {
                        url: processed_url.to_string(),
                        error: inner,
                    }
                })?;

                let url = response.url().to_string();
                let text_encoding = response
                    .headers()
                    .get("Content-Type")
                    .and_then(|content_type| content_type.to_str().ok())
                    .and_then(get_encoding);
                let status = response.status().as_u16();
                ruffle_core::aola_panel_diagnostics::resource(processed_url.as_str(), "resource-status", status as u64);
                let redirected = *response.url() != processed_url;
                let cache_policy = cache.as_ref().filter(|_| cookie_store.cookies(response.url()).is_none())
                    .and_then(|_| AoqiCache::response_policy(&processed_url, response.url(), status, response.headers()));
                if !response.status().is_success() {
                    if let Some(patch) = &display_patch { patch.recipe.audit("network", "http-error"); }
                    let error = Error::HttpNotOk(
                        format!("Got {}", response.status()),
                        status,
                        redirected,
                        response.content_length().unwrap_or_default(),
                    );
                    return Err(ErrorResponse { url, error });
                }

                let response: Box<dyn SuccessResponse> = Box::new(Response {
                    url,
                    response_body: ResponseBody::Network(Arc::new(Mutex::new(Some(response)))),
                    text_encoding,
                    status,
                    redirected,
                });
                let response = if let (Some(cache), Some(policy)) = (cache, cache_policy) {
                    Box::new(CachingResponse::new(response, cache, processed_url, policy)) as Box<dyn SuccessResponse>
                } else { response };
                Ok(aola_display_patch::wrap(display_patch, "network", aola_lifecycle::wrap(lifecycle, "network", response)))
            }),
        };
        if let Some((trial, resource, version)) = error_trial {
            return Box::pin(async move {
                fetch.await.inspect_err(|_| trial.reject(resource, &version, "fetch", "fetch-failed"))
            });
        }
        if let Some(selection) = game_ui {
            return Box::pin(async move { Ok(selection.wrap(fetch.await?)) });
        }
        fetch
    }

    fn resolve_url(&self, url: &str) -> Result<Url, ParseError> {
        match self.base_url.join(url) {
            Ok(url) => Ok(self.pre_process_url(url)),
            Err(error) => Err(error),
        }
    }

    fn spawn_future(&mut self, future: OwnedFuture<(), Error>) {
        self.future_spawner.spawn(future);
    }

    fn pre_process_url(&self, mut url: Url) -> Url {
        if self.upgrade_to_https && url.scheme() == "http" && url.set_scheme("https").is_err() {
            tracing::error!("Url::set_scheme failed on: {}", url);
        }
        url
    }

    fn connect_socket(
        &mut self,
        host: String,
        port: u16,
        timeout: Duration,
        handle: SocketHandle,
        receiver: Receiver<Vec<u8>>,
        sender: Sender<SocketAction>,
    ) {
        /// Tries to send the given action properly handling failures.
        ///
        /// Returns `true` when the action has been sent properly,
        /// `false` when the channel is closed.
        async fn send_action(sender: &Sender<SocketAction>, action: SocketAction) -> bool {
            sender
                .send(action)
                .await
                .inspect_err(|err| tracing::warn!("Failed to send SocketAction: {}", err))
                .is_ok()
        }

        let addr = format!("{host}:{port}");
        let is_allowed = self.socket_allowed.contains(&addr);
        let socket_mode = self.socket_mode;
        let interface = self.interface.clone();
        let tap_session = socket_tap::current_session();

        let future = Box::pin(async move {
            match (is_allowed, socket_mode) {
                (false, SocketMode::Allow) | (true, _) => {} // the process is allowed to continue. just dont do anything.
                (false, SocketMode::Deny) => {
                    // Just fail the connection.
                    let action = SocketAction::Connect(handle, ConnectionState::Failed);
                    let _ = send_action(&sender, action).await;

                    tracing::warn!(
                        "SWF tried to open a socket, but opening a socket is not allowed"
                    );

                    return;
                }
                (false, SocketMode::Ask) => {
                    let attempt_sandbox_connect = interface.confirm_socket(&host, port).await;

                    if !attempt_sandbox_connect {
                        // fail the connection.
                        let action = SocketAction::Connect(handle, ConnectionState::Failed);
                        let _ = send_action(&sender, action).await;
                        return;
                    }
                }
            }

            let host2 = host.clone();

            let timeout = async {
                Timer::after(timeout).await;
                Result::<TcpStream, io::Error>::Err(io::Error::new(ErrorKind::TimedOut, ""))
            };

            let mut stream = match TcpStream::connect((host, port)).or(timeout).await {
                Err(e) if e.kind() == ErrorKind::TimedOut => {
                    warn!("Connection to {}:{} timed out", host2, port);
                    let action = SocketAction::Connect(handle, ConnectionState::TimedOut);
                    let _ = send_action(&sender, action).await;
                    return;
                }
                Ok(stream) => {
                    let action = SocketAction::Connect(handle, ConnectionState::Connected);
                    if !send_action(&sender, action).await {
                        return;
                    }
                    stream
                }
                Err(err) => {
                    warn!("Failed to connect to {}:{}, error: {}", host2, port, err);
                    let action = SocketAction::Connect(handle, ConnectionState::Failed);
                    let _ = send_action(&sender, action).await;
                    return;
                }
            };

            let connection = socket_tap::Connection::connected(tap_session, handle, host2, port);
            let tap = connection.as_ref();
            let sender = sender;
            //NOTE: We clone the sender here as we cant share it between async tasks.
            let sender2 = sender.clone();
            let (mut read, mut write) = stream.split();

            let read = async move {
                let mut offset = 0;
                loop {
                    let mut buffer = [0; 4096];

                    match read.read(&mut buffer).await {
                        Err(e) if e.kind() == ErrorKind::TimedOut => {} // try again later.
                        Err(_) | Ok(0) => {
                            let _ = send_action(&sender, SocketAction::Close(handle)).await;
                            break;
                        }
                        Ok(read) => {
                            let buffer = buffer.into_iter().take(read).collect::<Vec<_>>();
                            if let Some(tap) = tap {
                                tap.data(SocketDirection::Recv, &mut offset, &buffer);
                            }

                            let action = SocketAction::Data(handle, buffer);
                            if !send_action(&sender, action).await {
                                return;
                            }
                        }
                    };
                }
            };

            let write = async move {
                let mut pending_write = vec![];
                let mut offset = 0;

                loop {
                    let close_connection = loop {
                        match receiver.try_recv() {
                            Ok(val) => {
                                pending_write.extend(val);
                            }
                            Err(TryRecvError::Empty) => break false,
                            Err(TryRecvError::Closed) => {
                                //NOTE: Channel sender has been dropped.
                                //      This means we have to close the connection,
                                //      but not here, as we might have a pending write.
                                break true;
                            }
                        }
                    };

                    if !pending_write.is_empty() {
                        match write.write(&pending_write).await {
                            Err(e) if e.kind() == ErrorKind::TimedOut => {} // try again later.
                            Err(_) => {
                                let _ = send_action(&sender2, SocketAction::Close(handle)).await;
                                return;
                            }
                            Ok(written) => {
                                if let Some(tap) = tap {
                                    tap.data(
                                        SocketDirection::Send,
                                        &mut offset,
                                        &pending_write[..written],
                                    );
                                }
                                let _ = pending_write.drain(..written);
                            }
                        }
                    } else if close_connection {
                        return;
                    } else {
                        // Receiver is empty and there's no pending data,
                        // we may block here and wait for new data.
                        match receiver.recv().await {
                            Ok(val) => {
                                pending_write.extend(val);
                            }
                            Err(_) => {
                                // Ignore the error here, it will be
                                // reported again in try_recv.
                            }
                        }
                    }
                }
            };

            //NOTE: If one future exits, this will take the other one down too.
            tokio::select! {
               _ = read => {},
               _ = write => {},
            }

            drop(connection);
            if let Err(e) = stream.shutdown().await {
                tracing::warn!("Failed to shutdown write half of TcpStream: {e}");
            }
        });

        tokio::spawn(future);
    }
}

/// Spawns a new asynchronous task in a tokio runtime, without the current executor needing to belong to tokio
pub async fn spawn_tokio<F>(future: F) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move { sender.send(future.await) });
    tokio::task::unconstrained(receiver)
        .await
        .expect("Oneshot should succeed")
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use ruffle_core::socket::SocketAction::{Close, Connect, Data};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use tokio::net::TcpListener;
    use tokio::task;

    use crate::content::ContentDescriptor;

    use super::*;

    impl NavigatorInterface for () {
        fn navigate_to_website(&self, _url: Url) {}

        async fn open_file(&self, path: &Path) -> io::Result<File> {
            File::open(path)
        }

        async fn confirm_socket(&self, _host: &str, _port: u16) -> bool {
            true
        }
    }

    const TIMEOUT_ZERO: Duration = Duration::ZERO;
    // The timeout has to be large enough to allow "instantaneous" actions
    // and local IO to execute, but small enough to fail tests quickly.
    const TIMEOUT: Duration = Duration::from_secs(1);

    struct TestFutureSpawner;

    impl<E: 'static> FutureSpawner<E> for TestFutureSpawner {
        fn spawn(&self, future: OwnedFuture<(), E>) {
            task::spawn_local(future);
        }
    }

    macro_rules! async_timeout {
        () => {
            async {
                Timer::after(TIMEOUT).await;
                panic!("An action which should complete timed out")
            }
        };
    }

    macro_rules! async_test {
        (
            async fn $test_name:ident() $content:block
        ) => {
            #[tokio::test(flavor = "current_thread")]
            async fn $test_name() {
                task::LocalSet::new().run_until(async move $content).await;
            }
        }
    }

    macro_rules! dummy_handle {
        () => {
            SocketHandle::default()
        };
    }

    macro_rules! assert_next_socket_actions {
        ($receiver:expr;) => {
            // no more actions
        };
        ($receiver:expr; $action:expr, $($more:expr,)*) => {
            assert_eq!($receiver.recv().or(async_timeout!()).await.expect("receive action"), $action);
            assert_next_socket_actions!($receiver; $($more,)*);
        };
    }

    fn new_test_backend(socket_allow: bool) -> ExternalNavigatorBackend<TestFutureSpawner, ()> {
        let url = Url::parse("https://example.com/path/").unwrap();
        ExternalNavigatorBackend::new(
            url.clone(),
            None,
            None,
            TestFutureSpawner,
            None,
            false,
            Default::default(),
            if socket_allow {
                SocketMode::Allow
            } else {
                SocketMode::Deny
            },
            Rc::new(PlayingContent::DirectFile(ContentDescriptor {
                url,
                root_content_path: None,
            })),
            (),
        )
    }

    async fn start_test_server() -> (task::JoinHandle<TcpStream>, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let accept_task = task::spawn_local(async move {
            let (socket, _) = listener.accept().or(async_timeout!()).await.unwrap();
            socket
        });
        (accept_task, addr)
    }

    #[tokio::test]
    async fn aoqi_embedded_resources_work_without_network() {
        let mut backend = new_test_backend(false);
        backend.client = None;
        let response = backend.fetch(Request::get("aoqi:///host.swf?version=test".into())).await;
        let response = match response {
            Ok(response) => response,
            Err(_) => panic!("embedded resource was not served"),
        };
        assert_eq!(response.status(), 200);
        assert_eq!(response.body().await.unwrap(), aoqi_assets::get("host.swf").unwrap().bytes);
        for url in ["aoqi:///missing.swf", "aoqi://remote/host.swf"] {
            assert!(backend.fetch(Request::get(url.into())).await.is_err());
        }
    }

    #[tokio::test]
    #[ignore = "requires the archived notice SWF and the independently rebuilt candidate"]
    async fn manual_notice_navigator_fetch_uses_real_reference_and_preserves_cached_source() {
        check_display_patch("AOLA_TEST_NOTICE_FILES", "messagesendservice-notice.swf",
            "messagesend/messagesendservice", "2026062579184482", false).await;
    }

    #[tokio::test]
    #[ignore = "requires the archived battle SWF and the independently rebuilt candidate"]
    async fn manual_battle_navigator_fetch_uses_real_reference_and_preserves_cached_source() {
        check_display_patch("AOLA_TEST_BATTLE_FILES", "petfightnew-skill.swf",
            "pet/petfightnew", "261112849126071", true).await;
    }

    #[tokio::test]
    #[ignore = "requires both backpack references and independently rebuilt candidates"]
    async fn manual_backpack_navigator_fetch_checks_both_versions_and_preserves_cache() {
        let bundle = PathBuf::from(std::env::var_os("AOLA_TEST_BACKPACK_FILES").unwrap());
        let read = |name| std::fs::read(bundle.join(name)).unwrap();
        let supplied = read("supplied.swf");
        let archived = read("archived.swf");
        let supplied_optimized = read("supplied-optimized.swf");
        let archived_optimized = read("archived-optimized.swf");
        let mut changed = supplied.clone();
        *changed.last_mut().unwrap() ^= 1;
        let dir = tempfile::tempdir().unwrap();
        let cache = AoqiCache::new(dir.path().into()).unwrap();
        let cases = [
            (".swf", &supplied, &supplied_optimized),
            ("~123.swf", &supplied, &supplied_optimized),
            ("~2026111953323155.swf", &archived, &archived_optimized),
            ("~2026111953323155.swf?v=1", &supplied, &supplied),
            ("~123.swf?v=1", &archived, &archived),
            (".swf?v=1", &changed, &changed),
        ];
        for (suffix, bytes, _) in cases {
            let url = Url::parse(&format!("https://aola.100bt.com/play/pet/newpetpackage{suffix}")).unwrap();
            let policy = AoqiCache::response_policy(&url, &url, 200, &header::HeaderMap::new()).unwrap();
            assert!(cache.store(&url, &policy, bytes).unwrap());
        }
        for enabled in [false, true, false] {
            let mut backend = new_test_backend(false).with_aoqi_cache(dir.path().into()).unwrap();
            backend.client = None;
            if enabled { backend = backend.with_aola_backpack(Some(&bundle)); }
            for (suffix, bytes, optimized) in cases {
                let url = format!("https://aola.100bt.com/play/pet/newpetpackage{suffix}");
                let response = backend.fetch(Request::get(url.clone())).await.unwrap_or_else(|e| panic!("{}", e.error));
                assert_eq!(response.url().as_ref(), url);
                assert_eq!(response.status(), 200);
                let expected = if enabled { optimized } else { bytes };
                assert!(response.body().await.unwrap().as_slice() == expected.as_slice(), "{suffix}");
            }
        }
    }

    async fn check_display_patch(var: &str, output: &str, path: &str, version: &str, battle: bool) {
        let bundle = PathBuf::from(std::env::var_os(var).expect(var));
        let original = std::fs::read(bundle.join("original.swf")).unwrap();
        let candidate = std::fs::read(bundle.join(output)).unwrap();
        assert_ne!(original, candidate);
        let url = Url::parse(&format!("https://aola.100bt.com/play/{path}~{version}.swf?v=1")).unwrap();
        let old = Url::parse(&url.as_str().replace(version, "20200101")).unwrap();
        let mismatch = Url::parse(&url.as_str().replace("v=1", "v=2")).unwrap();
        let mut changed = original.clone();
        *changed.last_mut().unwrap() ^= 1;

        let dir = tempfile::tempdir().unwrap();
        let cache = AoqiCache::new(dir.path().into()).unwrap();
        for (id, bytes) in [(&url, &original), (&old, &original), (&mismatch, &changed)] {
            let policy = AoqiCache::response_policy(id, id, 200, &header::HeaderMap::new()).unwrap();
            assert!(cache.store(id, &policy, bytes).unwrap());
        }
        for hidden in [false, true, false] {
            let mut backend = new_test_backend(false).with_aoqi_cache(dir.path().into()).unwrap();
            backend.client = None;
            if hidden {
                backend = if battle { backend.with_aola_battle(Some(&bundle)) }
                          else { backend.with_aola_notice(Some(&bundle)) };
            }
            for (id, expected) in [(&url, if hidden { &candidate } else { &original }), (&old, &original), (&mismatch, &changed)] {
                let response = backend.fetch(Request::get(id.to_string())).await.unwrap_or_else(|e| panic!("{}", e.error));
                assert_eq!(response.url().as_ref(), id.as_str());
                assert_eq!(response.status(), 200);
                assert_eq!(response.body().await.unwrap().as_slice(), expected.as_slice());
            }
        }
    }

    async fn serve_swf_fixture(mut connection: TcpStream, url: &str, bytes: &[u8]) {
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            assert!(request.len() < 8192);
            request.push(connection.read_u8().await.unwrap());
        }
        assert!(request.starts_with(format!("GET {url} HTTP/1.1\r\n").as_bytes()));
        let headers = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
        connection.write_all(headers.as_bytes()).await.unwrap();
        connection.write_all(bytes).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires both rebuilt backpack fixtures and a local loopback listener"]
    async fn manual_backpack_local_http_patches_network_and_caches_both_originals() {
        let root = PathBuf::from(std::env::var_os("AOLA_TEST_BACKPACK_FILES").expect("AOLA_TEST_BACKPACK_FILES"));
        let cases = [
            ("http://aola.100bt.com/play/pet/newpetpackage.swf", "supplied"),
            ("http://aola.100bt.com/play/pet/newpetpackage~2026111953323155.swf", "archived"),
        ];
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let fixtures = root.clone();
        let server = tokio::spawn(async move {
            for (url, name) in cases {
                let (connection, _) = listener.accept().await.unwrap();
                let bytes = std::fs::read(fixtures.join(format!("{name}.swf"))).unwrap();
                serve_swf_fixture(connection, url, &bytes).await;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let mut online = new_test_backend(false).with_aola_cache(dir.path().into()).unwrap()
            .with_aola_backpack(Some(&root));
        online.client = Some(Rc::new(reqwest::Client::builder()
            .no_proxy().proxy(Proxy::all(format!("http://{address}")).unwrap())
            .resolve("aola.100bt.com", address)
            .timeout(Duration::from_secs(15)).build().unwrap()));
        let mut offline = new_test_backend(false).with_aola_cache(dir.path().into()).unwrap();
        offline.client = None;
        for (navigator, suffix) in [(&online, "-optimized.swf"), (&offline, ".swf")] {
            for (url, name) in cases {
                let expected = std::fs::read(root.join(format!("{name}{suffix}"))).unwrap();
                let response = navigator.fetch(Request::get(url.into())).await.unwrap_or_else(|e| panic!("{}", e.error));
                assert_eq!(response.status(), 200);
                assert_eq!(response.url().as_ref(), url);
                assert!(response.body().await.unwrap() == expected, "{name}{suffix}");
            }
        }
        server.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "transport smoke test requires permission to connect to the local loopback listener"]
    async fn manual_swf_local_http_delivers_real_roundtrip_through_fetch() {
        let root = PathBuf::from(std::env::var_os("AOLA_TEST_SWF_FILES").expect("AOLA_TEST_SWF_FILES"));
        let original = std::fs::read(root.join("newpetpackage-original.swf")).unwrap();
        let control = std::fs::read(root.join("newpetpackage-roundtrip.swf")).unwrap();
        let util = std::fs::read(root.join("library_util~2026090340574444.swf")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let proxy = format!("http://{address}");
        let received = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let accepted = received.clone();
        let url = "http://aola.100bt.com/play/pet/newpetpackage~2026111953323155.swf";
        let util_url = "http://aola.100bt.com/play/library/util~2026090340574444.swf";
        let served_util = util.clone();
        let server = tokio::spawn(async move {
            for (path, bytes) in [(util_url, served_util), (url, original)] {
                let (connection, _) = listener.accept().await.unwrap();
                accepted.store(true, std::sync::atomic::Ordering::Relaxed);
                serve_swf_fixture(connection, path, &bytes).await;
            }
        });
        let mut backend = new_test_backend(false).with_aola_lifecycle("roundtrip", &root, &root, false);
        backend.client = Some(Rc::new(reqwest::Client::builder().proxy(Proxy::all(proxy).unwrap()).build().unwrap()));
        for (path, expected) in [(util_url, &util), (url, &control)] {
            let response = tokio::time::timeout(Duration::from_secs(10), backend.fetch(Request::get(path.into())))
                .await.expect("loopback fixture should respond").unwrap_or_else(|e| {
                    let probe = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(1));
                    panic!("loopback fixture: {}; accepted={}; transport probe={:?}", e.error,
                        received.load(std::sync::atomic::Ordering::Relaxed), probe.err());
                });
            assert_eq!(&response.body().await.unwrap(), expected);
        }
        server.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires the four SWFs produced by the local experiment packager"]
    async fn aola_lifecycle_navigator_serves_real_candidate_only_after_util_in_same_player() {
        let root = PathBuf::from(std::env::var_os("AOLA_TEST_SWF_FILES").expect("AOLA_TEST_SWF_FILES"));
        let original = std::fs::read(root.join("newpetpackage-original.swf")).unwrap();
        let control = std::fs::read(root.join("newpetpackage-roundtrip.swf")).unwrap();
        let patched = std::fs::read(root.join("newpetpackage-lifecycle.swf")).unwrap();
        let util = std::fs::read(root.join("library_util~2026090340574444.swf")).unwrap();
        let pet_url = Url::parse("https://aola.100bt.com/play/pet/newpetpackage~2026111953323155.swf").unwrap();
        let old_url = Url::parse("https://aola.100bt.com/play/pet/newpetpackage~2026102936286521.swf").unwrap();
        let util_url = Url::parse("https://aola.100bt.com/play/library/util~2026090340574444.swf").unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = AoqiCache::new(cache_dir.path().into()).unwrap();
        // Serve identical current bytes at the obsolete address as well: even an exact
        // byte match cannot authorize the old resource identity.
        for (url, bytes) in [(&pet_url, &original), (&old_url, &original), (&util_url, &util)] {
            let policy = AoqiCache::response_policy(url, url, 200, &header::HeaderMap::new()).unwrap();
            assert!(cache.store(url, &policy, bytes).unwrap());
        }
        let new_player = |mode: &str| {
            let mut backend = new_test_backend(false).with_aoqi_cache(cache_dir.path().into()).unwrap()
                .with_aola_lifecycle(mode, &root, &root, false);
            backend.client = None;
            backend
        };
        async fn read(backend: &ExternalNavigatorBackend<TestFutureSpawner, ()>, url: &Url) -> Vec<u8> {
            backend.fetch(Request::get(url.to_string())).await.ok().unwrap().body().await.unwrap()
        }
        for (mode, output) in [("original", &original), ("roundtrip", &control), ("lifecycle", &patched)] {
            let first = new_player(mode);
            let second = new_player(mode);
            assert_eq!(read(&first, &util_url).await, util);
            assert_eq!(read(&second, &pet_url).await, original);
            assert_eq!(&read(&first, &pet_url).await, output);
            assert_eq!(read(&second, &util_url).await, util);
            assert_eq!(read(&second, &pet_url).await, original);
            let obsolete = new_player(mode);
            assert_eq!(read(&obsolete, &util_url).await, util);
            assert_eq!(read(&obsolete, &old_url).await, original);
            assert_eq!(read(&obsolete, &pet_url).await, original);
        }
    }

    #[tokio::test]
    async fn aoqi_cache_download_capture_and_migrated_offline_hits() {
        let temporary = tempfile::tempdir().unwrap();
        let source = AoqiCache::new(temporary.path().join("source-cache")).unwrap();
        let url = Url::parse("https://aoqi.100bt.com/play/library/common~123.swf").unwrap();
        let bytes = b"FWS\x09\x08\0\0\0".to_vec();
        let policy = AoqiCache::response_policy(&url, &url, 200, &header::HeaderMap::new()).unwrap();
        let response: Box<dyn SuccessResponse> = Box::new(Response {
            url: url.to_string(), response_body: ResponseBody::File(Ok(bytes.clone())),
            text_encoding: None, status: 200, redirected: false,
        });
        let response = Box::new(CachingResponse::new(response, source.clone(), url.clone(), policy));
        assert_eq!(response.body().await.unwrap(), bytes);
        assert_eq!(source.stats().unwrap()["entries"], 1);

        // The migration module copies this exact tree. Exercise the destination,
        // not the original instance, with networking explicitly disabled.
        let destination = AoqiCache::new(temporary.path().join("migrated-cache")).unwrap();
        for entry in std::fs::read_dir(source.root().join("v1")).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), destination.root().join("v1").join(entry.file_name())).unwrap();
        }
        let mut backend = new_test_backend(false).with_aoqi_cache(destination.root().to_owned()).unwrap();
        backend.client = None;
        let response = match backend.fetch(Request::get(url.to_string())).await {
            Ok(response) => response,
            Err(_) => panic!("migrated disk entry was not consumed by navigator"),
        };
        assert_eq!(response.status(), 200);
        assert_eq!(response.body().await.unwrap(), bytes);
        assert!(backend.fetch(Request::get("https://aoqi.100bt.com/play/missing~124.swf".into())).await.is_err());
        backend.cookie_store.add_cookie_str("session=synthetic-test-only", &url);
        assert!(backend.fetch(Request::get(url.to_string())).await.is_err(), "requests carrying implicit cookies must bypass cache");
    }

    #[tokio::test]
    async fn aoqi_cache_does_not_publish_incomplete_or_failed_bodies() {
        let temporary = tempfile::tempdir().unwrap();
        let cache = AoqiCache::new(temporary.path().join("cache")).unwrap();
        let url = Url::parse("https://aoqi.100bt.com/play/image~1.png").unwrap();
        let policy = AoqiCache::response_policy(&url, &url, 200, &header::HeaderMap::new()).unwrap();
        let response: Box<dyn SuccessResponse> = Box::new(Response {
            url:url.to_string(), response_body:ResponseBody::File(Ok(b"\x89PNG\r\n\x1a\n".to_vec())),
            text_encoding:None,status:200,redirected:false,
        });
        let mut response = CachingResponse::new(response,cache.clone(),url.clone(),policy.clone());
        assert!(response.next_chunk().await.unwrap().is_some());
        assert!(cache.lookup(&url).unwrap().is_none());
        assert!(response.next_chunk().await.unwrap().is_none());
        assert!(cache.lookup(&url).unwrap().is_some());
        let failed_url=Url::parse("https://aoqi.100bt.com/play/failed~1.png").unwrap();
        let failed:Box<dyn SuccessResponse>=Box::new(Response {
            url:failed_url.to_string(),response_body:ResponseBody::File(Err(io::Error::other("interrupted"))),
            text_encoding:None,status:200,redirected:false,
        });
        let failed=Box::new(CachingResponse::new(failed,cache.clone(),failed_url.clone(),policy));
        assert!(failed.body().await.is_err());
        assert!(cache.lookup(&failed_url).unwrap().is_none());
    }

    fn connect_test_socket(
        addr: SocketAddr,
        timeout: Duration,
        socket_allow: bool,
    ) -> (Sender<Vec<u8>>, Receiver<SocketAction>) {
        let mut backend = new_test_backend(socket_allow);

        let (write, receiver) = async_channel::unbounded();
        let (sender, read) = async_channel::unbounded();

        backend.connect_socket(
            addr.ip().to_string(),
            addr.port(),
            timeout,
            dummy_handle!(),
            receiver,
            sender,
        );

        (write, read)
    }

    async fn write_server(server_socket: &mut TcpStream, data: &str) {
        server_socket
            .write(data.as_bytes())
            .or(async_timeout!())
            .await
            .expect("server write");
    }

    async fn read_server(server_socket: &mut TcpStream) -> String {
        let mut buffer = [0; 4096];

        let read = match server_socket.read(&mut buffer).await {
            Err(e) => panic!("server read error: {e}"),
            Ok(read) => read,
        };

        let buffer = buffer.into_iter().take(read).collect::<Vec<_>>();
        String::from_utf8(buffer).unwrap()
    }

    async fn write_client(client_write: &Sender<Vec<u8>>, data: &str) {
        client_write
            .send(data.as_bytes().to_vec())
            .or(async_timeout!())
            .await
            .expect("client write");
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_timeout() {
        let (_accept_task, addr) = start_test_server().await;
        let (_client_write, client_read) = connect_test_socket(addr, TIMEOUT_ZERO, true);
        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::TimedOut),
        );
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_connect() {
        let (accept_task, addr) = start_test_server().await;
        let (_client_write, client_read) = connect_test_socket(addr, TIMEOUT, true);
        let _server_socket = accept_task.await.unwrap();
        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Connected),
        );
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_deny() {
        let (_accept_task, addr) = start_test_server().await;
        let (_client_write, client_read) = connect_test_socket(addr, TIMEOUT, false);

        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Failed),
        );
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_fail() {
        // We want to use here an address that will cause the connection to fail instantly.
        // On Windows and macOS we use an IPv6 black hole address,
        // as connections to 127.0.0.42:226 do not fail instantly.
        // On Linux we try to use a loopback address with an unused port,
        // because some hosts try connecting to the black hole address.
        let addr = if cfg!(target_os = "linux") {
            SocketAddr::from_str("127.0.0.42:226")
        } else {
            SocketAddr::from_str("[100::]:42")
        }
        .expect("test address");
        let (_client_write, client_read) = connect_test_socket(addr, TIMEOUT, true);

        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Failed),
        );
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_server_close() {
        let (accept_task, addr) = start_test_server().await;
        let (_client_write, client_read) = connect_test_socket(addr, TIMEOUT, true);

        let server_socket = accept_task.await.unwrap();
        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Connected),
        );

        drop(server_socket);

        assert_next_socket_actions!(
            client_read;
            Close(dummy_handle!()),
        );
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_client_close() {
        let (accept_task, addr) = start_test_server().await;
        let (client_write, client_read) = connect_test_socket(addr, TIMEOUT, true);

        let mut server_socket = accept_task.await.unwrap();
        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Connected),
        );

        drop(client_write);

        assert_eq!(read_server(&mut server_socket).await, "");
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_basic_communication() {
        let (accept_task, addr) = start_test_server().await;
        let (client_write, client_read) = connect_test_socket(addr, TIMEOUT, true);

        let mut server_socket = accept_task.await.unwrap();
        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Connected),
        );

        write_server(&mut server_socket, "Hello World!").await;

        assert_next_socket_actions!(
            client_read;
            Data(dummy_handle!(), "Hello World!".as_bytes().to_vec()),
        );

        write_client(&client_write, "Hello from client").await;

        assert_eq!(read_server(&mut server_socket).await, "Hello from client");

        write_server(&mut server_socket, "from server 2").await;
        write_client(&client_write, "from client 2").await;

        assert_next_socket_actions!(
            client_read;
            Data(dummy_handle!(), "from server 2".as_bytes().to_vec()),
        );
        assert_eq!(read_server(&mut server_socket).await, "from client 2");
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_tap_full_queue_preserves_io_and_offsets() {
        use socket_tap::{SocketTapEvent, SocketTapKind};
        use std::cell::RefCell;
        use std::sync::mpsc::{SyncSender, sync_channel};

        // These tests use single-thread Tokio runtimes; keep the synthetic sink local to this test.
        thread_local! {
            static SINK: RefCell<Option<SyncSender<SocketTapEvent>>> = const { RefCell::new(None) };
        }
        socket_tap::install(|event| {
            SINK.with_borrow(|sink| {
                if let Some(sink) = sink {
                    let _ = sink.try_send(event);
                }
            });
        });
        let session = socket_tap::new_session();
        let (sender, events) = sync_channel(1);
        SINK.set(Some(sender));
        let (accepted, addr) = start_test_server().await;
        let (client_write, client_read) = connect_test_socket(addr, TIMEOUT, true);
        let mut server = accepted.await.unwrap();
        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Connected),
        );

        // Connected occupies the only queue slot throughout this two-way exchange.
        write_server(&mut server, "a").await;
        assert_next_socket_actions!(client_read; Data(dummy_handle!(), vec![b'a']),);
        let sent = vec![42; 9000];
        client_write.send(sent.clone()).await.unwrap();
        let mut received = vec![0; sent.len()];
        server
            .read_exact(&mut received)
            .or(async_timeout!())
            .await
            .unwrap();
        assert_eq!(received, sent);

        let connected = events.try_recv().unwrap();
        assert_eq!(connected.session, session);
        assert_eq!(
            connected.kind,
            SocketTapKind::Connected {
                host: addr.ip().to_string(),
                port: addr.port(),
            }
        );
        let next = || {
            let event = events.try_recv().unwrap();
            assert_eq!((event.session, event.handle), (session, connected.handle));
            assert!(std::sync::Arc::ptr_eq(&event.lifetime, &connected.lifetime));
            event.kind
        };
        write_client(&client_write, "x").await;
        assert_eq!(read_server(&mut server).or(async_timeout!()).await, "x");
        assert_eq!(
            next(),
            SocketTapKind::Data {
                direction: SocketDirection::Send,
                offset: sent.len() as u64,
                bytes: vec![b'x'],
            }
        );
        write_server(&mut server, "y").await;
        assert_next_socket_actions!(client_read; Data(dummy_handle!(), vec![b'y']),);
        assert_eq!(
            next(),
            SocketTapKind::Data {
                direction: SocketDirection::Recv,
                offset: 1,
                bytes: vec![b'y'],
            }
        );

        // An old socket can still transfer bytes after its Player has been replaced.
        let new_session = socket_tap::new_session();
        write_server(&mut server, "z").await;
        assert_next_socket_actions!(client_read; Data(dummy_handle!(), vec![b'z']),);
        assert!(events.try_recv().is_err());
        drop(client_write);
        assert_eq!(read_server(&mut server).or(async_timeout!()).await, "");
        assert_eq!(next(), SocketTapKind::Closed);
        assert!(events.try_recv().is_err());

        let (accepted, addr) = start_test_server().await;
        let (_client_write, client_read) = connect_test_socket(addr, TIMEOUT, true);
        let server = accepted.await.unwrap();
        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Connected),
        );
        let reopened = events.try_recv().unwrap();
        assert_eq!((reopened.session, reopened.handle), (new_session, connected.handle));
        assert!(!std::sync::Arc::ptr_eq(&reopened.lifetime, &connected.lifetime));
        assert!(matches!(reopened.kind, SocketTapKind::Connected { .. }));
        drop(server);
        assert_next_socket_actions!(client_read; Close(dummy_handle!()),);
        let closed = events.try_recv().unwrap();
        assert_eq!((closed.session, closed.handle), (new_session, reopened.handle));
        assert!(std::sync::Arc::ptr_eq(&closed.lifetime, &reopened.lifetime));
        assert_eq!(closed.kind, SocketTapKind::Closed);
        assert!(events.try_recv().is_err());
        SINK.take();
    }

    #[macro_rules_attribute::apply(async_test)]
    async fn test_socket_flush_before_close() {
        let (accept_task, addr) = start_test_server().await;
        let (client_write, client_read) = connect_test_socket(addr, TIMEOUT, true);

        let mut server_socket = accept_task.await.unwrap();
        assert_next_socket_actions!(
            client_read;
            Connect(dummy_handle!(), ConnectionState::Connected),
        );

        write_client(&client_write, "Sending some data").await;
        client_write.close();

        assert_eq!(read_server(&mut server_socket).await, "Sending some data");
    }
}
