//! Aola official-page login adapter on the shared native Ruffle renderer.
//! Query and cookie values stay in memory; diagnostics contain only stage names.
use std::collections::BTreeMap;
use std::cell::Cell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use ruffle_core::context::UpdateContext;
use ruffle_core::external::{ExternalInterfaceProvider, Value};
use serde_json::json;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;
use url::Url;
use winit::event::WindowEvent;
use winit::event_loop::EventLoopProxy;
use winit::window::Window;
use wry::{PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};
use crate::custom_event::RuffleEvent;

pub const PAGE_URL: &str = "https://aola.100bt.com/play/play.html?from_h5=true";
pub const START_URL: &str = "https://aola.100bt.com/play/start~2107300001.swf";
static ENABLED: AtomicBool = AtomicBool::new(false);
static STATE: OnceLock<Mutex<PageState>> = OnceLock::new();
static PROXY: OnceLock<EventLoopProxy<RuffleEvent>> = OnceLock::new();
static CLOCK: OnceLock<Instant> = OnceLock::new();
static LOG: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
static DATA_ROOT: OnceLock<PathBuf> = OnceLock::new();

#[derive(Default)]
struct PageState {
    href: String,
    referrer: String,
    user_agent: String,
    cookies: BTreeMap<String, String>,
    bootstrapped: bool,
    launched: bool,
}

pub fn initialize(root: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(root.join("diagnostics"))?;
    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
    let file=std::fs::File::create(root.join("diagnostics").join(format!("startup-{stamp}-{}.jsonl",std::process::id())))?;
    let _=LOG.set(Mutex::new(file));
    let _=DATA_ROOT.set(root.to_owned());
    STATE.get_or_init(|| Mutex::new(PageState {href:PAGE_URL.into(),..Default::default()}));
    CLOCK.get_or_init(Instant::now);
    ENABLED.store(true,Ordering::Relaxed);
    if ruffle_core::aola_panel_diagnostics::enabled() { record("host", "panel-diagnostics-enabled"); }
    Ok(())
}
pub fn enabled() -> bool { ENABLED.load(Ordering::Relaxed) }
pub fn elapsed_ms() -> u128 { CLOCK.get_or_init(Instant::now).elapsed().as_millis() }
pub fn write_event(row: serde_json::Value) {
    if let Some(log)=LOG.get() { let _=writeln!(log.lock().unwrap(),"{row}"); }
}
pub fn set_proxy(proxy: EventLoopProxy<RuffleEvent>) { let _=PROXY.set(proxy); }
fn send(event: RuffleEvent) {
    if let Some(proxy) = PROXY.get() {
        let _ = proxy.send_event(event);
    }
}

/// Call names are useful evidence; values and arbitrary script text are never logged.
fn safe_name(name: &str) -> &str {
    if name.len() <= 128 && name.bytes().all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c)) {
        name
    } else {
        "dynamic"
    }
}

pub fn record(kind: &str, name: &str) {
    let row = json!({"elapsedMs": CLOCK.get_or_init(Instant::now).elapsed().as_millis(), "type": kind, "name": safe_name(name)});
    write_event(row);
}

fn record_page(kind: &str, address: &str) {
    let Ok(url) = Url::parse(address) else { return };
    let row = json!({"elapsedMs": CLOCK.get_or_init(Instant::now).elapsed().as_millis(), "type": kind, "origin": url.origin().ascii_serialization(), "path": url.path()});
    write_event(row);
}

pub fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    record("host-error", "panic");
    if let Some(location) = info.location() {
        let row = json!({"elapsedMs": CLOCK.get_or_init(Instant::now).elapsed().as_millis(), "type":"panic-location",
            "file":location.file(), "line":location.line(), "column":location.column()});
        write_event(row);
    }
}

pub struct DiagnosticLayer;

