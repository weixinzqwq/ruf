//! Passive telemetry only. All decoding and disk work runs outside the socket and UI threads.
use super::battle_state::BattleState;
use super::packet_log::{self, LogStatus, Logger};
use super::protocol::{Decoder, Direction, Packet};
use chrono::Utc;
use ruffle_core::socket_tap::{self, SocketDirection, SocketTapEvent, SocketTapKind};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_QUEUE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ROWS: usize = 512;
const MAX_ROW_BYTES: usize = 4 * 1024 * 1024;
const MAX_EXPORT_BYTES: usize = 64 * 1024;
const MAX_CONNECTIONS: usize = 32;
static HUB: OnceLock<Result<Arc<Hub>, String>> = OnceLock::new();

enum Message {
    Event(SocketTapEvent, usize),
    Reset(u64),
    Retain(u64, bool),
    #[cfg(test)]
    Barrier(mpsc::Sender<()>),
    End,
}
struct Ingress {
    tx: mpsc::SyncSender<Message>,
    bytes: Arc<AtomicUsize>,
    dropped: Arc<AtomicU64>,
    stopped: AtomicBool,
}
impl Ingress {
    fn observe(&self, event: SocketTapEvent) {
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        let size = match &event.kind {
            SocketTapKind::Data { bytes, .. } => bytes.len(),
            _ => 0,
        };
        if self
            .bytes
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                n.checked_add(size).filter(|n| *n <= MAX_QUEUE_BYTES)
            })
            .is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.tx.try_send(Message::Event(event, size)).is_err() {
            self.bytes.fetch_sub(size, Ordering::Relaxed);
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
fn observe(event: SocketTapEvent) {
    if event.session == socket_tap::current_session()
        && let Some(Ok(hub)) = HUB.get()
    {
        hub.input.observe(event);
    }
}

struct Row {
    seq: u64,
    data: Value,
    export: Option<String>,
    bytes: usize,
}
#[derive(Default)]
struct View {
    battle: Option<Arc<Value>>,
    epoch: u64,
    seq: u64,
    rows: VecDeque<Arc<Row>>,
    bytes: usize,
    dropped: u64,
    gaps: u64,
    log: Arc<Mutex<LogStatus>>,
    log_drops: Arc<AtomicU64>,
}
impl View {
    fn push(&mut self, data: Value, export: Option<String>, bytes: usize) {
        self.seq += 1;
        while self.rows.len() >= MAX_ROWS || self.bytes + bytes > MAX_ROW_BYTES {
            let Some(old) = self.rows.pop_front() else {
                break;
            };
            self.bytes -= old.bytes;
            self.dropped += 1;
        }
        self.bytes += bytes;
        self.rows.push_back(Arc::new(Row {
            seq: self.seq,
            data,
            export,
            bytes,
        }));
    }
}
#[derive(Default)]
struct Connection {
    generation: u64,
    lifetime: Weak<()>,
    send: Decoder,
    receive: Decoder,
    offsets: [Option<u64>; 2],
}
impl Connection {
    fn opened(generation: u64, lifetime: Weak<()>) -> Self {
        Self {
            generation,
            lifetime,
            offsets: [Some(0), Some(0)],
            ..Default::default()
        }
    }

    /// A gap invalidates only the affected direction. Keep reading the intact peer stream.
    fn feed(
        &mut self,
        dir: SocketDirection,
        offset: u64,
        data: &[u8],
        mut output: impl FnMut(Packet),
    ) -> bool {
        let player_id = self.send.player_id();
        let (i, decoder, direction) = match dir {
            SocketDirection::Send => (0, &mut self.send, Direction::Send),
            SocketDirection::Recv => (1, &mut self.receive, Direction::Receive),
        };
        let Some(expected) = self.offsets[i] else {
            return false;
        };
        if offset != expected {
            self.offsets[i] = None;
            decoder.invalidate();
            return true;
        }
        self.offsets[i] = offset.checked_add(data.len() as u64);
        decoder.feed_each(direction, data, |mut packet| {
            if direction == Direction::Receive {
                packet.connection_player_id = player_id;
            }
            output(packet)
        });
        if !decoder.can_frame() {
            self.offsets[i] = None;
            return true;
        }
        false
    }
}

#[derive(Default)]
struct BattleFeed {
    // Connected order distinguishes a late start/close on a previous socket from a reconnect.
    owner: Option<(u64, u64)>,
    retired_generation: u64,
    state: BattleState,
}
impl BattleFeed {
    fn clear(&mut self, handle: u64, view: &Mutex<View>) {
        if let Some((owner, generation)) = self.owner
            && owner == handle
        {
            self.retired_generation = generation;
            self.owner = None;
            self.state = BattleState::default();
            view.lock().unwrap().battle = None;
        }
    }
    fn observe(
        &mut self,
        handle: u64,
        generation: u64,
        packet: &Packet,
        view: &Mutex<View>,
    ) -> bool {
        if packet.direction != Direction::Receive
            || packet.connection_player_id.is_none()
            || generation <= self.retired_generation
        {
            return false;
        }
        let Some(param) = packet.parameters.as_ref() else {
            return false;
        };
        if self.owner != Some((handle, generation)) {
            if self
                .owner
                .is_some_and(|(_, owner_gen)| generation < owner_gen)
            {
                return false;
            }
            let mut state = BattleState::default();
            if !state.observe(&packet.command, param, packet.connection_player_id) {
                return false;
            }
            self.state = state;
            self.owner = Some((handle, generation));
        } else if !self
            .state
            .observe(&packet.command, param, packet.connection_player_id)
        {
            return false;
        }
        let snapshot = Arc::new(self.state.snapshot());
        view.lock().unwrap().battle = Some(snapshot);
        true
    }
}

struct Worker {
    epoch: u64,
    generation: u64,
    battle: BattleFeed,
    root: PathBuf,
    connections: HashMap<u64, Connection>,
    view: Arc<Mutex<View>>,
    ingress_drops: Arc<AtomicU64>,
    logger: Option<Logger>,
    closing: Option<JoinHandle<()>>,
}
impl Worker {
    fn new(root: &Path, view: Arc<Mutex<View>>, ingress_drops: Arc<AtomicU64>) -> Self {
        Self {
            epoch: 0,
            generation: 0,
            battle: BattleFeed::default(),
            root: root.to_owned(),
            connections: HashMap::new(),
            view,
            ingress_drops,
            logger: None,
            closing: None,
        }
    }
    fn reap_closed(&mut self) {
        self.connections.retain(|handle, conn| {
            if conn.lifetime.strong_count() != 0 {
                return true;
            }
            self.battle.clear(*handle, &self.view);
            false
        });
    }
    fn reset(&mut self, epoch: u64) {
        if epoch <= self.epoch {
            return;
        }
        self.close_log("sessionReset");
        self.connections.clear();
        self.battle = BattleFeed::default();
        self.epoch = epoch;
        *self.view.lock().unwrap() = View {
            epoch,
            ..Default::default()
        };
    }
    fn boundary(&self, event: &str) -> Value {
        let (seen, gaps, lost, evicted, battle) = {
            let view = self.view.lock().unwrap();
            (
                view.seq,
                view.gaps,
                view.log_drops.load(Ordering::Relaxed),
                view.dropped,
                view.battle.clone(),
            )
        };
        let disabled_directions = self
            .connections
            .values()
            .flat_map(|c| &c.offsets)
            .filter(|offset| offset.is_none())
            .count();
        json!({"event":event,"processId":std::process::id(),"timeUtc":Utc::now().to_rfc3339(),
            "epoch":self.epoch,"observedRows":seen,"droppedChunks":self.ingress_drops.load(Ordering::Relaxed),
            "gaps":gaps,"disabledDirections":disabled_directions,"droppedLogRows":lost,
            "evictedUiRows":evicted,"battle":packet_log::battle_evidence(battle.as_deref())})
    }
    fn close_log(&mut self, reason: &str) {
        if let Some(logger) = self.logger.take() {
            let mut end = self.boundary("end");
            end["reason"] = json!(reason);
            self.closing = Some(logger.close(end));
        }
    }
    fn retain(&mut self, enabled: bool) {
        if !enabled {
            self.close_log("retentionOff");
            return;
        }
        let (status, dropped) = {
            let view = self.view.lock().unwrap();
            (view.log.clone(), view.log_drops.clone())
        };
        if let Some(active) = &self.logger {
            if !active.finished() && status.lock().unwrap().error.is_none() {
                return;
            }
            self.close_log("writerRestart");
            // A failed writer has already closed its packet receiver and only awaits the footer.
            if let Some(done) = self.closing.take() {
                let _ = done.join();
            }
        }
        if self.closing.as_ref().is_some_and(|c| !c.is_finished()) {
            status.lock().unwrap().error = Some("上一份日志正在写完，请稍后重试".into());
            return;
        }
        if let Some(done) = self.closing.take() {
            let _ = done.join();
        }
        match Logger::start(&self.root, status.clone(), dropped, self.boundary("start")) {
            Ok(logger) => self.logger = Some(logger),
            Err(e) => {
                *status.lock().unwrap() = LogStatus {
                    error: Some(e.to_string()),
                    ..Default::default()
                }
            }
        }
    }
    fn event(&mut self, event: SocketTapEvent) {
        self.reset(event.session);
        if event.session != self.epoch {
            return;
        }
        match event.kind {
            SocketTapKind::Connected { .. } => {
                if self.connections.len() >= MAX_CONNECTIONS {
                    self.reap_closed();
                }
                if self.connections.len() >= MAX_CONNECTIONS
                    && !self.connections.contains_key(&event.handle)
                {
                    self.view.lock().unwrap().gaps += 1;
                    return;
                }
                self.battle.clear(event.handle, &self.view);
                self.generation += 1;
                self.connections.insert(
                    event.handle,
                    Connection::opened(self.generation, Arc::downgrade(&event.lifetime)),
                );
            }
            SocketTapKind::Closed => {
                if self.connections.get(&event.handle).is_some_and(|c| {
                    std::ptr::eq(c.lifetime.as_ptr(), Arc::as_ptr(&event.lifetime))
                }) {
                    self.connections.remove(&event.handle);
                    self.battle.clear(event.handle, &self.view);
                }
            }
            SocketTapKind::Data {
                direction,
                offset,
                bytes,
            } => {
                let Some(conn) = self.connections.get_mut(&event.handle) else {
                    return;
                };
                if !std::ptr::eq(conn.lifetime.as_ptr(), Arc::as_ptr(&event.lifetime)) {
                    return;
                }
                let view = &self.view;
                let logger = self.logger.as_ref();
                let generation = conn.generation;
                let gap = conn.feed(direction, offset, &bytes, |packet| {
                    let applied = self.battle.observe(event.handle, generation, &packet, view);
                    Self::record(view, logger, event.handle, packet, applied)
                });
                if gap {
                    self.battle.clear(event.handle, view);
                    view.lock().unwrap().gaps += 1;
                }
            }
        }
    }
    fn record(
        view: &Mutex<View>,
        logger: Option<&Logger>,
        handle: u64,
        packet: Packet,
        applied: bool,
    ) {
        let now = Utc::now();
        if let Some(logger) = logger {
            let checkpoint = (packet.direction == Direction::Receive
                && matches!(packet.command.as_str(), "2303" | "2304" | "2401" | "2411" | "2413" | "2423" | "2426" | "2404" | "2309"))
                .then(|| {
                    let (seq, battle) = {
                        let current = view.lock().unwrap();
                        (current.seq + 1, current.battle.clone())
                    };
                    json!({"event":"battle","timeUtc":now.to_rfc3339(),"processId":std::process::id(),
                        "observedRow":seq,"socket":handle,"command":packet.command,"applied":applied,
                        "battle":packet_log::battle_evidence(battle.as_deref())})
                });
            logger.push(&packet_log::record(now, handle, &packet), checkpoint);
        }
        let direction = if packet.direction == Direction::Send {
            "send"
        } else {
            "recv"
        };
        let export = packet.export.filter(|s| s.len() <= MAX_EXPORT_BYTES);
        let command: String = packet.command.chars().take(256).collect();
        let summary: String = packet.summary.chars().take(512).collect();
        let size = command.len() + summary.len() + export.as_ref().map_or(0, String::len) + 256;
        let data = json!({"timeMs":now.timestamp_millis(),"direction":direction,"socket":handle.to_string(),
            "cmd":command,"summary":summary,"length":packet.length,
            "decoded":!matches!(packet.command.as_str(),"decode-error"|"unavailable"),"exportable":export.is_some()});
        view.lock().unwrap().push(data, export, size);
    }
    fn run(mut self, rx: mpsc::Receiver<Message>, bytes: Arc<AtomicUsize>) {
        let interval = Duration::from_millis(200);
        let mut next_sweep = Instant::now() + interval;
        loop {
            let now = Instant::now();
            if now >= next_sweep {
                self.reap_closed();
                next_sweep = now + interval;
            }
            match rx.recv_timeout(next_sweep - now) {
                Ok(Message::Event(e, size)) => {
                    bytes.fetch_sub(size, Ordering::Relaxed);
                    self.event(e);
                }
                Ok(Message::Reset(epoch)) => self.reset(epoch),
                Ok(Message::Retain(epoch, enabled)) => {
                    self.reset(epoch);
                    if epoch == self.epoch {
                        self.retain(enabled);
                    }
                }
                Ok(Message::End) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                #[cfg(test)]
                Ok(Message::Barrier(done)) => {
                    let _ = done.send(());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => self.reset(socket_tap::current_session()),
            }
        }
        self.close_log("shutdown");
        if let Some(job) = self.closing.take() {
            let _ = job.join();
        }
    }
}

struct Hub {
    input: Ingress,
    view: Arc<Mutex<View>>,
    job: Mutex<Option<JoinHandle<()>>>,
}
impl Hub {
    fn finish(&self) {
        if !self.input.stopped.swap(true, Ordering::Relaxed) {
            let _ = self.input.tx.send(Message::End);
            if let Some(job) = self.job.lock().unwrap().take() {
                let _ = job.join();
            }
        }
    }
    fn start(root: &Path) -> Result<Self, String> {
        let (tx, rx) = mpsc::sync_channel(2048);
        let bytes = Arc::new(AtomicUsize::new(0));
        let view = Arc::new(Mutex::new(View::default()));
        let dropped = Arc::new(AtomicU64::new(0));
        let worker = Worker::new(root, view.clone(), dropped.clone());
        let pending = bytes.clone();
        let job = thread::Builder::new()
            .name("mypro-packet-decoder".into())
            .spawn(move || worker.run(rx, pending))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            input: Ingress {
                tx,
                bytes,
                dropped,
                stopped: AtomicBool::new(false),
            },
            view,
            job: Mutex::new(Some(job)),
        })
    }
}

