//! Bounded, read-only observations for the native MyPro tools window.
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const LIMIT: usize = 512;
static LIVE: OnceLock<Mutex<Live>> = OnceLock::new();

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Default)]
struct Live {
    rows: VecDeque<Value>,
    dropped: u64,
    next_seq: u64,
    enabled: bool,
    session: String,
    battle_id: Option<u64>,
    battle_ended: bool,
    switches: u64,
    updated: Option<u64>,
}

impl Live {
    fn push_panel(&mut self, url: &str, cls: &str) {
        if !self.enabled {
            return;
        }
        self.next_seq += 1;
        if self.rows.len() == LIMIT {
            self.rows.pop_front();
            self.dropped += 1;
        }
        self.rows.push_back(json!({
            "seq": self.next_seq, "timeMs": now_ms(), "url": url, "cls": cls,
        }));
    }

    fn battle(&self) -> Value {
        json!({"state":if self.battle_id.is_none() {"unknown"} else if self.battle_ended {"ended"} else {"active"},
            "battleId":self.battle_id, "switches":self.battle_id.map(|_|self.switches), "updatedMs":self.updated,
            "source":"petfightnew/FightPanel + SwitchPm"})
    }
}

fn panel_asset_url(raw: &str) -> Option<&str> {
    let valid = |byte: u8| byte.is_ascii_alphanumeric() || b"/%.-_~".contains(&byte);
    if raw.is_empty() || raw.len() > 768 || !raw.bytes().all(valid) {
        return None;
    }

    // Examine percent escapes as well while keeping the original game path in the row.
    let decoded;
    let path = if raw.contains('%') {
        let mut bytes = Vec::with_capacity(raw.len());
        let mut rest = raw.as_bytes();
        while let Some((&byte, tail)) = rest.split_first() {
            rest = tail;
            if byte == b'%' {
                let (escape, tail) = rest.split_at_checked(2)?;
                let hi = char::from(escape[0]).to_digit(16)?;
                let lo = char::from(escape[1]).to_digit(16)?;
                let decoded = (hi * 16 + lo) as u8;
                if decoded == b'%' || !valid(decoded) {
                    return None;
                }
                bytes.push(decoded);
                rest = tail;
            } else {
                bytes.push(byte);
            }
        }
        decoded = bytes;
        decoded.as_slice()
    } else {
        raw.as_bytes()
    };
    const SENSITIVE: &[&[u8]] = &[
        b"login",
        b"auth",
        b"oauth",
        b"authorization",
        b"account",
        b"session",
        b"ticket",
        b"token",
        b"password",
        b"captcha",
        b"cookie",
    ];
    if path.starts_with(b"//")
        || path.split(|&b| b == b'/').any(|part| matches!(part, b"." | b".."))
        || path
            .split(|b| !b.is_ascii_alphanumeric())
            .any(|token| SENSITIVE.iter().any(|word| token.eq_ignore_ascii_case(word)))
    {
        return None;
    }
    Some(raw)
}

fn asset_event(kind: &str, raw: &str, cls: &str) {
    if !matches!(kind, "panel" | "panel-error")
        || cls.is_empty()
        || cls.len() > 192
        || !cls.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:$".contains(&b))
    {
        return;
    }
    if let (Some(live), Some(url)) = (LIVE.get(), panel_asset_url(raw)) {
        live.lock().unwrap().push_panel(url, cls);
    }
}

pub fn in_battle() -> bool {
    LIVE.get().is_some_and(|live| {
        let live = live.lock().unwrap();
        live.battle_id.is_some() && !live.battle_ended
    })
}

pub fn game_event(row: &Value) {
    let Some(state) = LIVE.get() else { return };
    let Some(op) = row["operation"].as_str() else {
        return;
    };
    let mut live = state.lock().unwrap();
    match op {
        "battle-enter" | "battle-exit" => {
            let Some(id) = row["battleId"].as_u64() else {
                return;
            };
            if op == "battle-enter" {
                live.battle_id = Some(id);
                live.switches = 0;
            } else if live.battle_id != Some(id) {
                return;
            }
            live.battle_ended = op == "battle-exit";
            live.updated = Some(now_ms());
            drop(live);
            crate::mypro_features::battle_event(op, id);
        }
        "pet-switch-completed" if live.battle_id.is_some() && !live.battle_ended => {
            live.switches += 1;
            live.updated = Some(now_ms());
        }
        _ => {}
    }
}

pub struct ToolsService {
    packets: crate::mypro_features::network::Packets,
}

impl ToolsService {
    pub fn new(root: PathBuf) -> Self {
        LIVE.get_or_init(Default::default);
        ruffle_core::mypro_observer::install(asset_event);
        Self {
            packets: crate::mypro_features::network::Packets::new(&root),
        }
    }

    pub fn set_session(&mut self, id: &str) {
        let mut live = LIVE.get().unwrap().lock().unwrap();
        if live.session != id {
            self.packets.reset();
            let enabled = live.enabled;
            *live = Live {
                session: id.to_owned(),
                enabled,
                ..Default::default()
            };
        }
    }