pub fn diagnostic_enabled(meta: &tracing::Metadata<'_>) -> bool {
    if meta.target() == "aola_close_phase" { return crate::aola_close_phases::configured(); }
    matches!(meta.target(), "aola_notice" | "aola_battle" | "aola_backpack") || meta.target() == "aola_swf_lifecycle" || meta.target() == "aola_swf_operation" || meta.target() == "aola_pet_patch" || meta.target() == "aola_panel_diagnostic" || meta.target() == "avm_trace" || *meta.level() <= tracing::Level::WARN
}

#[derive(Default)]
struct SwfOperation(serde_json::Map<String, serde_json::Value>);

impl Visit for SwfOperation {
    fn record_str(&mut self, field: &Field, value: &str) {
        if (field.name() == "operation" && matches!(value, "panel-created" | "view-switch" | "panel-cleanup"
                | "classify-return" | "battle-enter" | "battle-exit" | "battle-main-gc" | "battle-followup-gc"))
            || (field.name() == "mode" && matches!(value, "finish-cycle" | "slice")) {
            self.0.insert(field.name().into(), value.into());
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        if matches!(field.name(), "battleId" | "registeredMovies" | "registeredMoviesAfter" | "nodesBefore"
            | "nodesAfter" | "dropped" | "freed" | "weakEntriesPruned" | "cyclesCompleted" | "gcUs" | "maintenanceUs") {
            self.0.insert(field.name().into(), value.into());
        }
    }
    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}

fn swf_operation_event(event: &Event<'_>) -> Option<serde_json::Value> {
    if event.metadata().target() != "aola_swf_operation" { return None; }
    let mut fields = SwfOperation::default();
    event.record(&mut fields);
    fields.0.get("operation")?;
    fields.0.insert("type".into(), "swf-operation".into());
    fields.0.insert("build".into(), crate::aola_diagnostics::BUILD.into());
    fields.0.insert("elapsedMs".into(), (elapsed_ms() as u64).into());
    Some(fields.0.into())
}

#[derive(Default)]
struct PanelDiagnostic(serde_json::Map<String, serde_json::Value>);

impl Visit for PanelDiagnostic {
    fn record_str(&mut self, field: &Field, value: &str) {
        if matches!(field.name(), "source" | "phase" | "stack") {
            self.0.insert(field.name().into(), value.chars().take(4096).collect::<String>().into());
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "value" { self.0.insert("value".into(), value.into()); }
    }
    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}

fn panel_diagnostic_event(event: &Event<'_>) -> Option<serde_json::Value> {
    if event.metadata().target() != "aola_panel_diagnostic" { return None; }
    let mut fields = PanelDiagnostic::default();
    event.record(&mut fields);
    fields.0.insert("type".into(), "panel-diagnostic".into());
    fields.0.insert("elapsedMs".into(), (elapsed_ms() as u64).into());
    Some(fields.0.into())
}

#[derive(Default)]
struct PetPatchHit(Option<bool>);

impl Visit for PetPatchHit {
    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "optimized" { self.0 = Some(value); }
    }
    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}

fn pet_patch_event(event: &Event<'_>) -> Option<serde_json::Value> {
    if event.metadata().target() != "aola_pet_patch" { return None; }
    let mut hit = PetPatchHit::default();
    event.record(&mut hit);
    hit.0.map(|optimized| json!({"elapsedMs":elapsed_ms(), "type":"pet-swf-replacement", "optimized":optimized}))
}

#[derive(Default)]
struct SwfLifecycleFields(serde_json::Map<String, serde_json::Value>);

impl Visit for SwfLifecycleFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if matches!(field.name(), "mode" | "resource" | "version" | "source" | "result") {
            self.0.insert(field.name().into(), value.into());
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "bytes" { self.0.insert("bytes".into(), value.into()); }
    }
    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}

#[derive(Default)]
struct ErrorCode(String);

impl Visit for ErrorCode {
    fn record_debug(&mut self, _field: &Field, value: &dyn std::fmt::Debug) {
        // Keep only the numeric Flash error, never the original message or arguments.
        let value = format!("{value:?}");
        if let Some((_, rest)) = value.split_once("Error #") {
            let code: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if code.len() == 4 { self.0 = code; }
        }
    }
}

