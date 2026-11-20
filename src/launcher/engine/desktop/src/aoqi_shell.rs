//! State and command routing for the recovered aoqi.WindowChrome contract.
//! Window operations are returned to the event loop, never performed under an
//! ActionScript/player lock. Game/account behavior remains in the recovered AS.

use crate::aoqi_storage::Storage;
use serde_json::{json, Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum ShellAction {
    ShowPanel { tab: Option<String> },
    HidePanel,
    FocusGame,
    PanelMessage(Value),
    Dispatch { kind: &'static str, payload: Value },
    ShowDialog(Value),
    CloseDialog { key: String },
    ConfirmPanel { request: Value, message: String },
    Loading { message: String, visible: bool, animated: bool },
    SizeContent { width: f64, height: f64, center: bool },
    CenterWindow,
    PrepareExit,
    BeginExit,
    FadeExit { seconds: f64 },
    UpdateCheck,
}

#[derive(Default)]
pub struct Shell {
    state: Map<String, Value>,
    commands: Vec<Value>,
    dialogs: Vec<Value>,
    panel_visible: bool,
    selected_account: String,
    serial: u64,
}

impl Shell {
    pub fn call(
        &mut self,
        name: &str,
        args: &[Value],
        storage: &Storage,
    ) -> Result<(Value, Vec<ShellAction>), String> {
        let mut actions = Vec::new();
        let result = match name {
            "shell.drainCommands" => Value::Array(std::mem::take(&mut self.commands)),
            "shell.panelSnapshot" => Value::Object(self.state.clone()),
            "shell.panelPrefs" => self.preferences(args, storage)?,
            "shell.panelExecute" => {
                let action = text(args, 0)?;
                let value = decoded(args.get(1).ok_or("Missing panel action value")?)?;
                self.enqueue(json!({"action":action,"value":value}), &mut actions)?;
                Value::Null
            }
            "shell.shellState" => {
                let value = decoded(args.first().ok_or("Missing shell state")?)?;
                let patch = value.as_object().ok_or("Shell state must be an object")?;
                for (key, value) in patch {
                    self.state.insert(key.clone(), value.clone());
                }
                actions.push(ShellAction::PanelMessage(json!({"type":"state","value":value})));
                if patch.keys().any(|key| matches!(key.as_str(), "scale" | "quality" | "frameRate" | "timeScale" | "muted" | "accounts")) {
                    actions.push(ShellAction::PanelMessage(self.platform_state()));
                }
                Value::Null
            }
            "shell.shellCommand" => {
                let value = decoded(args.first().ok_or("Missing shell command")?)?;
                self.shell_command(&value, &mut actions)?;
                Value::Null
            }
            "shell.showDialog" => {
                let value = decoded(args.first().ok_or("Missing dialog")?)?;
                let key = value.get("key").and_then(Value::as_str).ok_or("Dialog key is required")?;
                if !value.get("labels").is_some_and(Value::is_array) || value.get("token").is_none() {
                    return Err("Dialog labels and token are required".into());
                }
                self.dialogs.retain(|dialog| dialog["key"] != key);
                self.dialogs.push(value.clone());
                actions.push(ShellAction::ShowDialog(value));
                Value::Null
            }
            "shell.dialogCommand" => self.dialog_command(text(args, 0)?, args.get(1).and_then(Value::as_str).unwrap_or("state"), &mut actions)?,
            "shell.loading" => {
                actions.push(ShellAction::Loading {
                    message: text(args, 0)?.to_owned(),
                    visible: boolean(args, 1, false)?,
                    animated: boolean(args, 2, true)?,
                });
                Value::Null
            }
            "shell.sizeContent" => {
                let width = number(args, 0)?;
                let height = number(args, 1)?;
                if width <= 0.0 || height <= 0.0 {
                    return Err("Content dimensions must be positive".into());
                }
                actions.push(ShellAction::SizeContent { width, height, center: boolean(args, 2, false)? });
                Value::Bool(true)
            }
            "shell.centerWindow" => { actions.push(ShellAction::CenterWindow); Value::Null }
            "shell.prepareExit" => {
                self.close_dialogs(&mut actions);
                self.panel_visible = false;
                actions.push(ShellAction::HidePanel);
                actions.push(ShellAction::PrepareExit);
                Value::Null
            }
            "shell.beginExit" => { actions.push(ShellAction::BeginExit); Value::Null }
            "shell.fadeExit" => {
                let seconds = number(args, 0)?;
                if seconds < 0.0 { return Err("Fade duration must not be negative".into()); }
                actions.push(ShellAction::FadeExit { seconds });
                Value::Null
            }
            "shell.updateCheck" => { actions.push(ShellAction::UpdateCheck); Value::Null }
            "shell.updateState" => json!({"available":false,"state":"unavailable","message":"重建版本尚未配置更新源"}),
            // The parent provides actual observed window, payload and memory
            // values. Returning zeroes here would fabricate measurements.
            "shell.inspect" | "shell.payloadState" | "shell.processMemory" | "shell.rendererMemory" => {
                return Err(format!("{name} requires the parent runtime's observed state"));
            }
            _ => return Err(format!("Unknown shell bridge method: {name}")),
        };
        Ok((result, actions))
    }

    fn preferences(&self, args: &[Value], storage: &Storage) -> Result<Value, String> {
        let operation = text(args, 0)?;
        let key = text(args, 1)?;
        // Keys are opaque JSON keys, never paths. Storage performs atomic writes
        // within the application root and preserves failures for the AS caller.
        let path = json!("native-panel-prefs.json");
        let raw = storage.call("storage.readText", &[path.clone()])?;
        let mut values: Map<String, Value> = match raw {
            Value::Null => Map::new(),
            Value::String(text) => serde_json::from_str(&text).map_err(|error| format!("Cannot read panel preferences: {error}"))?,
            _ => return Err("Invalid panel preference storage response".into()),
        };
        match operation {
            "read" => Ok(values.get(key).cloned().unwrap_or(Value::Null)),
            "write" => {
                values.insert(key.to_owned(), Value::String(text(args, 2)?.to_owned()));
                storage.call("storage.writeText", &[path, json!(Value::Object(values).to_string()), Value::Bool(false)])
            }
            _ => Err(format!("Unknown panel preference operation: {operation}")),
        }
    }

    fn shell_command(&mut self, command: &Value, actions: &mut Vec<ShellAction>) -> Result<(), String> {
        let action = command.get("action").and_then(Value::as_str).ok_or("Shell command action is required")?;
        match action {
            "settings" | "panel-about" | "accounts" | "panel" => {
                let tab = match action { "settings" => Some("settings"), "panel-about" => Some("about"), "accounts" => Some("accounts"), _ => None };
                self.panel_visible = true;
                actions.push(ShellAction::ShowPanel { tab: tab.map(str::to_owned) });
                self.enqueue(json!({"action":"panelVisible","value":true}), actions)?;
            }
            "focus-game" => {
                self.panel_visible = false;
                actions.push(ShellAction::HidePanel);
                actions.push(ShellAction::FocusGame);
                self.enqueue(json!({"action":"panelVisible","value":false}), actions)?;
            }
            _ => return Err(format!("Unknown native shell command: {action}")),
        }
        Ok(())
    }

    fn enqueue(&mut self, mut command: Value, actions: &mut Vec<ShellAction>) -> Result<(), String> {
        let object = command.as_object_mut().ok_or("Command must be an object")?;
        if !object.get("action").is_some_and(Value::is_string) {
            return Err("Command action is required".into());
        }
        self.serial += 1;
        object.entry("id").or_insert_with(|| json!(self.serial));
        object.entry("sentAt").or_insert_with(|| json!(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64));
        self.commands.push(command);
        if !actions.iter().any(|action| matches!(action, ShellAction::Dispatch { kind: "shell-commands", .. })) {
            actions.push(ShellAction::Dispatch { kind: "shell-commands", payload: Value::Null });
        }
        Ok(())
    }

    /// Pass the original WebEvent message payload here. The recovered panel
    /// currently sends JSON strings; object messages are accepted as well.
    pub fn panel_message(&mut self, message: &Value) -> Result<Vec<ShellAction>, String> {
        let message = decoded(message)?;
        let mut actions = Vec::new();
        match message.get("type").and_then(Value::as_str) {
            Some("ready") => {
                actions.push(ShellAction::PanelMessage(json!({"type":"bootstrap","version":"1.0.3 纹理池与占用测试版"})));
                actions.push(ShellAction::PanelMessage(json!({"type":"window","mode":"overlay"})));
                actions.push(ShellAction::PanelMessage(self.platform_document()));
                actions.push(ShellAction::PanelMessage(self.platform_state()));
                actions.push(ShellAction::PanelMessage(json!({"type":"state","value":self.state})));
                self.enqueue(json!({"action":"panel-refresh","value":true}), &mut actions)?;
                self.enqueue(json!({"action":"panelVisible","value":self.panel_visible}), &mut actions)?;
            }
            Some("close") => {
                self.panel_visible = false;
                actions.push(ShellAction::HidePanel);
                actions.push(ShellAction::FocusGame);
                self.enqueue(json!({"action":"panelVisible","value":false}), &mut actions)?;
            }
            Some("command") => self.enqueue(message.clone(), &mut actions)?,
            Some("platform-event") => self.platform_event(&message["value"], &mut actions)?,
            Some("confirm") => {
                actions.push(ShellAction::ConfirmPanel {
                    request: message.get("request").cloned().ok_or("Confirmation request ID is required")?,
                    message: message.get("message").and_then(Value::as_str).ok_or("Confirmation message is required")?.to_owned(),
                });
            }
            _ => return Err("Unknown panel message type".into()),
        }
        Ok(actions)
    }

    pub fn panel_confirm(&self, request: Value, confirmed: bool) -> ShellAction {
        ShellAction::PanelMessage(json!({"type":"confirm-result","request":request,"confirmed":confirmed}))
    }

    fn platform_event(&mut self, event: &Value, actions: &mut Vec<ShellAction>) -> Result<(), String> {
        let id = event.get("id").and_then(Value::as_str).ok_or("Platform event ID is required")?;
        let value = event.get("value").cloned().unwrap_or(Value::Null);
        let action = match id {
            "settings.scale" => "scale",
            "settings.quality" => "quality",
            "settings.frameRate" => "frameRate",
            "settings.timeScale" => "timeScale",
            "settings.muted" => "muted",
            "about.refresh" => "refresh",
            "about.update" => "checkUpdate",
            "accounts.web-login" => "login",
            "accounts.cancel" => "accounts-cancel",
            "accounts.entries" => {
                self.selected_account = value.as_str().ok_or("Account selection must be a string")?.to_owned();
                actions.push(ShellAction::PanelMessage(self.platform_state()));
                return Ok(());
            }
            "accounts.login" | "accounts.remove" => {
                if self.selected_account.is_empty() { return Err("请先选择账号".into()); }
                let action = if id == "accounts.login" { "accounts-login" } else { "accounts-remove" };
                return self.enqueue(json!({"action":action,"value":self.selected_account}), actions);
            }
            _ => return Err(format!("Unknown platform control: {id}")),
        };
        self.enqueue(json!({"action":action,"value":value}), actions)
    }

    fn dialog_command(&mut self, key: &str, command: &str, actions: &mut Vec<ShellAction>) -> Result<Value, String> {
        match command {
            "state" => Ok(self.dialogs.iter().rev().find(|dialog| dialog["key"] == key).map_or(json!({"visible":false}), |dialog| json!({"visible":true,"key":dialog["key"],"token":dialog["token"]}))),
            "closeActive" => {
                let closed = if let Some(dialog) = self.dialogs.pop() {
                    actions.push(ShellAction::CloseDialog { key: dialog["key"].as_str().unwrap_or_default().to_owned() });
                    true
                } else { false };
                Ok(json!({"visible":false,"closed":closed}))
            }
            "closeAll" => {
                let closed = !self.dialogs.is_empty();
                self.close_dialogs(actions);
                Ok(json!({"visible":false,"closed":closed}))
            }
            _ => Err(format!("Unknown dialog command: {command}")),
        }
    }

    fn close_dialogs(&mut self, actions: &mut Vec<ShellAction>) {
        for dialog in self.dialogs.drain(..) {
            actions.push(ShellAction::CloseDialog { key: dialog["key"].as_str().unwrap_or_default().to_owned() });
        }
    }

    pub fn dialog_result(&mut self, key: &str, token: &Value, index: i64) -> Vec<ShellAction> {
        let Some(position) = self.dialogs.iter().position(|dialog| dialog["key"] == key && &dialog["token"] == token) else { return Vec::new() };
        let dialog = &self.dialogs[position];
        let length = dialog["labels"].as_array().map_or(0, Vec::len) as i64;
        if index < -1 || index >= length { return Vec::new(); }
        self.dialogs.remove(position);
        vec![ShellAction::Dispatch { kind: "dialog-action", payload: json!({"key":key,"token":token,"index":index}) }]
    }

    fn platform_document(&self) -> Value {
        let select = |id: &str, label: &str, options: Vec<Value>| json!({"id":id,"type":"field","kind":"picker","label":label,"options":options,"enabled":false});
        let number_options = |values: &[f64]| values.iter().map(|value| json!({"value":value,"label":value.to_string()})).collect();
        json!({"type":"platform","leading":[
            {"id":"settings","title":"设置","sections":[{"id":"settings.general","title":"游戏窗口","blocks":[
                {"id":"settings.fixed-profile","type":"note","text":"滤镜提交测试配置：内部渲染固定 1200×700，DirectX 12，高画质（4×抗锯齿），FSR 关闭，60 FPS 目标（由游戏脚本控制），1 倍速，100% 缩放，静音。全屏仅缩放显示。"},
                {"id":"settings.options","type":"form","layout":"stacked","items":[
                    select("settings.scale","窗口缩放",number_options(&[0.5,0.75,0.9,1.0,1.25,1.5,1.75,2.0,2.25,2.5,3.0])),
                    select("settings.quality","画质",vec![json!({"value":"game","label":"游戏默认"}),json!({"value":"low","label":"低"}),json!({"value":"medium","label":"中"}),json!({"value":"high","label":"高"})]),
                    select("settings.frameRate","帧率",vec![json!({"value":0,"label":"自动"}),json!({"value":25,"label":"25"}),json!({"value":30,"label":"30"}),json!({"value":40,"label":"40"}),json!({"value":50,"label":"50"}),json!({"value":60,"label":"60"})]),
                    select("settings.timeScale","时间倍率",number_options(&[1.0,2.0,3.0,4.0,5.0,6.0,7.0,8.0,9.0,10.0])),
                    {"id":"settings.muted","type":"field","kind":"toggle","label":"静音","value":false,"enabled":false}
                ]}
            ]}]},
            {"id":"accounts","title":"账号","sections":[{"id":"accounts.manager","title":"本地账号","blocks":[
                {"id":"accounts.notice","type":"note","text":"等待账号状态","tone":"normal"},
                {"id":"accounts.entries","type":"table","selectable":true,"empty":"暂无本地账号","columns":[{"id":"account","title":"账号"},{"id":"duoduoId","title":"多多号"},{"id":"status","title":"状态"}],"rows":[]},
                {"id":"accounts.actions","type":"actions","items":[
                    {"id":"accounts.login","type":"action","label":"登录选中账号","enabled":false},
                    {"id":"accounts.remove","type":"action","label":"删除本地记录","confirm":"删除选中的本地账号记录？","enabled":false},
                    {"id":"accounts.web-login","type":"action","label":"打开官方登录页"},
                    {"id":"accounts.cancel","type":"action","label":"取消登录","enabled":false}
                ]}
            ]}]}
        ],"trailing":[{"id":"about","title":"关于","sections":[{"id":"about.app","title":"Air 奥奇传说","blocks":[
            {"id":"about.version","type":"note","text":"1.0.3 纹理池与占用测试版"},
            {"id":"about.memory","type":"note","text":"资源分类诊断启动后每 5 秒保存一次；统计为资源描述符估算，不等于驱动显存驻留。"},
            {"id":"about.actions","type":"actions","items":[{"id":"about.refresh","type":"action","label":"重新加载游戏"},{"id":"about.update","type":"action","label":"检查更新"}]}
        ]}]}]})
    }

    fn platform_state(&self) -> Value {
        let mut changes = Vec::new();
        for key in ["scale", "quality", "frameRate", "timeScale", "muted"] {
            changes.push(json!({"id":format!("settings.{key}"),"set":{"value":self.state.get(key),"enabled":false}}));
        }
        let accounts = self.state.get("accounts");
        let busy = accounts.and_then(|v| v["busy"].as_bool()).unwrap_or(false);
        let rows: Vec<Value> = accounts.and_then(|v| v["entries"].as_array()).into_iter().flatten().map(|entry| json!({"id":entry["key"],"values":[entry["account"],entry["duoduoId"],entry["status"]]})).collect();
        let selected = rows.iter().any(|row| row["id"] == self.selected_account);
        let notice = accounts.map(|v| ["error","storageError","notice"].into_iter().filter_map(|key| v[key].as_str()).filter(|text| !text.is_empty()).collect::<Vec<_>>().join("\n")).unwrap_or_else(|| "等待账号状态".into());
        changes.push(json!({"id":"accounts.entries","set":{"rows":rows,"selection":self.selected_account,"enabled":accounts.is_some()&&!busy}}));
        changes.push(json!({"id":"accounts.notice","set":{"text":notice}}));
        for id in ["accounts.login", "accounts.remove"] { changes.push(json!({"id":id,"set":{"enabled":selected&&!busy}})); }
        changes.push(json!({"id":"accounts.cancel","set":{"enabled":busy}}));
        changes.push(json!({"id":"accounts.web-login","set":{"enabled":accounts.is_some()&&!busy}}));
        json!({"type":"platformState","changes":changes})
    }
}

fn decoded(value: &Value) -> Result<Value, String> {
    match value {
        Value::String(text) => serde_json::from_str(text).map_err(|error| format!("Invalid bridge JSON: {error}")),
        value => Ok(value.clone()),
    }
}

fn text(args: &[Value], index: usize) -> Result<&str, String> {
    args.get(index).and_then(Value::as_str).ok_or_else(|| format!("Argument {index} must be a string"))
}

fn number(args: &[Value], index: usize) -> Result<f64, String> {
    args.get(index).and_then(Value::as_f64).filter(|number| number.is_finite()).ok_or_else(|| format!("Argument {index} must be a finite number"))
}

fn boolean(args: &[Value], index: usize, default: bool) -> Result<bool, String> {
    args.get(index).map_or(Ok(default), |value| value.as_bool().ok_or_else(|| format!("Argument {index} must be a boolean")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_storage() -> Storage {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        Storage::new(std::env::temp_dir().join(format!("aoqi-shell-test-{}-{unique}", std::process::id()))).unwrap()
    }

    #[test]
    fn queued_panel_command_is_delivered_once_with_original_id() {
        let storage = temporary_storage();
        let mut shell = Shell::default();
        let actions = shell.panel_message(&json!({"type":"command","id":17,"action":"quality","value":"high"})).unwrap();
        assert!(actions.iter().any(|action| matches!(action, ShellAction::Dispatch { kind: "shell-commands", .. })));
        let (commands, _) = shell.call("shell.drainCommands", &[], &storage).unwrap();
        assert_eq!(commands.as_array().unwrap().len(), 1);
        assert_eq!(commands[0]["id"], 17);
        assert_eq!(commands[0]["action"], "quality");
        assert_eq!(commands[0]["value"], "high");
        assert!(commands[0]["sentAt"].is_u64());
        assert_eq!(shell.call("shell.drainCommands", &[], &storage).unwrap().0, json!([]));
        std::fs::remove_dir(storage.root()).unwrap();
    }

    #[test]
    fn replaced_dialog_rejects_stale_results_and_duplicate_completion() {
        let storage = temporary_storage();
        let mut shell = Shell::default();
        for token in [10, 11] {
            shell.call("shell.showDialog", &[json!({"key":"recover","token":token,"labels":["继续","取消"]})], &storage).unwrap();
        }
        assert!(shell.dialog_result("recover", &json!(10), 0).is_empty());
        let result = shell.dialog_result("recover", &json!(11), 1);
        assert!(matches!(&result[0], ShellAction::Dispatch { kind: "dialog-action", payload } if payload["index"] == 1));
        assert!(shell.dialog_result("recover", &json!(11), 1).is_empty());
        assert_eq!(shell.call("shell.dialogCommand", &[json!("recover"),json!("state")], &storage).unwrap().0, json!({"visible":false}));
        std::fs::remove_dir(storage.root()).unwrap();
    }

    #[test]
    fn preferences_survive_new_shell_and_corruption_is_not_overwritten() {
        let storage = temporary_storage();
        let mut shell = Shell::default();
        shell.call("shell.panelPrefs", &[json!("write"),json!("../opaque-key"),json!("中文设置")], &storage).unwrap();
        let mut reopened = Shell::default();
        assert_eq!(reopened.call("shell.panelPrefs", &[json!("read"),json!("../opaque-key")], &storage).unwrap().0, json!("中文设置"));
        storage.call("storage.writeText", &[json!("native-panel-prefs.json"),json!("broken"),json!(false)]).unwrap();
        assert!(reopened.call("shell.panelPrefs", &[json!("write"),json!("key"),json!("replacement")], &storage).is_err());
        assert_eq!(storage.call("storage.readText", &[json!("native-panel-prefs.json")]).unwrap(), json!("broken"));
        std::fs::remove_file(storage.root().join("native-panel-prefs.json")).unwrap();
        std::fs::remove_dir(storage.root()).unwrap();
    }
}
