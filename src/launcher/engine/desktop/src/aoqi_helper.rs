//! HelperCore JSON protocol and independent no-login helper state machine.
//! The protocol/continuations are recovered from HelperCore/HelperMachine.as.
//! loginRecovery.plan follows native 0x140377b40 and its exact PE string constants.
//! The original battle/daily/recording machine is not represented as implemented.

use serde_json::{json, Value};
use std::collections::BTreeMap;

const PREFS_KEY: &str = "reconstructed.helper.preferences";
const LOGIN_PANELS: [&str; 3] = [
    "mmo.login.login.panel::LoginWithUserInfoPanel",
    "mmo.login.login.panel::LoginWithoutUserInfoPanel",
    "mmo.login.panel::InvalidPanel_res",
];

#[derive(Default)]
pub struct Helper {
    next_handle: u32,
    machines: BTreeMap<u32, Machine>,
}

impl Helper {
    pub fn call(&mut self, request: &str) -> String {
        let result = serde_json::from_str::<Value>(request)
            .map_err(|e| format!("Invalid helper JSON: {e}"))
            .and_then(|request| self.dispatch(request));
        match result {
            Ok(value) => json!({"value":value}).to_string(),
            Err(error) => json!({"error":error}).to_string(),
        }
    }

    fn dispatch(&mut self, request: Value) -> Result<Value, String> {
        if request["protocol"].as_u64() != Some(1) {
            return Err("辅助核心协议必须为 1".into());
        }
        let op = request["op"].as_str().ok_or("辅助核心操作名必须为字符串")?;
        let args = request["args"].as_array().ok_or("辅助核心参数必须为数组")?;
        match op {
            "core.protocol" => Ok(json!(1)),
            "core.reset" => {
                self.machines.clear();
                // Keep monotonically increasing handles so a stale machine cannot
                // accidentally address a newly created machine after a reset.
                Ok(Value::Null)
            }
            "machine.create" => {
                if args.first().and_then(Value::as_str) != Some("helper") {
                    return Err("未重建的辅助状态机类型".into());
                }
                if args.get(1).is_some_and(|v| !v.is_null()) {
                    return Err("helper 状态机尚不支持非空初始化参数".into());
                }
                self.next_handle = self.next_handle.checked_add(1).ok_or("辅助状态机句柄已耗尽")?;
                self.machines.insert(self.next_handle, Machine::default());
                Ok(json!(self.next_handle))
            }
            "machine.dispose" => {
                let handle = handle(args)?;
                self.machines.remove(&handle).ok_or("辅助状态机不存在或已释放")?;
                Ok(Value::Null)
            }
            "machine.input" | "machine.answer" | "machine.view" => {
                let machine = self.machines.get_mut(&handle(args)?).ok_or("辅助状态机不存在或已释放")?;
                let value = args.get(1).unwrap_or(&Value::Null);
                match op {
                    "machine.input" => machine.input(value),
                    "machine.answer" => machine.answer(value),
                    _ => machine.view(value),
                }
            }
            "loginRecovery.plan" => Ok(login_recovery_plan(args.first().unwrap_or(&Value::Null))),
            _ => Err(format!("未恢复的辅助核心操作：{op}")),
        }
    }
}

enum Pending {
    Snapshot,
    PreferencesRead,
    PreferencesWrite(bool),
}

#[derive(Default)]
struct Machine {
    initialized: bool,
    disposed: bool,
    pending: Option<Pending>,
    observe: bool,
    context: Value,
    snapshot: Value,
    accounts_busy: bool,
    revision: u64,
    requests: u64,
    replies: u64,
    last_error: String,
}

impl Machine {
    fn input(&mut self, input: &Value) -> Result<Value, String> {
        if self.disposed { return Err("辅助状态机已停止".into()); }
        if self.pending.is_some() { return Err("辅助状态机正在等待宿主操作回应".into()); }
        let kind = input["type"].as_str().ok_or("缺少辅助事件类型")?;
        if let Some(context) = input.get("context") {
            if self.context != *context {
                self.context = context.clone();
                self.revision += 1;
            }
        }
        match kind {
            "command" => self.command(input),
            "tick" => Ok(self.output()),
            "invalidate" => {
                self.requests = 0;
                self.replies = 0;
                self.last_error.clear();
                self.revision += 1;
                Ok(self.output())
            }
            "dispose" => {
                self.disposed = true;
                self.observe = false;
                Ok(self.output())
            }
            "observe" => {
                if self.observe {
                    match input["kind"].as_str() {
                        Some("request") => self.requests += 1,
                        Some("reply") => self.replies += 1,
                        _ => return Err("未知的通信观察事件".into()),
                    }
                    // Only counts are retained; credentials/payloads are not stored.
                    self.revision += 1;
                }
                Ok(self.output())
            }
            _ => self.unsupported(format!("未恢复的辅助事件：{kind}")),
        }
    }