impl<S: Subscriber> Layer<S> for DiagnosticLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() == "aola_close_phase" {
            crate::aola_close_phases::on_event(event);
            return;
        }
        if event.metadata().target() == "aola_swf_operation" {
            if let Some(row) = swf_operation_event(event) { write_event(row); }
            return;
        }
        let target = event.metadata().target();
        if matches!(target, "aola_swf_lifecycle" | "aola_notice" | "aola_battle" | "aola_backpack") {
            let mut fields = SwfLifecycleFields::default();
            event.record(&mut fields);
            fields.0.insert("type".into(), match target { "aola_notice" => "notice-swf", "aola_battle" => "battle-swf", "aola_backpack" => "backpack-swf", _ => "swf-lifecycle" }.into());
            fields.0.insert("build".into(), crate::aola_diagnostics::BUILD.into());
            fields.0.insert("elapsedMs".into(), (elapsed_ms() as u64).into());
            write_event(fields.0.into());
            return;
        }
        if let Some(row) = panel_diagnostic_event(event) { write_event(row); return; }
        if let Some(row) = pet_patch_event(event) {
            write_event(row);
            return;
        }
        let meta = event.metadata();
        if *meta.level() <= tracing::Level::WARN || meta.target() == "avm_trace" {
            let mut code = ErrorCode::default();
            event.record(&mut code);
            if meta.target() != "avm_trace" { record("runtime-diagnostic", meta.target()); }
            if !code.0.is_empty() {
                record("flash-error-code", &code.0);
            }
        }
    }
}

fn arguments(args: &[Value]) -> &[Value] {
    match args {
        [Value::List(items)] => items,
        _ => args,
    }
}

fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn query(href: &str, key: &str) -> Option<String> {
    let query = href.split_once('?')?.1.split('#').next()?;
    query.split('&').find_map(|part| {
        let mut pair = part.split('=');
        (pair.next()? == key).then(|| pair.next().unwrap_or("").to_owned())
    })
}

fn query_result(href: &str, key: &str) -> Value {
    match query(href, key) {
        Some(value) => Value::String(value),
        None if href.split_once('?').is_some_and(|(_, rest)| !rest.split('#').next().unwrap_or("").is_empty()) => Value::Undefined,
        None => Value::Null,
    }
}

fn cookie_script(key: &str, value: &str, expires: f64) -> String {
    format!("(() => {{ let s={key}+'='+encodeURIComponent({value})+';path=/;domain=.100bt.com'; if({expires}!==0) s+=';expires='+new Date(Date.now()+{expires}).toUTCString(); document.cookie=s; }})();")
}

pub struct AolaInterface {
    hide_notice: bool,
    observed_notice_channels: Cell<u8>,
    block_skill: bool,
    hide_transform_portrait: bool,
    skip_battle_backdrop: bool,
    observed_battle_effects: Cell<u8>,
}

impl AolaInterface {
    pub fn new(hide_notice: bool, block_skill: bool, hide_transform_portrait: bool, skip_battle_backdrop: bool) -> Self {
        Self {
            hide_notice, block_skill, hide_transform_portrait, skip_battle_backdrop,
            observed_notice_channels: Cell::new(0), observed_battle_effects: Cell::new(0),
        }
    }

    fn skill_result(&self, args: &[Value]) -> Value {
        let [Value::String(name)] = arguments(args) else { return Value::Bool(false) };
        let (bit, blocked) = match name.as_str() {
            "wanyu-zhichao" => (1, self.block_skill),
            "transform-portrait-yiren" => (2, self.hide_transform_portrait),
            "transform-portrait-jiuri" => (4, self.hide_transform_portrait),
            "transform-portrait-juexing" => (8, self.hide_transform_portrait),
            "transform-portrait-dengshen" => (16, self.hide_transform_portrait),
            "transform-portrait-zhenli" => (32, self.hide_transform_portrait),
            "battle-backdrop-gq" => (64, self.skip_battle_backdrop),
            "battle-backdrop-xinghui" => (128, self.skip_battle_backdrop),
            _ => return Value::Bool(false),
        };
        let observed = self.observed_battle_effects.get();
        if blocked && observed & bit == 0 {
            self.observed_battle_effects.set(observed | bit);
            record("battle-effect", name);
        }
        Value::Bool(blocked)
    }

