//! Observes the existing navigator channels. No bytes or socket actions are rewritten.
use super::{bridge, protocol::Direction};
use async_channel::{Receiver, Sender, bounded};
use ruffle_core::backend::navigator::{ErrorResponse, NavigationMethod, NavigatorBackend, OwnedFuture, Request, SuccessResponse};
use ruffle_core::indexmap::IndexMap;
use ruffle_core::loader::Error;
use ruffle_core::socket::{ConnectionState, SocketAction, SocketHandle};
use std::collections::{BTreeSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use url::{ParseError, Url};

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_EVENTS: usize = 4096;
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);
static QUEUE: OnceLock<Mutex<ObservationQueue>> = OnceLock::new();

#[derive(Debug)]
pub enum NetworkEvent {
    Opened { connection: u64 },
    Data { connection: u64, direction: Direction, bytes: Vec<u8> },
    Closed { connection: u64 },
    Gap { connection: u64 },
}

#[derive(Default)]
struct ObservationQueue {
    generation: u64,
    bytes: usize,
    active: BTreeSet<u64>,
    events: VecDeque<NetworkEvent>,
}

fn queue() -> &'static Mutex<ObservationQueue> {
    QUEUE.get_or_init(|| Mutex::new(ObservationQueue::default()))
}

/// Call before constructing a new movie's navigator. Old relays still forward traffic,
/// but their observations cannot contaminate the new movie's decoder state.
pub fn reset() {
    let mut queue = queue().lock().unwrap();
    let generation = queue.generation.wrapping_add(1);
    *queue = ObservationQueue { generation, ..Default::default() };
}

pub fn drain() -> Vec<NetworkEvent> {
    let mut queue = queue().lock().unwrap();
    queue.bytes = 0;
    queue.events.drain(..).collect()
}

impl ObservationQueue {
    fn push(&mut self, generation: u64, event: NetworkEvent) {
        if generation != self.generation { return; }
        let bytes = match &event { NetworkEvent::Data { bytes, .. } => bytes.len(), _ => 0 };
        if self.bytes + bytes > MAX_BYTES || self.events.len() >= MAX_EVENTS {
            self.events.clear();
            self.bytes = 0;
            // A global queue gap affects every active parser, including those whose
            // earlier data happened to be removed. Never silently resynchronize EXT.
            for &connection in &self.active {
                self.events.push_back(NetworkEvent::Gap { connection });
            }
            if bytes > MAX_BYTES { return; }
        }
        match &event {
            NetworkEvent::Opened { connection } => { self.active.insert(*connection); }
            NetworkEvent::Closed { connection } => { self.active.remove(connection); }
            _ => {}
        }
        self.bytes += bytes;
        self.events.push_back(event);
    }
}

fn observe(generation: u64, event: NetworkEvent) {
    queue().lock().unwrap().push(generation, event);
}

async fn send_relay(source: Receiver<Vec<u8>>, backend: Sender<Vec<u8>>, generation: u64, connection: u64) {
    loop {
        let data = tokio::select! {
            _ = backend.closed() => break,
            result = source.recv() => match result { Ok(data) => data, Err(_) => break },
        };
        observe(generation, NetworkEvent::Data { connection, direction: Direction::Send, bytes: data.clone() });
        if backend.send(data).await.is_err() { break; }
    }
    source.close();
    backend.close();
}

async fn receive_relay(source: Receiver<SocketAction>, core: Sender<SocketAction>, generation: u64, connection: u64) {
    loop {
        let action = tokio::select! {
            _ = core.closed() => break,
            result = source.recv() => match result { Ok(action) => action, Err(_) => break },
        };
        let terminal = matches!(&action, SocketAction::Close(_) |
            SocketAction::Connect(_, ConnectionState::Failed | ConnectionState::TimedOut));
        if let SocketAction::Data(_, bytes) = &action {
            observe(generation, NetworkEvent::Data { connection, direction: Direction::Receive, bytes: bytes.clone() });
        }
        if core.send(action).await.is_err() { break; }
        if terminal { break; }
    }
    source.close();
    observe(generation, NetworkEvent::Closed { connection });
}

pub struct AolaNavigator<N> { inner: N, generation: u64 }

impl<N> AolaNavigator<N> {
    pub fn new(inner: N) -> Self {
        Self { inner, generation: queue().lock().unwrap().generation }
    }
}

