use super::protocol::{Direction, Packet};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const QUEUED_BYTES: usize = 8 * 1024 * 1024;
const FLUSH_INTERVAL: Duration = Duration::from_millis(200);

pub fn record(now: DateTime<Utc>, handle: u64, packet: &Packet) -> Value {
    let mut row = json!({
        "timeUtc": now.to_rfc3339_opts(SecondsFormat::Micros, true),
        "timeLocal": now.with_timezone(&Local).to_rfc3339_opts(SecondsFormat::Micros, false),
        "direction": if packet.direction == Direction::Send { "发送" } else { "接收" },
        "processId": std::process::id(), "socket": handle, "connectionPlayerId":packet.connection_player_id,
        "length": packet.length, "decoded": !matches!(packet.command.as_str(),"decode-error"|"unavailable"),
        "summary":packet.summary
    });
    if !packet.command.is_empty() {
        row["cmd"] = json!(packet.command);
    }
    if let Some(id) = packet.extension_id {
        row["extensionId"] = json!(id);
    }
    if let Some(sequence) = packet.outer_sequence {
        row["outerSeq"] = sequence.into();
    }
    if let Some(kind) = packet.message_type {
        row["type"] = kind.into();
    }
    if let Some(action) = packet.action {
        row["action"] = action.into();
    }
    if packet.action == Some(10016)
        || matches!(packet.command.as_str(), "logIn" | "logOK" | "logKO")
    {
        row["rawHex"] = json!("<登录帧已脱敏>");
    } else if let Some(param) = &packet.parameters {
        row["param"] = param.clone();
    }
    row
}

/// Only gameplay fields are needed to check the panel against the packet stream.
pub(super) fn battle_evidence(snapshot: Option<&Value>) -> Value {
    let Some(state) = snapshot else {
        return json!({"state":"unknown","enemyPets":null});
    };
    let pets = state["enemy"]["pets"].as_array().map(|pets| {
        pets.iter()
            .map(|pet| {
                let skills = pet["skills"].as_array().map(|items| {
                    items
                        .iter()
                        .map(|skill| json!({"id":skill["id"],"pp":skill["pp"],"cd":skill["cd"]}))
                        .collect::<Vec<_>>()
                });
                let loadout = pet["loadout"].as_object().map(|groups| {
                    groups
                        .iter()
                        .map(|(name, group)| {
                            let ids = group["items"].as_array().map(|items| {
                                items
                                    .iter()
                                    .filter_map(|item| item["id"].as_i64())
                                    .collect::<Vec<_>>()
                            });
                            (name.clone(), json!({"state":group["state"],"ids":ids}))
                        })
                        .collect::<serde_json::Map<_, _>>()
                });
                json!({"slotId":pet["slotId"],"raceId":pet["raceId"],"currentHp":pet["currentHp"],
               "maximumHp":pet["maximumHp"],"skills":skills,"loadout":loadout})
            })
            .collect::<Vec<_>>()
    });
    json!({"state":state["state"],"lastCommand":state["lastCommand"],"enemyPets":pets})
}

#[derive(Clone, Default)]
pub struct LogStatus {
    pub file: Option<String>,
    pub error: Option<String>,
    pub enabled: bool,
    pub pending: bool,
}

pub struct Logger {
    sender: mpsc::SyncSender<(String, Option<String>)>,
    finish: mpsc::Sender<Value>,
    job: JoinHandle<()>,
    queued_bytes: Arc<AtomicUsize>,
    pub dropped: Arc<AtomicU64>,
}

impl Logger {
    pub fn finished(&self) -> bool {
        self.job.is_finished()
    }

    pub fn start(
        root: &Path,
        status: Arc<Mutex<LogStatus>>,
        dropped: Arc<AtomicU64>,
        mut start: Value,
    ) -> io::Result<Self> {
        let (sender, rx) = mpsc::sync_channel::<(String, Option<String>)>(256);
        let (finish, finished) = mpsc::channel::<Value>();
        let queued_bytes = Arc::new(AtomicUsize::new(0));
        let pending = queued_bytes.clone();
        let dir = root.join("logs");
        let suffix = format!(
            "{}-{}.jsonl",
            Local::now().format("%Y%m%d-%H%M%S-%f"),
            std::process::id()
        );
        let file = format!("aola-recv-{suffix}");
        start["packetFile"] = json!(file);
        *status.lock().unwrap() = LogStatus {
            pending: true,
            ..Default::default()
        };
        let job = thread::Builder::new()
            .name("mypro-packet-log".into())
            .spawn(move || {
                let open = |name: &str| {
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(dir.join(name))
                };
                let mut audit = None;
                let mut written = 0u64;
                let write = || -> io::Result<()> {
                    fs::create_dir_all(&dir)?;
                    let mut evidence = BufWriter::new(open(&format!("aola-audit-{suffix}"))?);
                    writeln!(evidence, "{start}")?;
                    evidence.flush()?;
                    audit = Some(evidence);
                    let evidence = audit.as_mut().unwrap();
                    let dest = open(&file)?;
                    let mut writer = BufWriter::with_capacity(64 * 1024, dest);
                    *status.lock().unwrap() = LogStatus {
                        file: Some(file),
                        enabled: true,
                        ..Default::default()
                    };
                    let mut last = Instant::now();
                    let mut dirty = false;
                    loop {
                        match rx.recv_timeout(FLUSH_INTERVAL) {
                            Ok((line, checkpoint)) => {
                                writeln!(writer, "{line}")?;
                                written += 1;
                                if let Some(state) = &checkpoint {
                                    writeln!(evidence, "{state}")?;
                                }
                                pending.fetch_sub(
                                    line.len() + 1 + checkpoint.as_ref().map_or(0, String::len),
                                    Ordering::Relaxed,
                                );
                                dirty = true;
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => return writer.flush(),
                        }
                        if dirty && last.elapsed() >= FLUSH_INTERVAL {
                            writer.flush()?;
                            evidence.flush()?;
                            last = Instant::now();
                            dirty = false;
                        }
                    }
                };
                let error = write().err().map(|e| e.to_string());
                drop(rx);
                {
                    let mut state = status.lock().unwrap();
                    state.pending = false;
                    state.enabled = false;
                    state.error = error.clone();
                }
                if let (Some(mut audit), Ok(mut end)) = (audit, finished.recv()) {
                    // The buffered packet count is durable through flush only when writeError is null.
                    end["writtenRows"] = json!(written);
                    end["writeError"] = json!(error);
                    if let Err(e) = writeln!(audit, "{end}").and_then(|_| audit.flush()) {
                        status.lock().unwrap().error = Some(e.to_string());
                    }
                }
            })?;
        Ok(Self {
            sender,
            finish,
            job,
            queued_bytes,
            dropped,
        })
    }

    pub fn push(&self, row: &Value, checkpoint: Option<Value>) {
        let line = row.to_string();
        let checkpoint = checkpoint.map(|value| value.to_string());
        let size = line.len() + 1 + checkpoint.as_ref().map_or(0, String::len);
        if self
            .queued_bytes
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                n.checked_add(size).filter(|n| *n <= QUEUED_BYTES)
            })
            .is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if self.sender.try_send((line, checkpoint)).is_err() {
            self.queued_bytes.fetch_sub(size, Ordering::Relaxed);
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn close(self, end: Value) -> JoinHandle<()> {
        let _ = self.finish.send(end);
        drop(self.sender);
        self.job
    }
}