    fn command(&mut self, input: &Value) -> Result<Value, String> {
        let action = input["action"].as_str().ok_or("缺少辅助命令 action")?;
        match action {
            "panel-refresh" => {
                self.revision += 1;
                if !self.initialized {
                    self.pending = Some(Pending::Snapshot);
                    Ok(self.call_host("panel.snapshot", json!([])))
                } else { Ok(self.output()) }
            }
            "panel-event" => {
                let event = &input["value"];
                if event["id"].as_str() != Some("helper.observe") {
                    return self.unsupported(format!("未恢复的面板控件：{}", event["id"]));
                }
                let observe = event["value"].as_bool().ok_or("通信计数选项必须为布尔值")?;
                self.pending = Some(Pending::PreferencesWrite(observe));
                let preferences = json!({"schema":1,"observe":observe}).to_string();
                Ok(self.call_host("panel.prefs.write", json!([PREFS_KEY, preferences])))
            }
            "accounts-busy" => {
                self.accounts_busy = input["value"].as_bool().ok_or("账号忙碌状态必须为布尔值")?;
                self.revision += 1;
                Ok(self.output())
            }
            "account-switch" => {
                self.context = Value::Null;
                self.requests = 0;
                self.replies = 0;
                self.revision += 1;
                Ok(self.output())
            }
            _ => self.unsupported(format!("未恢复的辅助命令：{action}")),
        }
    }

    fn answer(&mut self, answer: &Value) -> Result<Value, String> {
        let pending = self.pending.take().ok_or("辅助状态机没有等待中的宿主操作")?;
        if let Some(thrown) = answer.get("thrown") {
            return self.unsupported(format!("宿主操作失败：{}", thrown.as_str().unwrap_or("未知错误")));
        }
        let value = answer.get("value").ok_or("宿主回应必须包含 value 或 thrown")?;
        match pending {
            Pending::Snapshot => {
                if !value.is_object() { return self.unsupported("宿主快照必须是对象".into()); }
                self.snapshot = value.clone();
                self.pending = Some(Pending::PreferencesRead);
                Ok(self.call_host("panel.prefs.read", json!([PREFS_KEY])))
            }
            Pending::PreferencesRead => {
                if let Some(text) = value.as_str().filter(|text| !text.is_empty()) {
                    let preferences: Value = serde_json::from_str(text).map_err(|e| format!("辅助偏好格式错误：{e}"))?;
                    if preferences["schema"].as_u64() != Some(1) || !preferences["observe"].is_boolean() {
                        return self.unsupported("辅助偏好版本或字段无效".into());
                    }
                    self.observe = preferences["observe"] == true;
                } else if !value.is_null() && value.as_str() != Some("") {
                    return self.unsupported("辅助偏好读取结果必须为字符串或 null".into());
                }
                self.initialized = true;
                self.revision += 1;
                Ok(self.output())
            }
            Pending::PreferencesWrite(observe) => {
                // ControlPanelBridge.platform throws when persistence fails.
                if !value.is_null() {
                    return self.unsupported("辅助偏好写入回应与宿主契约不符".into());
                }
                self.observe = observe;
                self.revision += 1;
                Ok(self.output())
            }
        }
    }

    fn output(&self) -> Value {
        json!({"call":null,"wake":null,"brief":{
            "observe":self.observe,"stacks":false,"panelDirty":true,
            "panelDue":null,"statusBarRevision":self.revision,
        }})
    }

    fn call_host(&self, op: &str, args: Value) -> Value {
        let mut output = self.output();
        output["call"] = json!({"op":op,"args":args});
        output
    }

    fn unsupported(&mut self, message: String) -> Result<Value, String> {
        self.last_error = message.clone();
        self.revision += 1;
        let mut output = self.output();
        output["thrown"] = json!(message);
        Ok(output)
    }

    fn view(&self, query: &Value) -> Result<Value, String> {
        if self.disposed { return Err("辅助状态机已停止".into()); }
        match query["part"].as_str() {
            Some("panel") => Ok(json!({
                "op":"panelSet","revision":self.revision,"state":[],
                "document":{"schema":3,"tabs":[{"id":"info","title":"信息","sections":[{
                    "id":"helper.status","title":"宿主状态","error":self.last_error,
                    "blocks":[
                        {"id":"helper.connection","type":"status","title":if self.initialized { "辅助宿主已就绪" } else { "辅助宿主初始化中" },"detail":if self.accounts_busy { "账号操作正在进行" } else { "原生窗口与面板已连接" }},
                        {"id":"helper.scope","type":"note","text":"此辅助核心为独立重建。战斗、日常、兑换与录制自动化尚未恢复。"},
                        {"id":"helper.preferences","type":"form","items":[{"id":"helper.observe","type":"field","kind":"toggle","label":"记录当前会话的通信次数","value":self.observe}]},
                        {"id":"helper.counts","type":"facts","items":[{"label":"发出请求","value":self.requests},{"label":"收到回应","value":self.replies}]}
                    ]
                }]}]}
            })),
            Some("status-bar") => Ok(json!({"revision":self.revision,"activities":[],"items":[{"label":"辅助核心","value":"重建模式"}]})),
            Some("performance") => Ok(json!({"requests":self.requests,"replies":self.replies,"pendingHostCall":self.pending.is_some()})),
            _ => Err(format!("未恢复的辅助视图：{}", query["part"])),
        }
    }
}