impl<N: NavigatorBackend> NavigatorBackend for AolaNavigator<N> {
    fn navigate_to_url(&self, url: &str, target: &str, vars: Option<(NavigationMethod, IndexMap<String, String>)>) {
        self.inner.navigate_to_url(url, target, vars);
    }
    fn fetch(&self, request: Request) -> OwnedFuture<Box<dyn SuccessResponse>, ErrorResponse> {
        if let Ok(url) = self.inner.resolve_url(request.url())
            && bridge::accepts(&request, &url)
        {
            crate::aola_host::record("launcher", "send-bridge-resource-hit");
            return bridge::response(url.to_string());
        }
        self.inner.fetch(request)
    }
    fn resolve_url(&self, url: &str) -> Result<Url, ParseError> { self.inner.resolve_url(url) }
    fn pre_process_url(&self, url: Url) -> Url { self.inner.pre_process_url(url) }
    fn spawn_future(&mut self, future: OwnedFuture<(), Error>) { self.inner.spawn_future(future); }
    fn connect_socket(&mut self, host: String, port: u16, timeout: Duration, handle: SocketHandle,
                      receiver: Receiver<Vec<u8>>, sender: Sender<SocketAction>) {
        let connection = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
        // Allocate decoder state before either relay can publish bytes. Connected
        // can be consumed by the game before its receive relay is next scheduled.
        observe(self.generation, NetworkEvent::Opened { connection });
        let (out_sender, out_receiver) = bounded(16);
        let (in_sender, in_receiver) = bounded(16);
        self.inner.connect_socket(host, port, timeout, handle, out_receiver, in_sender);
        tokio::spawn(send_relay(receiver, out_sender, self.generation, connection));
        tokio::spawn(receive_relay(in_receiver, sender, self.generation, connection));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_network_overflow_invalidates_all_active_connections() {
        let mut queue = ObservationQueue::default();
        queue.push(0, NetworkEvent::Opened { connection: 1 });
        queue.push(0, NetworkEvent::Opened { connection: 2 });
        queue.push(0, NetworkEvent::Data { connection: 1, direction: Direction::Send, bytes: vec![0; MAX_BYTES + 1] });
        assert_eq!(queue.events.len(), 2);
        assert!(queue.events.iter().all(|event| matches!(event, NetworkEvent::Gap { .. })));
        assert_eq!(queue.bytes, 0);
        queue.push(1, NetworkEvent::Opened { connection: 3 });
        assert_eq!(queue.events.len(), 2);
    }
    #[tokio::test]
    async fn aola_network_send_preserves_chunks_and_closes() {
        let (source, receiver) = bounded(4);
        let (sender, backend) = bounded(4);
        source.send(vec![0, 255, 1]).await.unwrap();
        source.send(b"<policy-file-request/>\0".to_vec()).await.unwrap();
        source.close();
        send_relay(receiver, sender, u64::MAX, 1).await;
        assert_eq!(backend.recv().await.unwrap(), [0, 255, 1]);
        assert_eq!(backend.recv().await.unwrap(), b"<policy-file-request/>\0");
        assert!(backend.recv().await.is_err());
    }
    #[tokio::test]
    async fn aola_network_receive_preserves_policy_and_terminal_actions() {
        let (backend, source) = bounded(4);
        let (core, consumer) = bounded(4);
        let handle = SocketHandle::default();
        backend.send(SocketAction::Connect(handle, ConnectionState::Connected)).await.unwrap();
        let policy = b"<cross-domain-policy/>\0".to_vec();
        backend.send(SocketAction::Data(handle, policy.clone())).await.unwrap();
        backend.send(SocketAction::Close(handle)).await.unwrap();
        receive_relay(source, core, u64::MAX, 1).await;
        assert_eq!(consumer.recv().await.unwrap(), SocketAction::Connect(handle, ConnectionState::Connected));
        assert_eq!(consumer.recv().await.unwrap(), SocketAction::Data(handle, policy));
        assert_eq!(consumer.recv().await.unwrap(), SocketAction::Close(handle));
        assert!(consumer.recv().await.is_err());
        assert!(backend.is_closed());
    }
    #[tokio::test]
    async fn aola_network_backend_drop_releases_send_relay() {
        let (source, receiver) = bounded(4);
        let (backend, transport) = bounded(4);
        drop(transport);
        tokio::time::timeout(Duration::from_secs(1), send_relay(receiver, backend, u64::MAX, 1)).await.unwrap();
        assert!(source.is_closed());
    }
}