    pub fn capabilities(&self) -> Value {
        let mut caps = serde_json::Map::new();
        for key in [
            "panel_capture.toggle",
            "panel_capture.clear",
            "panel_capture.list",
            "battle.snapshot",
        ] {
            caps.insert(key.into(), json!({"available":true,"reason":""}));
        }
        for key in [
            "battle.details",
            "packets.page",
            "packets.intercept.set",
            "packets.clear",
            "packets.retain.set",
            "packets.export",
        ] {
            caps.insert(key.into(), json!({"available":self.packets.available(),"reason":if self.packets.available(){""}else{"抓包后台不可用"}}));
        }
        Value::Object(caps)
    }
    pub fn command(&mut self, name: &str, p: &Value) -> Option<Result<Value, String>> {
        Some(match name {
            "packets.page" | "packets.intercept.set" | "packets.clear" | "packets.retain.set"
            | "packets.export" => self.packets.command(name, p),
            "panel_capture.toggle" => p["enabled"]
                .as_bool()
                .ok_or_else(|| "InvalidArguments".to_string())
                .map(|enabled| {
                    let mut live = LIVE.get().unwrap().lock().unwrap();
                    live.enabled = enabled;
                    ruffle_core::mypro_observer::set_recording(enabled);
                    json!({"enabled": enabled})
                }),
            "panel_capture.clear" => {
                let mut live = LIVE.get().unwrap().lock().unwrap();
                live.rows.clear();
                live.dropped = 0;
                Ok(json!({"enabled": live.enabled}))
            }
            "panel_capture.list" => {
                let live = LIVE.get().unwrap().lock().unwrap();
                Ok(json!({"enabled": live.enabled, "rows": live.rows, "dropped": live.dropped}))
            }
            "battle.snapshot" => {
                let mut snapshot = LIVE.get().unwrap().lock().unwrap().battle();
                snapshot["details"] = self.packets.battle_details();
                Ok(snapshot)
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mypro_panel_capture_keeps_extensionless_paths() {
        for url in ["activity/panel", "pet/newpetpackage", "activity/demo~123.swf", "activity/p%61nel"] {
            assert_eq!(panel_asset_url(url), Some(url));
        }
        for url in [
            "", "https://aola.100bt.com/play/demo.swf", "//private/folder", "file:///secret",
            "activity/../demo", "activity/%2e%2e/demo", "activity/a?ticket=private",
            "activity/demo#private", "activity/bad path", "account/profile", "activity/Login",
            "activity/session-token", "activity/%6cogin", "activity/%256cogin", "activity/test%",
        ] {
            assert!(panel_asset_url(url).is_none(), "{url}");
        }
        assert_eq!(panel_asset_url(&"a".repeat(768)).unwrap().len(), 768);
        assert!(panel_asset_url(&"a".repeat(769)).is_none());
    }

    #[test]
    fn mypro_panel_capture_is_bounded_and_counts_discarded_rows() {
        let mut live = Live { enabled: true, ..Default::default() };
        for _ in 0..LIMIT + 3 {
            live.push_panel("activity/demo", "Panel");
        }
        assert_eq!(live.rows.len(), LIMIT);
        assert_eq!(live.dropped, 3);
        assert_eq!(live.rows[0]["seq"], 4);
        assert_eq!(live.rows[LIMIT - 1]["seq"], LIMIT + 3);
    }

    #[test]
    fn mypro_panel_capture_commands_preserve_the_independent_battle_feed() {
        let root = std::env::temp_dir().join(format!("mypro-panel-capture-{}-{}", std::process::id(), now_ms()));
        let mut service = ToolsService::new(root);
        let request = |service: &mut ToolsService, name: &str, data: Value| {
            service.command(name, &data).unwrap().unwrap()
        };
        service.set_session("capture-test");
        assert!(!ruffle_core::mypro_observer::recording());
        crate::mypro_features::network::tests::verify_battle_feed(|name| request(&mut service, name, json!({})));

        let invoke = |kind, url, cls| ruffle_core::mypro_observer::observe(kind, url, cls);
        invoke("panel", "activity/paused", "Inactive");
        assert_eq!(request(&mut service, "panel_capture.toggle", json!({"enabled": true}))["enabled"], true);
        assert!(ruffle_core::mypro_observer::recording());
        invoke("panel", "activity/demo", "mmo.demo.Panel");
        invoke("panel-error", "pet/newpetpackage", "PetPackagePanel");
        invoke("resource", "activity/unrelated", "Ignored");
        invoke("panel", "activity/demo?token=private", "Ignored");
        invoke("panel", "activity/demo", "bad class");
        let rows = request(&mut service, "panel_capture.list", Value::Null);
        assert_eq!(rows["enabled"], true);
        assert_eq!(rows["rows"].as_array().unwrap().len(), 2);
        assert_eq!(rows["rows"][0]["url"], "activity/demo");
        assert_eq!(rows["rows"][0]["cls"], "mmo.demo.Panel");
        assert_eq!(rows["rows"][0]["seq"], 1);
        assert!(rows["rows"][0]["timeMs"].as_u64().is_some());
        assert_eq!(rows["rows"][1]["url"], "pet/newpetpackage");
        assert_eq!(rows["rows"][1]["cls"], "PetPackagePanel");

        request(&mut service, "panel_capture.clear", Value::Null);
        assert!(ruffle_core::mypro_observer::recording());
        assert!(request(&mut service, "panel_capture.list", Value::Null)["rows"].as_array().unwrap().is_empty());
        invoke("panel", "activity/demo", "NextPanel");
        request(&mut service, "panel_capture.toggle", json!({"enabled": false}));
        invoke("panel", "activity/paused", "Ignored");
        assert!(!ruffle_core::mypro_observer::recording());
        let rows = request(&mut service, "panel_capture.list", Value::Null);
        assert_eq!(rows["rows"].as_array().unwrap().len(), 1);
        assert_eq!(rows["rows"][0]["seq"], 3);
        assert!(service.command("capture.export", &Value::Null).is_none());
    }
}
