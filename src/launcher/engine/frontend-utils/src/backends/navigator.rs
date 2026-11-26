mod fetch;
mod aola_pet_patch;

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

        if let Some(patch) = &self.aola_pet_patch {
            if aola_pet_patch::PetPatch::accepts(&request, &processed_url) {
                let response: Box<dyn SuccessResponse> = Box::new(Response {
                    url: processed_url.to_string(), response_body: ResponseBody::File(Ok(patch.bytes())),
                    text_encoding: None, status: 200, redirected: false,
                });
                return async_return(Ok(response));
            }
        }

        let client = self.client.clone();
        let cookie_store = self.cookie_store.clone();
        ruffle_core::aola_panel_diagnostics::resource(processed_url.as_str(), "resource-request", 0);
        let cache = self.aoqi_cache.clone().filter(|_| {
            AoqiCache::accepts_request(&request, &processed_url)
                && self.cookie_store.cookies(&processed_url).is_none()
        });

        match processed_url.scheme() {
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
                            return Ok(response);
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
                if let (Some(cache), Some(policy)) = (cache, cache_policy) {
                    Ok(Box::new(CachingResponse::new(response, cache, processed_url, policy)) as Box<dyn SuccessResponse>)
                } else { Ok(response) }
            }),
        }
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

            let sender = sender;
            //NOTE: We clone the sender here as we cant share it between async tasks.
            let sender2 = sender.clone();
            let (mut read, mut write) = stream.split();

            let read = async move {
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
