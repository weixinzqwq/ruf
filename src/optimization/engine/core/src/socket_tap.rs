//! Optional observations of successful game socket I/O. The desktop sink must never wait.
use crate::socket::SocketHandle;
use slotmap::Key;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

static SINK: OnceLock<fn(SocketTapEvent)> = OnceLock::new();
static SESSION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum SocketDirection {
    Send,
    Recv,
}

#[derive(Debug, Eq, PartialEq)]
pub struct SocketTapEvent {
    pub session: u64,
    pub handle: u64,
    /// Owned by the connection and its pending events. A weak observer can detect a lost close.
    pub lifetime: Arc<()>,
    pub kind: SocketTapKind,
}

#[derive(Debug, Eq, PartialEq)]
pub enum SocketTapKind {
    Connected {
        host: String,
        port: u16,
    },
    Data {
        direction: SocketDirection,
        /// TCP byte position in this direction, including chunks dropped by the sink.
        offset: u64,
        bytes: Vec<u8>,
    },
    Closed,
}

pub fn install(sink: fn(SocketTapEvent)) {
    let _ = SINK.set(sink);
}

pub fn new_session() -> u64 {
    SESSION.fetch_add(1, Ordering::AcqRel) + 1
}

pub fn current_session() -> u64 {
    SESSION.load(Ordering::Acquire)
}

pub struct Connection {
    session: u64,
    handle: u64,
    sink: fn(SocketTapEvent),
    lifetime: Arc<()>,
}

impl Connection {
    /// The caller captures the session before spawning the TCP connection attempt.
    pub fn connected(session: u64, handle: SocketHandle, host: String, port: u16) -> Option<Self> {
        let sink = *SINK.get()?;
        if session != current_session() {
            return None;
        }
        let connection = Self {
            session,
            handle: handle.data().as_ffi(),
            sink,
            lifetime: Arc::new(()),
        };
        connection.emit(SocketTapKind::Connected { host, port });
        Some(connection)
    }

    fn emit(&self, kind: SocketTapKind) {
        (self.sink)(SocketTapEvent {
            session: self.session,
            handle: self.handle,
            lifetime: self.lifetime.clone(),
            kind,
        });
    }

    pub fn data(&self, direction: SocketDirection, offset: &mut u64, bytes: &[u8]) {
        if self.session == current_session() {
            // A single socket write can be much larger than a read. Bound each queued allocation.
            for chunk in bytes.chunks(4096) {
                self.emit(SocketTapKind::Data {
                    direction,
                    offset: *offset,
                    bytes: chunk.to_vec(),
                });
                *offset += chunk.len() as u64;
            }
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.emit(SocketTapKind::Closed);
    }
}