    fn notice_result(&self, args: &[Value]) -> Value {
        let channel = match arguments(args).first() {
            Some(Value::String(value)) => value.as_str(),
            _ => "",
        };
        let (bit, label) = match channel {
            "random" => (1, "random"),
            "honor" => (2, "honor"),
            "title" => (4, "title"),
            "broadcast" => (8, "broadcast"),
            "coming" => (16, "coming"),
            _ => (32, "default"),
        };
        let observed = self.observed_notice_channels.get();
        if observed & bit == 0 {
            self.observed_notice_channels.set(observed | bit);
            record("notice-channel", label);
        }
        Value::Bool(self.hide_notice)
    }
}

impl ExternalInterfaceProvider for AolaInterface {
    fn call_method(&self, _context: &mut UpdateContext<'_>, name: &str, args: &[Value]) -> Value {
        if name == "aolaBlockSkill" { return self.skill_result(args); }
        if name == "aolaHideNotice" { return self.notice_result(args); }
        record("external-call", name);
        let args = arguments(args);
        let mut state = STATE.get().unwrap().lock().unwrap();
        match name {
            "getQS" | "getU" | "getCode" => {
                let key = match name { "getU" => "u".into(), "getCode" => "code".into(), _ => text(args.first()) };
                query_result(&state.href, &key)
            }
            "getHref" | "getCurWindow" | "getWindowLocationHref" => Value::String(state.href.clone()),
            "getUserAgent" => Value::String(state.user_agent.clone()),
            "getFrom" => Value::String(query(&state.href, "site").unwrap_or_default()),
            "getRef" | "getSN" => {
                let from = if state.referrer.is_empty() { query(&state.href, "site").unwrap_or_default() } else { state.referrer.clone() };
                Value::String(if name == "getSN" && from.contains('/') { from.split('/').nth(2).unwrap_or("").into() } else { from })
            }
            "isReg" => Value::Bool(query(&state.href, "reg").is_some()),
            "getCookie" => state.cookies.get(&text(args.first())).cloned().map(Value::String).unwrap_or(Value::Null),
            "addCookieFromAs" => {
                let args = arguments(args);
                let key = text(args.first());
                let value = text(args.get(1));
                state.cookies.insert(key.clone(), value.clone());
                send(RuffleEvent::AolaCookie(key, value, text(args.get(2))));
                Value::Undefined
            }
            "delCookie" => {
                let key = text(args.first());
                state.cookies.remove(&key);
                send(RuffleEvent::AolaCookie(key, String::new(), "-1000".into()));
                Value::Undefined
            }
            "showWebLogin" => {
                record("login-argument", match args.first() {
                    Some(Value::String(_)) => "string",
                    Some(Value::List(_)) => "array",
                    Some(Value::Object(_)) => "object",
                    _ => "other",
                });
                state.launched = false;
                send(RuffleEvent::AolaNavigate(text(args.first())));
                Value::Undefined
            }
            "setSWFIsReady" => {
                record("startup-marker", "setSWFIsReady");
                write_event(json!({"elapsedMs": elapsed_ms(), "type": "loader-ready"}));
                Value::Undefined
            }
            "sendResourceData" | "addIOErrorLog" | "addSocketConnectFailLog" => Value::Undefined,
            _ => { record("unhandled-external-call", name); Value::Undefined }
        }
    }