fn handle(args: &[Value]) -> Result<u32, String> {
    args.first().and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0).ok_or_else(|| "无效的辅助状态机句柄".into())
}

/// Pure decision only: the caller owns any actual login/cookie changes.
/// Static branch/constant reconstruction, not yet differential runtime validation.
pub fn login_recovery_plan(facts: &Value) -> Value {
    let flag = |name: &str| facts[name].as_bool().unwrap_or(false);
    let password_panel = facts["panels"].as_array().is_some_and(|panels| {
        panels.iter().filter_map(Value::as_str).any(|name| LOGIN_PANELS.contains(&name))
    });
    if !flag("allowed") || flag("busy") || flag("prompted") || (!flag("required") && !password_panel) {
        return json!({"action":"wait","passwordPanel":password_panel});
    }
    if flag("hasPassword") && !flag("attempted") {
        return json!({"action":"password","passwordPanel":password_panel,"invalidate":flag("hasSession"),"attempted":true});
    }
    let message = if flag("attempted") {
        "自动登录未能进入游戏，已停止自动重试。请重新登录；如需验证码，请打开官方登录页完成验证。"
    } else {
        "登录会话已失效，当前账号没有可用的已保存密码。请在账号管理中保存密码，或重新登录。"
    };
    json!({"action":"prompt","passwordPanel":password_panel,"invalidate":flag("hasSession"),"message":message,"prompted":true})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(helper: &mut Helper, op: &str, args: Value) -> Value {
        serde_json::from_str(&helper.call(&json!({"protocol":1,"op":op,"args":args}).to_string())).unwrap()
    }
    fn create(helper: &mut Helper) -> Value { call(helper, "machine.create", json!(["helper",null]))["value"].clone() }

    #[test]
    fn protocol_reset_lifetime_and_unknown_operations() {
        let mut helper = Helper::default();
        assert_eq!(call(&mut helper,"core.protocol",json!([]))["value"],1);
        assert!(serde_json::from_str::<Value>(&helper.call("{} ")).unwrap().get("error").is_some());
        let handle = create(&mut helper);
        call(&mut helper,"core.reset",json!([]));
        assert!(call(&mut helper,"machine.view",json!([handle,{"part":"panel"}])).get("error").is_some());
        assert_ne!(create(&mut helper),handle);
        assert!(call(&mut helper,"unimplemented",json!([])).get("error").is_some());
    }

    #[test]
    fn panel_startup_runs_real_snapshot_and_persistence_continuations() {
        let mut helper = Helper::default(); let handle = create(&mut helper);
        let first = call(&mut helper,"machine.input",json!([handle,{"type":"command","action":"panel-refresh"}]));
        assert_eq!(first["value"]["call"]["op"],"panel.snapshot");
        let second = call(&mut helper,"machine.answer",json!([handle,{"value":{"development":false,"utcOffsetMinutes":480}}]));
        assert_eq!(second["value"]["call"]["op"],"panel.prefs.read");
        let third = call(&mut helper,"machine.answer",json!([handle,{"value":"{\"schema\":1,\"observe\":true}"}]));
        assert_eq!(third["value"]["brief"]["observe"],true);
        assert_eq!(third["value"]["wake"],Value::Null);
        let panel = call(&mut helper,"machine.view",json!([handle,{"part":"panel"}]));
        assert_eq!(panel["value"]["document"]["schema"],3);
        let unknown = call(&mut helper,"machine.input",json!([handle,{"type":"command","action":"battle-start"}]));
        assert!(unknown["value"]["thrown"].as_str().unwrap().contains("未恢复"));
    }

    #[test]
    fn failed_preference_write_does_not_commit_state() {
        let mut helper = Helper::default(); let handle = create(&mut helper);
        let write = call(&mut helper,"machine.input",json!([handle,{"type":"command","action":"panel-event","value":{"id":"helper.observe","value":true}}]));
        assert_eq!(write["value"]["call"]["op"],"panel.prefs.write");
        let failed = call(&mut helper,"machine.answer",json!([handle,{"thrown":"disk full"}]));
        assert_eq!(failed["value"]["brief"]["observe"],false);
        assert!(failed["value"]["thrown"].as_str().unwrap().contains("disk full"));
    }

    #[test]
    fn recovery_decision_uses_exact_classes_and_strict_boolean_flags() {
        for name in LOGIN_PANELS {
            let plan = login_recovery_plan(&json!({"allowed":true,"hasPassword":true,"hasSession":true,"panels":[name]}));
            assert_eq!(plan,json!({"action":"password","passwordPanel":true,"invalidate":true,"attempted":true}));
        }
        assert_eq!(login_recovery_plan(&json!({"allowed":true,"panels":["UnknownLoginPanel"]}))["action"],"wait");
        assert_eq!(login_recovery_plan(&json!({"allowed":true,"required":true,"attempted":true}))["action"],"prompt");
        assert_eq!(login_recovery_plan(&json!({"allowed":true,"required":true,"busy":true}))["action"],"wait");
        assert_eq!(login_recovery_plan(&json!({"allowed":"true","required":true}))["action"],"wait");
    }
}