pub struct Packets {
    hub: Result<Arc<Hub>, String>,
    epoch: u64,
    drop_base: u64,
    last_details: Option<Arc<Value>>,
}
impl Packets {
    pub fn new(root: &Path) -> Self {
        let hub = HUB.get_or_init(|| Hub::start(root).map(Arc::new)).clone();
        if hub.is_ok() {
            socket_tap::install(observe);
        }
        Self {
            hub,
            epoch: socket_tap::current_session(),
            drop_base: 0,
            last_details: None,
        }
    }
    pub fn available(&self) -> bool {
        self.hub.is_ok()
    }
    pub fn battle_details(&mut self) -> Value {
        if let Ok(hub) = &self.hub
            && let Ok(view) = hub.view.try_lock()
        {
            self.last_details = (view.epoch == self.epoch)
                .then(|| view.battle.clone())
                .flatten();
        }
        self.last_details
            .as_deref()
            .cloned()
            .unwrap_or_else(|| BattleState::default().snapshot())
    }
    pub fn reset(&mut self) {
        self.last_details = None;
        self.epoch = socket_tap::new_session();
        if let Ok(hub) = &self.hub {
            self.drop_base = hub.input.dropped.load(Ordering::Relaxed);
            let _ = hub.input.tx.try_send(Message::Reset(self.epoch));
        }
    }
    pub fn command(&self, name: &str, p: &Value) -> Result<Value, String> {
        let hub = self.hub.as_ref().map_err(|e| format!("Unavailable: {e}"))?;
        if name == "packets.retain.set" {
            let enabled = p["enabled"].as_bool().ok_or("InvalidArguments")?;
            hub.input
                .tx
                .try_send(Message::Retain(self.epoch, enabled))
                .map_err(|_| "Busy: 抓包后台正在处理")?;
            return Ok(json!({"requested":enabled}));
        }
        let view = hub.view.try_lock().map_err(|_| "Busy")?;
        let empty = View::default();
        let current = if view.epoch == self.epoch {
            &*view
        } else {
            &empty
        };
        if name == "packets.export" {
            let seq = p["seq"].as_u64().ok_or("InvalidArguments")?;
            let row = current
                .rows
                .iter()
                .find(|row| row.seq == seq)
                .cloned()
                .ok_or("NotFound")?;
            drop(view);
            return Ok(json!({"seq":seq,"text":row.export.as_ref().ok_or("NotExportable")?}));
        }
        let limit = p
            .get("limit")
            .map(|v| {
                v.as_u64()
                    .filter(|n| (1..=100).contains(n))
                    .ok_or("InvalidArguments")
            })
            .transpose()?
            .unwrap_or(40) as usize;
        let after = p
            .get("after")
            .map(|v| v.as_u64().ok_or("InvalidArguments"))
            .transpose()?;
        let filter = p
            .get("exportableOnly")
            .map(|v| v.as_bool().ok_or("InvalidArguments"))
            .transpose()?
            .unwrap_or(false);
        let mut matches = current
            .rows
            .iter()
            .rev()
            .filter(|r| after.is_none_or(|a| r.seq < a) && (!filter || r.export.is_some()));
        let selected: Vec<_> = matches.by_ref().take(limit).cloned().collect();
        let next = matches.next().and_then(|_| selected.last().map(|r| r.seq));
        let first = current.rows.front().map(|r| r.seq);
        let last = current.rows.back().map(|r| r.seq);
        let dropped = current.dropped;
        let gaps = current.gaps;
        let log_drops = current.log_drops.load(Ordering::Relaxed);
        let log = current.log.clone();
        drop(view);
        let log = log.try_lock().map_err(|_| "Busy")?.clone();
        let rows: Vec<_> = selected
            .iter()
            .map(|r| {
                let mut v = r.data.clone();
                v["seq"] = json!(r.seq);
                v
            })
            .collect();
        Ok(
            json!({"rows":rows,"nextAfter":next,"oldestSeq":first,"newestSeq":last,
            "gap":dropped>0&&after.zip(first).is_some_and(|(a,f)|a<=f),"droppedRows":dropped,"gaps":gaps,
            "droppedChunks":hub.input.dropped.load(Ordering::Relaxed).saturating_sub(self.drop_base),
            "retaining":log.enabled,"logPending":log.pending,"logFile":log.file,"logError":log.error,"droppedLogRows":log_drops}),
        )
    }
}