    fn on_callback_available(&self, name: &str) {
        record("external-callback", name);
    }
    fn get_id(&self) -> Option<String> { Some("swfContainer".into()) }
}

const SNAPSHOT_SCRIPT: &str = r#"(() => {
    if (window !== window.top) return;
    let lastPageInfo = '';
    function report() {
        if (!window.ipc) return;
        const pageInfo = JSON.stringify({kind:'page-info', origin:location.origin, path:location.pathname,
            plugins:document.querySelectorAll('object,embed').length,
            passwords:document.querySelectorAll('input[type=password]').length,
            forms:document.forms.length});
        if (pageInfo !== lastPageInfo) {
            window.ipc.postMessage(pageInfo);
            lastPageInfo = pageInfo;
        }
        if (location.hostname !== 'aola.100bt.com') return;
        const cookies = {};
        document.cookie.split(';').forEach(part => {
            const at = part.indexOf('=');
            if (at >= 0) cookies[part.slice(0, at).trim()] = decodeURIComponent(part.slice(at + 1));
        });
        window.ipc.postMessage(JSON.stringify({href:location.href, referrer:document.referrer, userAgent:navigator.userAgent.toLowerCase(), cookies}));
    }
    window.addEventListener('DOMContentLoaded', () => {
        if (location.hostname === 'account.100bt.com' && location.pathname === '/game/auth/login.html') {
            const style = document.createElement('style');
            style.textContent = '#flashBg { display: none !important; }';
            document.head.appendChild(style);
        }
        report();
    });
    window.addEventListener('pageshow', report);
    setInterval(report, 1000);
})();"#;

pub struct LoginBrowser {
    webview: WebView,
    _context: WebContext,
    window: Arc<Window>,
}

