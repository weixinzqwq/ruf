use crate::player::PlayerController;
use ruffle_core::external::Value as Flash;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::time::{Duration, Instant};

thread_local! {
    static THEME: RefCell<Theme> = RefCell::default();
}

#[derive(Default)]
struct Theme {
    session: Option<i64>,
    desired: u8,
    applied: u8,
    request: i32,
    status: Option<String>,
    message: String,
    next: Option<Instant>,
}

impl Theme {
    fn snapshot(&self) -> Value {
        json!({"themeId":self.desired,"appliedThemeId":self.applied,"requestId":self.request,
            "status":self.status.as_deref().unwrap_or("NotReady"),
            "message":self.message})
    }

    fn accept(&mut self, answer: Option<Flash>) {
        let mut status = "NotReady";
        let mut message = String::new();
        let result = match answer {
            Some(Flash::String(s)) => serde_json::from_str::<Value>(&s).ok(),
            _ => None,
        };
        if let Some(result) = &result {
            if result["requestId"].as_i64() == Some(i64::from(self.request))
                && result["themeId"].as_u64() == Some(u64::from(self.desired))
            {
                status = result["status"].as_str().unwrap_or("Failed");
                message = result["message"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(240)
                    .collect();
            } else {
                status = "Failed";
                message = "游戏返回了其他主题请求的状态".into();
            }
        }
        match status {
            "Applied" => {
                self.applied = self.desired;
                self.next = None;
            }
            "Loading" | "NotReady" | "WaitingBattleEnd" => {
                self.next = Some(Instant::now() + Duration::from_millis(500))
            }
            _ => {
                status = "Failed";
                self.next = None;
            }
        }
        self.status = Some(status.to_owned());
        self.message = message;
    }
}

fn drive(player: &PlayerController, id: u8, request: i32) {
    let answer = player.get().and_then(|mut p| {
        p.try_call_internal_interface(
            "onAolaThemeApply",
            [Flash::from(u32::from(id)), Flash::from(request)],
        )
    });
    THEME.with_borrow_mut(|s| s.accept(answer));
}

pub fn sync(player: &PlayerController) {
    let now = Instant::now();
    let current = player.session_id();
    let work = THEME.with_borrow_mut(|s| {
        if current != s.session {
            *s = Theme {
                session: current,
                ..Default::default()
            };
        }
        s.next
            .filter(|deadline| *deadline <= now)
            .map(|_| (s.desired, s.request))
    });
    if let Some((id, request)) = work {
        drive(player, id, request);
    }
}

pub fn command(
    player: &PlayerController,
    name: &str,
    payload: &Value,
) -> Option<Result<Value, String>> {
    match name {
        "game.panels" => Some(Ok(json!({"panels":[
            {"id":"theme-settings","name":"控制栏装扮"},{"id":"summoner","name":"召唤师"}]}))),
        "game.panel.open" => {
            let id = payload["id"].as_str().unwrap_or("");
            if !matches!(id, "theme-settings" | "summoner") {
                return Some(Err("InvalidArguments: 未知游戏面板".into()));
            }
            let result = player.get().and_then(|mut p| {
                p.try_call_internal_interface("onAolaOpenPanel", [Flash::from(id.to_owned())])
            });
            let status = match result.as_ref() {
                Some(Flash::String(value))
                    if matches!(value.as_str(), "requested" | "blocked" | "failed") =>
                {
                    value.as_str()
                }
                _ => "unavailable",
            };
            Some(Ok(json!({"id":id,"status":status,"message":""})))
        }
        "theme.read" => {
            sync(player);
            if THEME.with_borrow(|s| s.next.is_none() && s.status.as_deref() != Some("Failed")) {
                let actual = player
                    .get()
                    .and_then(|mut p| p.try_call_internal_interface("onAolaThemeRead", []));
                THEME.with_borrow_mut(|s| match actual {
                    Some(Flash::Number(n)) if (0.0..=10.0).contains(&n) && n.fract() == 0.0 => {
                        s.applied = n as u8;
                        s.desired = s.applied;
                        s.status = Some("Applied".into());
                    }
                    _ => s.status = Some("NotReady".into()),
                });
            }
            Some(Ok(THEME.with_borrow(Theme::snapshot)))
        }
        "theme.apply" => {
            let Some(id) = payload["themeId"].as_u64().filter(|id| *id <= 10) else {
                return Some(Err("InvalidArguments: 主题编号应为 0 到 10".into()));
            };
            let Some(current) = player.session_id() else {
                return Some(Err("NotInGame".into()));
            };
            let id = id as u8;
            let request = THEME.with_borrow_mut(|s| {
                if s.session != Some(current) {
                    *s = Theme {
                        session: Some(current),
                        ..Default::default()
                    };
                }
                s.desired = id;
                s.request = s.request.saturating_add(1);
                s.request
            });
            drive(player, id, request);
            Some(Ok(THEME.with_borrow(Theme::snapshot)))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_theme_confirms_only_the_current_request_after_battle() {
        let mut s = Theme {
            desired: 10,
            request: 4,
            applied: 2,
            ..Default::default()
        };
        for phase in ["WaitingBattleEnd", "Loading", "Applied"] {
            s.accept(Some(Flash::String(
                json!({"status":phase,"themeId":10,"requestId":4}).to_string(),
            )));
            assert_eq!(s.applied, if phase == "Applied" { 10 } else { 2 });
            assert_eq!(s.next.is_some(), phase != "Applied");
        }
        s.request = 5;
        s.desired = 3;
        s.accept(Some(Flash::String(
            json!({"status":"Applied","themeId":10,"requestId":4}).to_string(),
        )));
        assert_eq!(s.status.as_deref(), Some("Failed"));
        assert_eq!(s.applied, 10);
    }
}