/// Called only after the winit loop returns. Drain accepted rows before process exit.
pub fn finish() {
    if let Some(Ok(hub)) = HUB.get() {
        hub.finish();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    // Minimal AMF3 fields used by the offline battle fixtures.
    fn encode_amf(out: &mut Vec<u8>, v: &Value) {
        use crate::mypro_features::protocol::tests::{amf_string, u29};
        match v {
            Value::Bool(b) => out.push(if *b { 3 } else { 2 }),
            Value::String(s) => {
                out.push(6);
                amf_string(out, s);
            }
            Value::Number(n) => {
                out.push(4);
                u29(out, n.as_i64().unwrap() as u32 & 0x1fff_ffff);
            }
            Value::Array(a) => {
                out.push(9);
                u29(out, a.len() as u32 * 2 + 1);
                out.push(1);
                for i in a {
                    encode_amf(out, i);
                }
            }
            Value::Object(o) => {
                out.extend([10, 11, 1]);
                for (k, v) in o {
                    amf_string(out, k);
                    encode_amf(out, v);
                }
                out.push(1);
            }
            _ => panic!("unsupported fixture type"),
        }
    }
    fn battle_frame(param: &Value) -> Vec<u8> {
        let mut payload = vec![1];
        encode_amf(&mut payload, param);
        let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
        frame.extend(payload);
        frame
    }
    pub(crate) fn verify_battle_feed(mut command: impl FnMut(&str) -> Value) {
        let epoch = socket_tap::current_session();
        let lives: [Arc<()>; 5] = std::array::from_fn(|_| Arc::new(()));
        let deliver = |handle, kind| {
            observe(SocketTapEvent {
                session: epoch,
                handle,
                lifetime: lives[(handle - 100) as usize].clone(),
                kind,
            })
        };
        let barrier = || {
            let (tx, rx) = mpsc::channel();
            HUB.get()
                .unwrap()
                .as_ref()
                .unwrap()
                .input
                .tx
                .send(Message::Barrier(tx))
                .unwrap();
            rx.recv_timeout(Duration::from_secs(5)).unwrap();
        };
        let build = |cmd: &str, hp: u32| {
            let fields = json!({"_cmd":cmd,"battleUniqueId":"fixture","pmmList":[
                {"battleView":{"pmmId":12345,"slotId":0,"pmList":[{"pmSView":{"uqi":11,"r":100,"nm":"己方"}}]}},
                {"battleView":{"pmmId":-2002,"slotId":0,"pmList":[{"pmSView":{"uqi":22,"r":200,"nm":"敌方","c0":hp,"s0":110}}]}}]});
            battle_frame(&fields)
        };
        let login = crate::mypro_features::protocol::tests::login();
        let frame = build("2401", 100);
        deliver(100, open());
        deliver(100, chunk(SocketDirection::Send, 0, &login));
        deliver(100, chunk(SocketDirection::Recv, 0, &frame[..2]));
        deliver(100, chunk(SocketDirection::Recv, 2, &frame[2..]));
        barrier();
        let snapshot = command("battle.snapshot");
        assert!(snapshot.get("switches").is_some() && snapshot.get("battleId").is_some());
        assert_eq!(
            snapshot["details"]["enemy"]["pets"][0]["currentHp"], 100,
            "{snapshot}"
        );
        assert_eq!(command("packets.page")["rows"][0]["cmd"], "2401");
        {
            let _busy = HUB.get().unwrap().as_ref().unwrap().view.lock().unwrap();
            let same = command("battle.snapshot");
            assert_eq!(same["details"], snapshot["details"]);
            assert_eq!(same["switches"], snapshot["switches"]);
        }
        // An unrelated socket (including one without a handshake) cannot replace the opponent.
        deliver(101, open());
        deliver(101, chunk(SocketDirection::Recv, 0, &build("2401", 17)));
        deliver(101, SocketTapKind::Closed);
        barrier();
        assert_eq!(
            command("battle.snapshot")["details"]["enemy"]["pets"][0]["currentHp"],
            100
        );
        deliver(
            100,
            chunk(SocketDirection::Recv, frame.len() as u64 + 1, &frame),
        );
        barrier();
        assert!(command("battle.snapshot")["details"]["enemy"].is_null());
        deliver(102, open());
        deliver(102, chunk(SocketDirection::Send, 0, &login));
        let older = build("2411", 81);
        deliver(102, chunk(SocketDirection::Recv, 0, &older));
        deliver(100, SocketTapKind::Closed);
        barrier();
        assert_eq!(
            command("battle.snapshot")["details"]["enemy"]["pets"][0]["currentHp"],
            81
        );
        // A later socket finishes while the previous one still has a valid, identified stream.
        deliver(103, open());
        deliver(103, chunk(SocketDirection::Send, 0, &login));
        deliver(103, chunk(SocketDirection::Recv, 0, &build("2411", 61)));
        barrier();
        assert_eq!(
            command("battle.snapshot")["details"]["enemy"]["pets"][0]["currentHp"],
            61
        );
        deliver(103, SocketTapKind::Closed);
        deliver(
            102,
            chunk(SocketDirection::Recv, older.len() as u64, &older),
        );
        barrier();
        assert!(command("battle.snapshot")["details"]["enemy"].is_null());
        deliver(104, open());
        deliver(104, chunk(SocketDirection::Send, 0, &login));
        deliver(104, chunk(SocketDirection::Recv, 0, &build("2411", 77)));
        barrier();
        assert_eq!(
            command("battle.snapshot")["details"]["enemy"]["pets"][0]["currentHp"],
            77
        );
        // No Closed arrived and there are no further network events to wake the worker.
        drop(lives);
        let deadline = Instant::now() + Duration::from_secs(3);
        while !command("battle.snapshot")["details"]["enemy"].is_null() {
            assert!(
                Instant::now() < deadline,
                "idle worker retained a disconnected opponent"
            );
            std::thread::sleep(Duration::from_millis(8));
        }
    }

    fn tcp(session: u64, lifetime: &Arc<()>, kind: SocketTapKind) -> SocketTapEvent {
        SocketTapEvent {
            session,
            handle: u64::MAX - 2,
            lifetime: lifetime.clone(),
            kind,
        }
    }
    fn test_socket(hub: &Hub) -> impl Fn(u64, SocketTapKind) + '_ {
        let lifetime = Arc::new(());
        // Direct injection into this isolated worker keeps concurrent global-session tests independent.
        move |session, kind| {
            hub.input
                .tx
                .try_send(Message::Event(tcp(session, &lifetime, kind), 0))
                .unwrap()
        }
    }
    fn chunk(direction: SocketDirection, offset: u64, bytes: &[u8]) -> SocketTapKind {
        SocketTapKind::Data {
            direction,
            offset,
            bytes: bytes.to_vec(),
        }
    }
    fn open() -> SocketTapKind {
        SocketTapKind::Connected {
            host: "localhost".into(),
            port: 1234,
        }
    }
    fn unique_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "ruffle-packet-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn gap_stops_only_that_direction_until_next_connection() {
        let mut conn = Connection::opened(1, Weak::new());
        let mut messages = vec![];
        assert!(!conn.feed(SocketDirection::Recv, 0, b"<frag", |_| panic!("incomplete")));
        assert!(conn.feed(SocketDirection::Recv, 6, b"ment/>\0", |_| panic!("gap")));
        assert!(
            !conn.feed(SocketDirection::Recv, 13, b"<policy/>\0", |_| panic!(
                "no guessed boundary"
            ))
        );
        assert!(!conn.feed(SocketDirection::Send, 0, b"<fresh/>\0", |p| {
            messages.push(p.command)
        }));
        assert_eq!(messages, ["policy"]);
    }

    #[test]
    fn queue_overflow_counts_loss_without_waiting_or_retaining_bytes() {
        // The current epoch is supplied directly so a test resetting the process session cannot race this queue.
        let (tx, rx) = mpsc::sync_channel(1);
        let i = Ingress {
            tx,
            bytes: Arc::new(AtomicUsize::new(0)),
            dropped: Default::default(),
            stopped: AtomicBool::new(false),
        };
        let lifetime = Arc::new(());
        let e = || {
            tcp(
                socket_tap::current_session(),
                &lifetime,
                chunk(SocketDirection::Recv, 0, b"x"),
            )
        };
        i.observe(e());
        i.observe(e());
        assert_eq!(i.bytes.load(Ordering::Relaxed), 1);
        assert_eq!(i.dropped.load(Ordering::Relaxed), 1);
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn packet_page_and_logger_observe_retention_boundaries_and_reset() {
        let root = unique_dir();
        let hub = Arc::new(Hub::start(&root).unwrap());
        let epoch = u64::MAX - 1;
        let mut service = Packets {
            hub: Ok(hub.clone()),
            epoch,
            drop_base: 0,
            last_details: None,
        };
        let send = test_socket(&hub);
        send(epoch, open());
        send(epoch, chunk(SocketDirection::Send, 0, b"<before/>\0"));
        assert!(!root.exists());
        assert_eq!(
            service
                .command("packets.retain.set", &json!({"enabled":true}))
                .unwrap()["requested"],
            true
        );
        let frame = [0, 0, 0, 7, 0, 0x4e, 0x33, 255, 255, 255, 255];
        send(epoch, chunk(SocketDirection::Recv, 0, &frame[..2]));
        send(epoch, chunk(SocketDirection::Recv, 2, &frame[2..]));
        send(epoch, chunk(SocketDirection::Send, 10, b"<retained/>\0"));
        hub.input.tx.try_send(Message::Reset(epoch + 1)).unwrap();
        send(epoch, chunk(SocketDirection::Send, 22, b"<late/>\0"));
        send(epoch + 1, open());
        let mut chunked = frame.to_vec();
        chunked.extend(frame);
        chunked.extend(frame);
        send(epoch + 1, chunk(SocketDirection::Recv, 0, &chunked));
        hub.finish();

        let dir = root.join("logs");
        let paths: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect();
        assert_eq!(paths.len(), 2);
        let packet_file = paths
            .iter()
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("aola-recv-")
            })
            .unwrap();
        let text = fs::read_to_string(packet_file).unwrap();
        let rows: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["direction"], "接收");
        assert_eq!(rows[0]["cmd"], "joinOK");
        assert_eq!(rows[0]["length"], 11);
        assert_eq!(rows[0]["param"]["headerRoomId"], -1);
        assert!(rows[0]["timeUtc"].as_str().is_some());
        assert!(rows[0]["connectionPlayerId"].is_null());
        assert_eq!(rows[1]["direction"], "发送");
        assert_eq!(rows[1]["length"], 12);
        let audit_file = paths.iter().find(|p| *p != packet_file).unwrap();
        let audit = fs::read_to_string(audit_file).unwrap();
        let boundaries: Vec<Value> = audit
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(boundaries.len(), 2);
        assert_eq!(boundaries[0]["event"], "start");
        assert_eq!(
            boundaries[0]["packetFile"],
            packet_file.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(boundaries[0]["observedRows"], 1);
        assert_eq!(boundaries[1]["observedRows"], 3);
        assert_eq!(boundaries[1]["reason"], "sessionReset");
        assert_eq!(boundaries[1]["writtenRows"], 2);
        assert!(boundaries[1]["writeError"].is_null());
        service.epoch += 1;
        let first = service
            .command("packets.page", &json!({"limit":2}))
            .unwrap();
        assert_eq!(first["rows"][0]["seq"], 3);
        assert_eq!(first["nextAfter"], 2);
        assert_eq!(first["rows"][0]["socket"], (u64::MAX - 2).to_string());
        let second = service
            .command("packets.page", &json!({"after":2,"limit":2}))
            .unwrap();
        assert_eq!(second["rows"][0]["seq"], 1);
        assert!(second["nextAfter"].is_null());
        assert_eq!(
            service
                .command("packets.page", &json!({"exportableOnly":true}))
                .unwrap()["rows"],
            json!([])
        );
        assert_eq!(
            service
                .command("packets.export", &json!({"seq":1}))
                .unwrap_err(),
            "NotExportable"
        );
        for path in paths {
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(dir).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn failed_writer_can_open_new_recording() {
        let root = unique_dir();
        fs::create_dir(&root).unwrap();
        let blocked = root.join("logs");
        fs::write(&blocked, b"not a directory").unwrap();
        let mut worker = Worker::new(&root, Default::default(), Default::default());
        worker.retain(true);
        let status = worker.view.lock().unwrap().log.clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        while status.lock().unwrap().error.is_none() {
            assert!(Instant::now() < deadline, "log failure was not reported");
            thread::sleep(Duration::from_millis(5));
        }
        fs::remove_file(&blocked).unwrap();
        worker.retain(true);
        worker.retain(false);
        worker.closing.take().unwrap().join().unwrap();
        assert!(status.lock().unwrap().error.is_none());
        let files: Vec<_> = fs::read_dir(&blocked)
            .unwrap()
            .map(|r| r.unwrap().path())
            .collect();
        assert_eq!(files.len(), 2);
        for file in files {
            fs::remove_file(file).unwrap();
        }
        fs::remove_dir(blocked).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn audit_keeps_in_progress_state_and_reconciles_loss_on_shutdown() {
        let root = unique_dir();
        let ingress: Arc<AtomicU64> = Default::default();
        let mut worker = Worker::new(&root, Default::default(), ingress.clone());
        let epoch = u64::MAX - 1;
        let lifetime = Arc::new(());
        let login = crate::mypro_features::protocol::tests::login();
        worker.event(tcp(epoch, &lifetime, open()));
        worker.event(tcp(
            epoch,
            &lifetime,
            chunk(SocketDirection::Send, 0, &login),
        ));
        let side = |owner, uqi, hp| {
            json!({"battleView":{"pmmId":owner,"slotId":0,"pmList":[
            {"pmSView":{"uqi":uqi,"r":200,"c0":hp,"nick":"synthetic-private-name"}}]}})
        };
        let action =
            battle_frame(&json!({"_cmd":"2411","pmmList":[side(12345,11,50),side(-2,22,81)]}));
        worker.event(tcp(
            epoch,
            &lifetime,
            chunk(SocketDirection::Recv, 0, &action),
        ));
        worker.retain(true);
        let end = battle_frame(&json!({"_cmd":"2404"}));
        worker.event(tcp(
            epoch,
            &lifetime,
            chunk(SocketDirection::Recv, action.len() as u64, &end),
        ));
        worker.event(tcp(
            epoch,
            &lifetime,
            chunk(SocketDirection::Recv, 9999, b"x"),
        ));
        ingress.fetch_add(2, Ordering::Relaxed);
        worker.close_log("shutdown");
        worker.closing.take().unwrap().join().unwrap();
        let audit_file = fs::read_dir(root.join("logs"))
            .unwrap()
            .flatten()
            .map(|f| f.path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("aola-audit-")
            })
            .unwrap();
        let text = fs::read_to_string(audit_file).unwrap();
        assert!(
            !text.contains("synthetic-private-name")
                && !text.contains("pmmId")
                && !text.contains("uqi")
        );
        let audit: Vec<Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(audit.len(), 3);
        assert_eq!(audit[0]["battle"]["enemyPets"][0]["currentHp"], 81);
        assert_eq!(audit[1]["command"], "2404");
        assert_eq!(audit[1]["applied"], true);
        assert_eq!(audit[1]["battle"]["state"], "ended");
        assert!(audit[1]["battle"]["enemyPets"].is_null());
        assert_eq!(audit[2]["reason"], "shutdown");
        assert_eq!(audit[2]["writtenRows"], 1);
        assert_eq!(audit[2]["droppedChunks"], 2);
        assert_eq!(audit[2]["gaps"], 1);
        assert_eq!(audit[2]["disabledDirections"], 1);
        assert_eq!(audit[2]["droppedLogRows"], 0);
        assert!(audit[2]["writeError"].is_null());
        let logs = root.join("logs");
        for item in fs::read_dir(&logs).unwrap() {
            fs::remove_file(item.unwrap().path()).unwrap();
        }
        fs::remove_dir(logs).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn retained_pve_double_records_both_slots_after_a_single_battle_on_the_same_socket() {
        let root = unique_dir();
        let mut worker = Worker::new(&root, Default::default(), Default::default());
        let epoch = u64::MAX - 1;
        let lifetime = Arc::new(());
        let login = crate::mypro_features::protocol::tests::login();
        worker.event(tcp(epoch, &lifetime, open()));
        worker.event(tcp(
            epoch,
            &lifetime,
            chunk(SocketDirection::Send, 0, &login),
        ));
        worker.retain(true);
        let side = |owner, slot, uqi, hp| {
            json!({"battleView":{"pmmId":owner,"slotId":slot,
            "pmList":[{"pmSView":{"uqi":uqi,"r":124891,"c0":hp}}]}})
        };
        let own = side(12345, 0, 1, 80);
        let own2 = side(12345, 1, 2, 90);
        let left = side(-3, 10, 21, 100);
        let right = side(-4, 11, 22, 200);
        let events = [
            json!({"_cmd":"2401","pvp":false,"pt":0,"pmmList":[own,side(-2,11,20,777)]}),
            json!({"_cmd":"2404"}),
            json!({"_cmd":"2303","pt":1,"pmmList":[own,own2]}),
            json!({"_cmd":"2304","pt":0,"b":[right]}),
            json!({"_cmd":"2304","pt":0,"b":[left]}),
            json!({"_cmd":"2401","pvp":false,"pt":1,"pmmList":[own,left,own2,right]}),
            json!({"_cmd":"2423","pmmList":[side(-4,11,22,157)]}),
            json!({"_cmd":"2404"}),
        ];
        let mut offset = 0;
        for fields in &events {
            let frame = battle_frame(fields);
            worker.event(tcp(
                epoch,
                &lifetime,
                chunk(SocketDirection::Recv, offset, &frame),
            ));
            offset += frame.len() as u64;
        }
        worker.close_log("shutdown");
        worker.closing.take().unwrap().join().unwrap();
        let logs = root.join("logs");
        let raw_name = worker
            .view
            .lock()
            .unwrap()
            .log
            .lock()
            .unwrap()
            .file
            .clone()
            .unwrap();
        let read_rows = |name| {
            let path = logs.join(name);
            let rows: Vec<Value> = fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            fs::remove_file(path).unwrap();
            rows
        };
        let audit = read_rows(raw_name.replacen("aola-recv-", "aola-audit-", 1));
        let raw = read_rows(raw_name);
        fs::remove_dir(logs).unwrap();
        fs::remove_dir(root).unwrap();

        assert_eq!(raw.len(), events.len());
        assert_eq!(audit.len(), raw.len() + 2);
        assert_eq!(audit.last().unwrap()["writtenRows"], raw.len());
        let baseline = audit[0]["observedRows"].as_u64().unwrap();
        for (index, (packet, snapshot)) in raw.iter().zip(&audit[1..]).enumerate() {
            assert_eq!(packet["decoded"], true);
            assert_eq!(packet["cmd"], snapshot["command"]);
            assert_eq!(snapshot["observedRow"], baseline + index as u64 + 1);
        }
        for i in [2, 8] {
            assert_eq!(audit[i]["battle"]["state"], "ended");
            assert!(audit[i]["battle"]["enemyPets"].is_null());
        }
        assert!(audit[3]["battle"]["enemyPets"].is_null());
        assert_eq!(audit[1]["battle"]["enemyPets"][0]["currentHp"], 777);
        for (index, right_hp) in [(6, 200), (7, 157)] {
            let both = audit[index]["battle"]["enemyPets"].as_array().unwrap();
            assert_eq!(both.len(), 2);
            for (pet, (slot, hp)) in both.iter().zip([(10, 100), (11, right_hp)]) {
                assert_eq!(pet["raceId"], 124891);
                assert_eq!(pet["slotId"], slot);
                assert_eq!(pet["currentHp"], hp);
            }
        }
    }

    #[test]
    fn ring_evicts_by_count_and_memory() {
        let mut view = View::default();
        for _ in 0..=MAX_ROWS {
            view.push(json!({}), None, 4);
        }
        assert_eq!(view.rows.len(), MAX_ROWS);
        assert_eq!(view.dropped, 1);
        view.push(json!({}), None, MAX_ROW_BYTES);
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.bytes, MAX_ROW_BYTES);
    }

    #[test]
    fn dropped_close_waits_for_pending_bytes_and_releases_only_its_slot() {
        let epoch = 1;
        let mut worker = Worker::new(&unique_dir(), Default::default(), Default::default());
        let evt = |handle, lifetime: &Arc<()>, kind| SocketTapEvent {
            session: epoch,
            handle,
            lifetime: lifetime.clone(),
            kind,
        };
        let mut live: HashMap<_, _> = (1..=MAX_CONNECTIONS as u64)
            .map(|n| (n, Arc::new(())))
            .collect();
        for (handle, lease) in &live {
            worker.event(evt(*handle, lease, open()));
        }
        let owner = 1;
        worker.battle.owner = Some((owner, worker.connections[&owner].generation));
        worker.view.lock().unwrap().battle = Some(Arc::new(json!({"enemy":"current"})));
        let (tx, rx) = mpsc::sync_channel(1);
        let input = Ingress {
            tx,
            bytes: Default::default(),
            dropped: Default::default(),
            stopped: AtomicBool::new(false),
        };
        for victim in [2, owner] {
            let lease = live.remove(&victim).unwrap();
            input.observe(evt(
                victim,
                &lease,
                chunk(SocketDirection::Send, 0, b"<final/>\0"),
            ));
            input.observe(evt(victim, &lease, SocketTapKind::Closed)); // full, deliberately lost.
            drop(lease);
            worker.reap_closed();
            assert!(worker.connections.contains_key(&victim));
            assert!(worker.view.lock().unwrap().battle.is_some());
            let Message::Event(event, size) = rx.try_recv().unwrap() else {
                panic!()
            };
            input.bytes.fetch_sub(size, Ordering::Relaxed);
            worker.event(event);
            assert_eq!(
                worker.view.lock().unwrap().rows.back().unwrap().data["cmd"],
                "policy"
            );
            worker.reap_closed();
            assert!(!worker.connections.contains_key(&victim));
            assert_eq!(
                worker.view.lock().unwrap().battle.is_some(),
                victim != owner
            );
        }
        assert_eq!(input.dropped.load(Ordering::Relaxed), 2);
        // Fill the table again, lose a close, and admit the next connection immediately.
        for handle in [50, 51] {
            let lease = Arc::new(());
            worker.event(evt(handle, &lease, open()));
            live.insert(handle, lease);
        }
        assert_eq!(worker.connections.len(), MAX_CONNECTIONS);
        drop(live.remove(&3));
        let newest = Arc::new(());
        worker.event(evt(52, &newest, open()));
        assert_eq!(worker.connections.len(), MAX_CONNECTIONS);
        assert!(!worker.connections.contains_key(&3) && worker.connections.contains_key(&52));
    }

    #[test]
    fn missing_worker_still_provides_unknown_details() {
        let mut service = Packets {
            hub: Err("worker unavailable".into()),
            epoch: 0,
            drop_base: 0,
            last_details: None,
        };
        assert!(!service.available());
        assert_eq!(service.battle_details()["state"], "unknown");
    }

    #[test]
    fn export_contains_only_observed_business_parameters() {
        use crate::mypro_features::protocol::tests::{extension, login};
        let root = unique_dir();
        let hub = Arc::new(Hub::start(&root).unwrap());
        let epoch = u64::MAX - 1;
        let service = Packets {
            hub: Ok(hub.clone()),
            epoch,
            drop_base: 0,
            last_details: None,
        };
        let login = login();
        let send = test_socket(&hub);
        send(epoch, open());
        send(epoch, chunk(SocketDirection::Send, 0, &login));
        send(
            epoch,
            chunk(
                SocketDirection::Send,
                login.len() as u64,
                &extension(18, 42),
            ),
        );
        hub.finish();
        let page = service
            .command("packets.page", &json!({"exportableOnly":true}))
            .unwrap();
        assert_eq!(page["rows"].as_array().unwrap().len(), 1);
        let seq = &page["rows"][0]["seq"];
        let row = service
            .command("packets.export", &json!({"seq":seq}))
            .unwrap();
        let text = row["text"].as_str().unwrap();
        let exported: Value = serde_json::from_str(
            text.strip_prefix("#send=")
                .unwrap()
                .strip_suffix('|')
                .unwrap(),
        )
        .unwrap();
        assert_eq!(exported["id"], 42);
        assert_eq!(exported["cmd"], "fixture");
        assert!(!text.contains(":ext_seq;"));
        assert!(!text.contains("synthetic-secret"));
        assert_eq!(
            service
                .command("packets.export", &json!({"seq":1}))
                .unwrap_err(),
            "NotExportable"
        );
        assert!(!root.exists());
    }
}