impl LoginBrowser {
    pub fn new(window: Arc<Window>, proxy: EventLoopProxy<RuffleEvent>) -> anyhow::Result<Self> {
        let size = window.inner_size();
        let mut context=WebContext::new(Some(DATA_ROOT.get().unwrap().join("webview")));
        let webview = WebViewBuilder::new_with_web_context(&mut context)
            .with_incognito(true)
            .with_visible(false)
            .with_focused(false)
            .with_bounds(Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(size.width, size.height).into() })
            .with_initialization_script(SNAPSHOT_SCRIPT)
            .with_navigation_handler(|address| { record_page("page-navigation", &address); true })
            .with_on_page_load_handler(|event, address| {
                record_page(match event { PageLoadEvent::Started => "page-load-started", PageLoadEvent::Finished => "page-load-finished" }, &address);
            })
            .with_ipc_handler(move |request| { let _ = proxy.send_event(RuffleEvent::AolaPageState(request.into_body())); })
            .with_url(PAGE_URL)
            .build_as_child(window.as_ref())?;
        record("host", "embedded-webview2-created");
        Ok(Self { webview, _context:context, window })
    }

    pub fn window_event(&self, event: &WindowEvent) {
        let size = match event {
            WindowEvent::Resized(size) => *size,
            WindowEvent::ScaleFactorChanged { .. } => self.window.inner_size(),
            _ => return,
        };
        if size.width > 0 && size.height > 0 {
            if self.webview.set_bounds(Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(size.width, size.height).into() }).is_err() {
                record("host-error", "embedded-login-resize");
            }
        }
    }

    pub fn accept_snapshot(&self, payload: &str) -> bool {
        let Ok(snapshot) = serde_json::from_str::<serde_json::Value>(payload) else { return false };
        if snapshot["kind"] == "page-info" {
            let row = json!({"elapsedMs": CLOCK.get_or_init(Instant::now).elapsed().as_millis(), "type":"page-dom",
                "origin":snapshot["origin"].as_str().unwrap_or(""), "path":snapshot["path"].as_str().unwrap_or(""),
                "plugins":snapshot["plugins"].as_u64().unwrap_or(0),
                "passwords":snapshot["passwords"].as_u64().unwrap_or(0), "forms":snapshot["forms"].as_u64().unwrap_or(0)});
            write_event(row);
            return false;
        }
        let href = snapshot["href"].as_str().unwrap_or("");
        let Ok(url) = Url::parse(href) else { return false };
        if url.host_str() != Some("aola.100bt.com") || url.path() != "/play/play.html" { return false; }
        let mut state = STATE.get().unwrap().lock().unwrap();
        state.href = href.into();
        state.referrer = snapshot["referrer"].as_str().unwrap_or("").into();
        state.user_agent = snapshot["userAgent"].as_str().unwrap_or("").into();
        state.cookies = snapshot["cookies"].as_object().map(|items| items.iter().map(|(key, value)| (key.clone(), value.as_str().unwrap_or("").into())).collect()).unwrap_or_default();
        let has_game_auth_sid = state.cookies.get("game_auth_sid").is_some_and(|value| !value.is_empty());
        let has_sid_query = query(&state.href, "sid").is_some();
        let has_u_query = query(&state.href, "u").is_some();
        let has_code_query = query(&state.href, "code").is_some();
        let ready = !state.launched && has_game_auth_sid;
        let first_context = !state.bootstrapped;
        state.bootstrapped = true;
        if first_context {
            write_event(json!({"elapsedMs": elapsed_ms(), "type":"page-context", "phase":"first",
                "hasGameAuthSid":has_game_auth_sid, "hasSidQuery":has_sid_query,
                "hasUQuery":has_u_query, "hasCodeQuery":has_code_query,
                "cookieCount":state.cookies.len(), "hrefPath":url.path()}));
            record("host", "official-page-context-ready");
        }
        if ready {
            state.launched = true;
            // The first snapshot is usually taken before login; record the post-login
            // context separately so the checker can see the final cookie/query presence.
            write_event(json!({"elapsedMs": elapsed_ms(), "type":"page-context", "phase":"login-ready",
                "hasGameAuthSid":has_game_auth_sid, "hasSidQuery":has_sid_query,
                "hasUQuery":has_u_query, "hasCodeQuery":has_code_query,
                "cookieCount":state.cookies.len(), "hrefPath":url.path()}));
            record("host", "official-login-ready");
            if self.webview.set_visible(false).is_err() { record("host-error", "embedded-login-hide"); }
            if self.webview.focus_parent().is_err() { record("host-error", "game-focus"); }
            self.window.request_redraw();
            record("host", "embedded-login-hidden");
        }
        first_context || ready
    }

    pub fn navigate(&self, url: &str) {
        if self.webview.set_visible(true).is_err() { record("host-error", "embedded-login-show"); }
        record("login-address", if url.is_empty() { "empty" } else { "nonempty" });
        let mut url = Url::parse(PAGE_URL).unwrap().join(url).unwrap();
        if url.host_str() == Some("account.100bt.com") && url.path() == "/game/auth/login.html" {
            let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
            if pairs.iter().any(|(key, value)| key == "action" && value == "startgame") {
                let mut query = url.query_pairs_mut();
                query.clear();
                for (key, value) in &pairs {
                    query.append_pair(key, if key == "action" && value == "startgame" { "login" } else { value });
                }
                record("host", "official-html-login-selected");
            }
        }
        record_page("login-request", url.as_str());
        if self.webview.load_url(url.as_str()).is_err() { record("host-error", "login-load-url"); }
        self.window.focus_window();
        if self.webview.focus().is_err() { record("host-error", "embedded-login-focus"); }
        record("host", "embedded-login-visible");
        record("host", "login-navigation");
    }

    pub fn write_cookie(&self, key: &str, value: &str, expires: &str) {
        let key = serde_json::to_string(key).unwrap();
        let value = serde_json::to_string(value).unwrap();
        let expires: f64 = expires.parse().unwrap_or(0.0);
        let script = cookie_script(&key, &value, expires);
        let _ = self.webview.evaluate_script(&script);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_battle_gc_events_keep_the_numeric_per_cleanup_counters() {
        use tracing_subscriber::prelude::*;
        struct Capture(Arc<Mutex<Vec<serde_json::Value>>>);
        impl<S: Subscriber> Layer<S> for Capture {
            fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
                if let Some(row) = swf_operation_event(event) { self.0.lock().unwrap().push(row); }
            }
        }
        let rows = Arc::new(Mutex::new(Vec::new()));
        tracing::subscriber::with_default(tracing_subscriber::registry().with(Capture(rows.clone())), || {
            for operation in ["battle-main-gc", "battle-followup-gc"] {
                tracing::info!(target: "aola_swf_operation", operation, mode = "finish-cycle", battleId = 1u64,
                    registeredMovies = 306u64, registeredMoviesAfter = 301u64, nodesBefore = 17u64,
                    nodesAfter = 10u64, dropped = 8u64, freed = 7u64, weakEntriesPruned = 2u64,
                    cyclesCompleted = 1u64, gcUs = 42u64, maintenanceUs = 17u64, other = "ignored");
            }
        });
        let rows = rows.lock().unwrap();
        assert_eq!(rows.len(), 2);
        for (row, operation) in rows.iter().zip(["battle-main-gc", "battle-followup-gc"]) {
            assert_eq!(row["operation"], operation);
            assert_eq!(row["mode"], "finish-cycle");
            for (name, value) in [("battleId", 1), ("registeredMovies", 306), ("registeredMoviesAfter", 301),
                ("nodesBefore", 17), ("nodesAfter", 10), ("dropped", 8), ("freed", 7),
                ("weakEntriesPruned", 2), ("cyclesCompleted", 1), ("gcUs", 42), ("maintenanceUs", 17)] {
                assert_eq!(row[name].as_u64(), Some(value));
            }
            assert!(row.get("other").is_none());
        }
    }

    #[test]
    fn aola_notice_callback_shares_one_boolean_and_records_each_channel_once() {
        for enabled in [false, true] {
            let interface = AolaInterface::new(enabled, false, false, false);
            for name in ["random", "honor", "title", "broadcast", "coming", "random"] {
                assert!(matches!(interface.notice_result(&[Value::String(name.into())]), Value::Bool(v) if v == enabled));
            }
            assert_eq!(interface.observed_notice_channels.get(), 31);
            assert!(matches!(interface.notice_result(&[]), Value::Bool(v) if v == enabled));
            assert_eq!(interface.observed_notice_channels.get(), 63);
        }
    }
    #[test]
    fn aola_battle_callbacks_gate_portraits_and_attack_independently() {
        for mask in 0..8 {
            let (block_skill, hide_portrait, skip_backdrop) = (mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
            let interface = AolaInterface::new(false, block_skill, hide_portrait, skip_backdrop);
            let skill = Value::String("wanyu-zhichao".into());
            for malformed in [vec![], vec![Value::Bool(true)], vec![Value::String("other".into())],
                vec![skill.clone(), skill.clone()]] {
                assert!(matches!(interface.skill_result(&malformed), Value::Bool(false)));
            }
            assert_eq!(interface.observed_battle_effects.get(), 0);
            let mut expected_mask = 0;
            for (name, bit, enabled) in [
                ("transform-portrait-yiren", 2, hide_portrait),
                ("wanyu-zhichao", 1, block_skill),
                ("transform-portrait-jiuri", 4, hide_portrait),
                ("transform-portrait-juexing", 8, hide_portrait),
                ("transform-portrait-dengshen", 16, hide_portrait),
                ("transform-portrait-zhenli", 32, hide_portrait),
                ("battle-backdrop-gq", 64, skip_backdrop),
                ("battle-backdrop-xinghui", 128, skip_backdrop),
            ] {
                if enabled { expected_mask |= bit; }
                let channel = Value::String(name.into());
                for argument in [channel.clone(), Value::List(vec![channel])] {
                    let result = interface.skill_result(&[argument]);
                    assert!(matches!(result, Value::Bool(v) if v == enabled));
                    assert_eq!(interface.observed_battle_effects.get(), expected_mask);
                }
            }
        }
    }
    #[test]
    fn aola_panel_diagnostics_pass_filter_and_ignore_argument_fields() {
        use tracing_subscriber::prelude::*;
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<serde_json::Value>>>);
        impl<S: Subscriber> Layer<S> for Capture {
            fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
                if let Some(row) = panel_diagnostic_event(event) { self.0.lock().unwrap().push(row); }
            }
        }
        let rows = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(Capture(rows.clone()).with_filter(
            tracing_subscriber::filter::filter_fn(diagnostic_enabled)));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "aola_panel_diagnostic", source="compiled::method()", phase="method-error", value=0u64,
                stack="at compiled::method()", arguments="synthetic-secret", "ignored arbitrary message");
            tracing::info!(target: "unrelated", source="unrelated", phase="method-error");
        });
        let rows = rows.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["source"], "compiled::method()");
        assert_eq!(rows[0]["phase"], "method-error");
        assert_eq!(rows[0]["value"], 0);
        assert_eq!(rows[0]["stack"], "at compiled::method()");
        assert_eq!(rows[0]["type"], "panel-diagnostic");
        assert!(rows[0]["elapsedMs"].is_number());
        assert_eq!(rows[0].as_object().unwrap().len(), 6);
        assert!(!rows[0].to_string().contains("synthetic-secret"));
    }
    #[test]
    fn aola_pet_patch_hits_survive_filter_without_logging_arbitrary_fields() {
        use tracing_subscriber::prelude::*;
        #[derive(Clone)]
        struct Capture(Arc<Mutex<Vec<serde_json::Value>>>);
        impl<S: Subscriber> Layer<S> for Capture {
            fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
                if let Some(row) = pet_patch_event(event) { self.0.lock().unwrap().push(row); }
            }
        }
        let rows = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(Capture(rows.clone()).with_filter(
            tracing_subscriber::filter::filter_fn(diagnostic_enabled)));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "aola_pet_patch", optimized=false, resource="synthetic-private-value", "baseline hit");
            tracing::info!(target: "aola_pet_patch", optimized=true, "optimized hit");
            tracing::info!(target: "aola_pet_patch", optimized="invalid-type", "not a valid hit");
            tracing::info!(target: "unrelated", optimized=true, "not a patch hit");
        });
        let rows = rows.lock().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["optimized"], false);
        assert_eq!(rows[1]["optimized"], true);
        for row in rows.iter() {
            assert_eq!(row["type"], "pet-swf-replacement");
            assert!(row["elapsedMs"].is_number());
            assert_eq!(row.as_object().unwrap().len(), 3);
            assert!(!row.to_string().contains("synthetic-private-value"));
        }
    }
    #[test]
    fn aola_query_preserves_official_page_values() {
        assert!(matches!(query_result(PAGE_URL,"missing"),Value::Undefined));
        assert!(matches!(query_result("https://aola.100bt.com/play/play.html","u"),Value::Null));
        assert!(matches!(query_result("https://aola.100bt.com/play/play.html?u=a%2Bb+c","u"),Value::String(value) if value=="a%2Bb+c"));
        assert!(matches!(query_result("https://aola.100bt.com/play/play.html?code=x=y","code"),Value::String(value) if value=="x"));
    }
    #[test]
    fn aola_webutil_array_and_direct_arguments_match() {
        let direct=vec![Value::String("synthetic-key".into())];
        let wrapped=vec![Value::List(direct.clone())];
        assert_eq!(text(arguments(&direct).first()),text(arguments(&wrapped).first()));
        assert!(arguments(&[]).is_empty());
    }
    #[test]
    fn aola_diagnostic_names_do_not_accept_query_values() {
        assert_eq!(safe_name("getCookie"),"getCookie");
        assert_eq!(safe_name("https://example.test/?sid=synthetic"),"dynamic");
    }
}
