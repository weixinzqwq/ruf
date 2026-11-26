//! Reconstructed Windows host. AS contracts are recovered; lifecycle and UI
//! implementation are new Rust code. UI work is drained outside the Player lock.
use crate::aoqi_helper::Helper;
use crate::aoqi_recovery::StartupRecovery;
use crate::aoqi_storage::Storage;
use crate::aoqi_shell::ShellAction;
use crate::aoqi_web::{WebCommand, WebEvent, WebViews};
use crate::cli::Opt;
use crate::custom_event::RuffleEvent;
use ruffle_core::context::UpdateContext;
use ruffle_core::external::{ExternalInterfaceProvider, Value as ExternalValue};
use ruffle_core::Player;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak, mpsc};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::Window;

thread_local! {
    static CURRENT: RefCell<Option<Rc<RefCell<Host>>>> = const { RefCell::new(None) };
}

enum Command {
    Web(WebCommand),
    Shell(ShellAction),
    Exit,
    Restart,
    Reveal(String),
    Center,
}

#[derive(Default)]
struct WebInfo { url: String, title: String, render_rect: Option<[f64;4]> }

struct Host {
    storage: Storage,
    recovery: StartupRecovery,
    window: Arc<Window>,
    commands: VecDeque<Command>,
    web: HashMap<String, WebInfo>,
    serial: u64,
    player: Weak<Mutex<Player>>,
    callback_ready: bool,
    ready: bool,
    time_scale: f64,
    epoch: u32,
    handoff: Option<String>,
    arguments: Vec<String>,
    helper: Helper,
    shell: crate::aoqi_shell::Shell,
    proxy: EventLoopProxy<RuffleEvent>,
    toolbar_height: f64,
}

pub struct HostUi {
    host: Rc<RefCell<Host>>,
    web: WebViews,
    events: mpsc::Receiver<WebEvent>,
    panel_created: bool,
    panel_ready: bool,
    panel_visible: bool,
    panel_pending: Vec<Value>,
    dialogs: HashMap<String, String>,
    fade: Option<(std::time::Instant, std::time::Duration)>,
    memory_log: Option<crate::aoqi_diagnostics::MemoryLog>,
    presentation: crate::gui::PresentationStats,
}

pub fn provider() -> Option<Box<dyn ExternalInterfaceProvider>> {
    CURRENT.with(|current| current.borrow().as_ref().map(|host| {
        Box::new(Provider(host.clone())) as Box<dyn ExternalInterfaceProvider>
    }))
}

pub fn attach(player: &Arc<Mutex<Player>>) {
    CURRENT.with(|current| {
        if let Some(host) = current.borrow().as_ref() {
            let mut host = host.borrow_mut();
            host.player = Arc::downgrade(player);
            host.callback_ready = false;
        }
    });
}

pub fn time_scale() -> f64 {
    CURRENT.with(|current| current.borrow().as_ref().map_or(1.0, |h| h.borrow().time_scale))
}

/// Menu rendering may hold Player's lock; only queue work, never enter AS here.
pub fn toolbar(action:&str) {
    CURRENT.with(|current| {
        if let Some(host)=current.borrow().as_ref() {
            let mut host=host.borrow_mut();
            let result=match action {
                "restart"=>host.call("app.restart",&[]),
                "refresh"=>host.call("shell.panelExecute",&[json!("refresh"),json!("true")]),
                "fullscreen"=>{
                    let event=if host.window.fullscreen().is_some(){RuffleEvent::ExitFullScreen}else{RuffleEvent::EnterFullScreen};
                    let _=host.proxy.send_event(event);Ok(Value::Null)
                }
                _=>host.call("shell.shellCommand",&[json!({"action":action,"value":true}).to_string().into()]),
            };
            if let Err(error)=result{tracing::error!("Toolbar {action}: {error}");}
        }
    });
}

fn dispatch(host: &Rc<RefCell<Host>>, channel: &str, event: &str, payload: Value) -> ExternalValue {
    let player = {
        let host = host.borrow();
        if !host.callback_ready { return ExternalValue::Undefined; }
        host.player.upgrade()
    };
    let Some(player) = player else { return ExternalValue::Undefined; };
    match player.try_lock() {
        Ok(mut player) => player.call_internal_interface("hostDispatch", [
            channel.to_owned().into(), event.to_owned().into(), from_json(payload),
        ]),
        Err(error) => {
            tracing::error!("Host callback cannot re-enter a locked player: {channel}/{event}: {error}");
            ExternalValue::Undefined
        }
    }
}

impl HostUi {
    pub fn new(opt: &Opt, window: Arc<Window>, proxy: EventLoopProxy<RuffleEvent>) -> Result<Self, String> {
        let root = opt.aoqi_data_dir.clone().ok_or("Missing host data directory")?;
        let storage = Storage::new(root.clone())?;
        let recovery = StartupRecovery::begin(&root, opt.aoqi_safe_mode)?;
        let memory_log=match crate::aoqi_diagnostics::MemoryLog::new(&storage) {
            Ok(log)=>Some(log),
            Err(error)=>{tracing::error!("Resource diagnostics unavailable: {error}");None}
        };
        let host = Rc::new(RefCell::new(Host {
            storage, recovery, window, commands: VecDeque::new(), web: HashMap::new(),
            serial: 0, player: Weak::new(), callback_ready: false, ready: false,
            time_scale: 1.0, epoch: 0, handoff: None, arguments: opt.aoqi_argument.clone(),
            helper: Helper::default(), shell: crate::aoqi_shell::Shell::default(), proxy,
            toolbar_height: if opt.no_gui {0.0} else {f64::from(crate::gui::MENU_HEIGHT)},
        }));
        let (sender, events) = mpsc::channel();
        let mut web = WebViews::new(root.join("webview2"), sender);
        let callback_host = Rc::downgrade(&host);
        web.set_navigation_handler(Rc::new(move |id, url| {
            callback_host.upgrade().is_none_or(|host| {
                !matches!(dispatch(&host, &format!("web:{id}"), "navigating", json!(url)), ExternalValue::Bool(true))
            })
        }));
        CURRENT.with(|current| *current.borrow_mut() = Some(host.clone()));
        Ok(Self { host, web, events, panel_created:false, panel_ready:false, panel_visible:false,
            panel_pending:Vec::new(), dialogs:HashMap::new(), fade:None, memory_log, presentation:Default::default() })
    }

    pub fn close_requested(&self) -> bool {
        // App.as dispatchEvent returns false when its cancelable event is canceled.
        !matches!(dispatch(&self.host, "app", "closeRequest", Value::Null), ExternalValue::Bool(false))
    }

    pub fn resized(&mut self) {
        if let Err(error) = self.web.resize() { tracing::error!("WebView resize: {error}"); }
        // The internal viewport is fixed, so AS may not send a new web.frame.
        let (window,top,frames)={
            let host=self.host.borrow();
            let frames=host.web.iter().filter_map(|(id,info)|info.render_rect.map(|rect|(id.clone(),rect))).collect::<Vec<_>>();
            let top=if host.window.fullscreen().is_some(){0.0}else{host.toolbar_height};
            (host.window.clone(),top,frames)
        };
        let scale=window.scale_factor();
        for (id,rect) in frames {
            let rect=crate::gui::GuiController::aoqi_movie_rect_to_window(window.inner_size(),top*scale,rect);
            if let Err(error)=self.web.execute(&window,WebCommand::Frame{id,x:rect[0]/scale,y:rect[1]/scale,width:rect[2]/scale,height:rect[3]/scale}) {
                tracing::error!("WebView frame update failed: {error}");
            }
        }
        for id in self.dialogs.values().cloned().collect::<Vec<_>>() {let _=self.fill_view(&id);}
        dispatch(&self.host, "app", "resize", Value::Null);
    }

    pub fn pump(&mut self, event_loop: &ActiveEventLoop, presentation: crate::gui::PresentationStats) {
        self.presentation = presentation;
        // A bounded batch gives window/input events a turn even if AS emits more work.
        for _ in 0..128 {
            let command = self.host.borrow_mut().commands.pop_front();
            let Some(command) = command else { break; };
            let window = self.host.borrow().window.clone();
            match command {
                Command::Shell(action) => {
                    if let Err(error) = self.shell_action(action) { tracing::error!("Shell action: {error}"); }
                }
                Command::Web(command) => {
                    if let Err(error) = self.web.execute(&window, command) {
                        tracing::error!("WebView command failed: {error}");
                    }
                }
                Command::Exit => event_loop.exit(),
                Command::Restart => {
                    dispatch(&self.host, "app", "terminating", Value::Null);
                    self.web.dispose_all();
                    self.panel_created = false;
                    self.panel_ready = false;
                    self.panel_visible = false;
                    self.panel_pending.clear();
                    self.dialogs.clear();
                    let mut host = self.host.borrow_mut();
                    host.web.clear();
                    host.commands.clear();
                    host.epoch = host.epoch.saturating_add(1);
                    host.callback_ready = false;
                    host.time_scale = 1.0;
                    host.helper = Helper::default();
                    let _ = host.proxy.send_event(RuffleEvent::AoqiRestart);
                }
                Command::Reveal(path) => {
                    let result = std::process::Command::new("explorer.exe").arg(path).spawn();
                    if let Err(error) = result { tracing::error!("Reveal failed: {error}"); }
                }
                Command::Center => {
                    if let Some(monitor) = window.current_monitor() {
                        let p = monitor.position();
                        let m = monitor.size();
                        let w = window.outer_size();
                        window.set_outer_position(PhysicalPosition::new(
                            p.x + (m.width as i32 - w.width as i32) / 2,
                            p.y + (m.height as i32 - w.height as i32) / 2,
                        ));
                    }
                }
            }
        }
        while let Ok(event) = self.events.try_recv() {
            if event.id == "native-panel" {
                if event.kind == "failed" { tracing::error!("Native panel failed: {}",event.payload); }
                if event.kind == "message" {
                    self.panel_ready = true;
                    let actions = self.host.borrow_mut().shell.panel_message(&event.payload);
                    match actions {
                        Ok(actions) => for action in actions { self.host.borrow_mut().queue(Command::Shell(action)); },
                        Err(error) => tracing::error!("Panel message: {error}"),
                    }
                    let pending = std::mem::take(&mut self.panel_pending);
                    for message in pending { self.host.borrow_mut().queue(Command::Shell(ShellAction::PanelMessage(message))); }
                }
                continue;
            }
            if self.dialogs.values().any(|id| id == &event.id) {
                if event.kind == "message" {
                    let data = if let Some(text) = event.payload.as_str() { serde_json::from_str(text).unwrap_or(Value::Null) } else { event.payload };
                    if let Some(key) = data["key"].as_str() {
                        if self.dialogs.get(key) == Some(&event.id) {
                            let actions = self.host.borrow_mut().shell.dialog_result(key, &data["token"], data["index"].as_i64().unwrap_or(-1));
                            self.host.borrow_mut().queue(Command::Shell(ShellAction::CloseDialog { key:key.into() }));
                            for action in actions { self.host.borrow_mut().queue(Command::Shell(action)); }
                        }
                    }
                }
                continue;
            }
            {
                let mut host = self.host.borrow_mut();
                if let Some(info) = host.web.get_mut(&event.id) {
                    if event.kind == "title" { info.title = event.payload.as_str().unwrap_or_default().into(); }
                    if matches!(event.kind, "navigated" | "loaded") { info.url = event.payload.as_str().unwrap_or_default().into(); }
                }
            }
            if event.kind != "title" {
                dispatch(&self.host, &format!("web:{}", event.id), event.kind, event.payload);
            }
        }
        if self.panel_visible {
            let _ = self.fill_view("native-panel");
        }
        if let Some((start, duration)) = self.fade {
            let progress = if duration.is_zero() { 1.0 } else { (start.elapsed().as_secs_f64() / duration.as_secs_f64()).min(1.0) };
            if let Err(error) = set_alpha(&self.host.borrow().window, (255.0 * (1.0-progress)) as u8) { tracing::error!("Exit fade: {error}"); }
            if progress >= 1.0 { self.fade = None; }
        }
        let mut host = self.host.borrow_mut();
        if host.callback_ready && !host.ready {
            match host.recovery.mark_ready() {
                Ok(()) => host.ready = true,
                Err(error) => tracing::error!("Startup marker: {error}"),
            }
        }
        drop(host);
        self.record_memory(false);
    }

    pub fn finish(&mut self, presentation: crate::gui::PresentationStats) {
        self.presentation = presentation;
        self.record_memory(true);
        dispatch(&self.host, "app", "terminating", Value::Null);
        self.web.dispose_all();
        if let Err(error) = self.host.borrow_mut().recovery.finish_clean() {
            tracing::error!("Clean shutdown marker: {error}");
        }
        CURRENT.with(|current| *current.borrow_mut() = None);
    }

    fn record_memory(&mut self, final_sample: bool) {
        let Some(log)=&mut self.memory_log else {return;};
        if !final_sample && !log.due() {return;}
        let (player,window)={let host=self.host.borrow();(host.player.upgrade(),host.window.clone())};
        let Some(player)=player else {return;};
        if let Ok(player)=player.try_lock() {
            let process=process_memory().unwrap_or_else(|error|json!({"available":false,"error":error}));
            if let Err(error)=log.sample(&player,&window,process,self.presentation,final_sample) {
                tracing::error!("Resource diagnostics write failed: {error}");
            }
        }
    }

    fn fill_view(&mut self, id: &str) -> Result<(), String> {
        let window = self.host.borrow().window.clone();
        let size = window.inner_size().to_logical::<f64>(window.scale_factor());
        let top=if window.fullscreen().is_some(){0.0}else{self.host.borrow().toolbar_height};
        self.web.execute(&window, WebCommand::Frame { id:id.into(), x:0.0,y:top,width:size.width,height:(size.height-top).max(1.0) })
    }

    fn shell_action(&mut self, action: ShellAction) -> Result<(), String> {
        let window = self.host.borrow().window.clone();
        match action {
            ShellAction::Dispatch { kind, payload } => { dispatch(&self.host,"shell",kind,payload); }
            ShellAction::ShowPanel { tab } => {
                if !self.panel_created {
                    let url = {
                        let host = self.host.borrow();
                        let html = aoqi_assets::get("panel/index.html").ok_or("Embedded panel missing")?;
                        host.storage.call("storage.writeText", &[json!(".host-state/panel/index.html"),json!(std::str::from_utf8(html.bytes).map_err(|e|e.to_string())?)])?;
                        url::Url::from_file_path(host.storage.resolve(".host-state/panel/index.html")?).map_err(|_|"Invalid panel path")?.to_string()
                    };
                    self.web.execute(&window,WebCommand::Create{id:"native-panel".into()})?;
                    self.web.execute(&window,WebCommand::Load{id:"native-panel".into(),url})?;
                    self.panel_created = true;
                }
                self.panel_visible = true;
                self.fill_view("native-panel")?;
                if let Some(id)=tab { self.shell_action(ShellAction::PanelMessage(json!({"type":"select-tab","id":id})))?; }
            }
            ShellAction::HidePanel => {
                self.panel_visible=false;
                if self.panel_created { self.web.execute(&window,WebCommand::Frame{id:"native-panel".into(),x:0.0,y:0.0,width:0.0,height:0.0})?; }
            }
            ShellAction::FocusGame => window.focus_window(),
            ShellAction::PanelMessage(message) => {
                if self.panel_created && self.panel_ready {
                    self.web.execute(&window,WebCommand::PostJson{id:"native-panel".into(),message})?;
                } else if self.panel_created { self.panel_pending.push(message); }
            }
            ShellAction::ShowDialog(data) => {
                let key=data["key"].as_str().ok_or("Missing dialog key")?.to_owned();
                if let Some(id)=self.dialogs.remove(&key) { self.web.execute(&window,WebCommand::Dispose{id})?; }
                let (id,url) = {
                    let mut host=self.host.borrow_mut();
                    host.serial+=1;
                    let id=format!("native-dialog-{}",host.serial);
                    let path=format!(".host-state/dialogs/{id}.html");
                    let payload=data.to_string().replace('<',"\\u003c");
                    let html=format!(r#"<!doctype html><meta charset="utf-8"><style>body{{margin:0;background:#0006;font:17px 'Segoe UI','Microsoft YaHei';display:grid;place-items:center;height:100vh}}main{{background:white;border-radius:12px;padding:28px;max-width:650px;min-width:280px}}p{{white-space:pre-wrap}}button{{font:inherit;margin:6px;padding:8px 20px}}</style><main><h2></h2><p></p><footer></footer></main><script>const d={payload};document.querySelector('h2').textContent=d.title;document.querySelector('p').textContent=d.message;d.labels.forEach((label,index)=>{{let b=document.createElement('button');b.textContent=label;b.onclick=()=>chrome.webview.postMessage({{key:d.key,token:d.token,index}});document.querySelector('footer').appendChild(b)}});</script>"#);
                    host.storage.call("storage.writeText",&[json!(path),json!(html)])?;
                    let url=url::Url::from_file_path(host.storage.resolve(&path)?).map_err(|_|"Invalid dialog path")?.to_string();
                    (id,url)
                };
                self.web.execute(&window,WebCommand::Create{id:id.clone()})?;
                self.web.execute(&window,WebCommand::Load{id:id.clone(),url})?;
                self.fill_view(&id)?;
                self.dialogs.insert(key,id);
            }
            ShellAction::CloseDialog { key } => {
                if let Some(id)=self.dialogs.remove(&key) { self.web.execute(&window,WebCommand::Dispose{id})?; }
            }
            ShellAction::ConfirmPanel { request, message } => {
                let confirmed=rfd::MessageDialog::new().set_title("确认操作").set_description(message)
                    .set_buttons(rfd::MessageButtons::YesNo).show()==rfd::MessageDialogResult::Yes;
                let action=self.host.borrow().shell.panel_confirm(request,confirmed);
                self.host.borrow_mut().queue(Command::Shell(action));
            }
            ShellAction::Loading { message,visible,animated } => {
                window.set_title(&if visible {format!("Air奥奇传说 · {message}{}",if animated {"…"}else{""})} else {"Air奥奇传说 · 纹理池与占用测试版".into()});
            }
            ShellAction::SizeContent { width,height,center } => {
                if !(16.0..=16384.0).contains(&width) || !(16.0..=16384.0).contains(&height) {return Err("Invalid content size".into());}
                let toolbar=if window.fullscreen().is_some(){0.0}else{self.host.borrow().toolbar_height};
                let _=window.request_inner_size(LogicalSize::new(width,height+toolbar));
                if center {self.host.borrow_mut().queue(Command::Center);}
            }
            ShellAction::CenterWindow => self.host.borrow_mut().queue(Command::Center),
            ShellAction::PrepareExit => { self.web.dispose_all();self.panel_created=false;self.panel_ready=false;self.panel_visible=false;self.dialogs.clear(); }
            ShellAction::BeginExit => window.set_title("Air奥奇传说 · 正在退出"),
            ShellAction::FadeExit { seconds } => self.fade=Some((std::time::Instant::now(),std::time::Duration::from_secs_f64(seconds.min(30.0)))),
            ShellAction::UpdateCheck => self.shell_action(ShellAction::PanelMessage(json!({"type":"notice","value":"重建版本尚未配置更新源；没有下载或替换任何程序。"})))?,
        }
        Ok(())
    }
}

struct Provider(Rc<RefCell<Host>>);
impl ExternalInterfaceProvider for Provider {
    fn call_method(&self, context: &mut UpdateContext<'_>, name: &str, args: &[ExternalValue]) -> ExternalValue {
        let mut args: Vec<Value> = args.iter().map(to_json).collect();
        if name == "shell.rendererMemory" {
            return from_json(crate::aoqi_diagnostics::renderer_memory(context.renderer));
        }
        if name == "web.frame" && args.len() >= 5 {
            let matrix = context.stage.view_matrix();
            let mut host=self.0.borrow_mut();
            let scale = host.window.scale_factor();
            let top=if host.window.fullscreen().is_some(){0.0}else{host.toolbar_height};
            if let (Some(x),Some(y),Some(width),Some(height)) = (args[1].as_f64(),args[2].as_f64(),args[3].as_f64(),args[4].as_f64()) {
                let internal_rect=[x*f64::from(matrix.a)+matrix.tx.to_pixels(),y*f64::from(matrix.d)+matrix.ty.to_pixels(),
                    width*f64::from(matrix.a),height*f64::from(matrix.d)];
                if let Some(info)=args[0].as_str().and_then(|id|host.web.get_mut(id)) {info.render_rect=Some(internal_rect);}
                let rect=crate::gui::GuiController::aoqi_movie_rect_to_window(
                    host.window.inner_size(),top*scale,
                    internal_rect);
                for (index,value) in rect.into_iter().enumerate() {args[index+1]=json!(value/scale);}
            }
        }
        match self.0.borrow_mut().call(name, &args) {
            Ok(value) => from_json(value),
            Err(error) => {
                // Never include args: web/storage/core calls may carry login data.
                tracing::error!("Native bridge {name} failed: {error}");
                if name == "shell.panelExecute" { ExternalValue::String(error) }
                else if name == "shell.panelPrefs" { from_json(json!({"error":error})) }
                else { ExternalValue::Null }
            }
        }
    }
    fn on_callback_available(&self, name: &str) {
        if name == "hostDispatch" { self.0.borrow_mut().callback_ready = true; }
    }
    fn get_id(&self) -> Option<String> { Some("aoqi-reconstructed-host".into()) }
}

impl Host {
    fn queue(&mut self, command: Command) {
        self.commands.push_back(command);
        let _ = self.proxy.send_event(RuffleEvent::AoqiWake);
    }

    fn call(&mut self, name: &str, args: &[Value]) -> Result<Value, String> {
        if name.starts_with("storage.") {
            let result = self.storage.call(name, args)?;
            if name == "storage.reveal" {
                if let Some(path) = result["path"].as_str() { self.queue(Command::Reveal(path.into())); }
            }
            return Ok(result);
        }
        match name {
            "app.arguments" => Ok(json!(self.arguments)),
            "app.runtimeEpoch" => Ok(json!(self.epoch)),
            "app.runtimeVersion" => Ok(json!("1.0.3-stage3-test-20261104")),
            "app.windowActive" => Ok(json!(self.window.has_focus())),
            "app.fullScreen" => Ok(json!(self.window.fullscreen().is_some())),
            "app.showWindow" => { self.window.set_visible(true); Ok(Value::Null) }
            "app.hideWindow" => { self.window.set_visible(false); Ok(Value::Null) }
            "app.activateWindow" => { self.window.focus_window(); Ok(Value::Null) }
            "app.exit" => { self.queue(Command::Exit); Ok(Value::Null) }
            "app.restart" => { self.queue(Command::Restart); Ok(Value::Null) }
            "app.stageLoginHandoff" => {
                let text = string(args, 0)?;
                let _: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
                self.handoff = Some(text.into()); Ok(Value::Null)
            }
            "app.takeLoginHandoff" => {
                let result = self.handoff.take();
                Ok(if self.recovery.safe_mode { Value::Null } else { json!(result) })
            }
            "app.clearLoginHandoff" => { self.handoff = None; Ok(Value::Null) }
            "core.call" => Ok(json!(self.helper.call(string(args, 0)?))),
            "ruffle.timeScale" => {
                let value = number(args, 0)?;
                if !(0.0..=32.0).contains(&value) { return Err("timeScale must be between 0 and 32".into()); }
                self.time_scale = value; Ok(json!(value))
            }
            "ruffle.cacheState" => {
                let cache=ruffle_frontend_utils::backends::aoqi_cache::AoqiCache::new(self.storage.root().join("cache"))
                    .map_err(|e|e.to_string())?;
                Ok(json!({"root":self.storage.root().join("cache"),"cache":cache.stats().map_err(|e|e.to_string())?,
                    "migration":self.storage.read_json(".host-state/cache-migration.json")?,"recovery":self.recovery.summary()}))
            }
            "web.create" => {
                self.serial += 1;
                let id = format!("web-{}-{}", self.epoch, self.serial);
                self.web.insert(id.clone(), WebInfo::default());
                self.queue(Command::Web(WebCommand::Create { id: id.clone() }));
                Ok(json!(id))
            }
            "web.url" | "web.title" => {
                let view = self.web.get(string(args, 0)?).ok_or("Unknown WebView")?;
                Ok(json!(if name == "web.url" { &view.url } else { &view.title }))
            }
            "web.load" | "web.run" | "web.post" | "web.frame" | "web.dispose" => {
                let id = string(args, 0)?.to_owned();
                if !self.web.contains_key(&id) { return Err("Unknown WebView".into()); }
                let command = match name {
                    "web.load" => WebCommand::Load { id, url: string(args, 1)?.into() },
                    "web.run" => WebCommand::Run { id, script: string(args, 1)?.into() },
                    "web.post" => WebCommand::Post { id, message: string(args, 1)?.into() },
                    "web.frame" => WebCommand::Frame { id, x: number(args, 1)?, y: number(args, 2)?, width: number(args, 3)?, height: number(args, 4)? },
                    _ => { self.web.remove(&id); WebCommand::Dispose { id } }
                };
                self.queue(Command::Web(command)); Ok(Value::Null)
            }
            "shell.centerWindow" => { self.queue(Command::Center); Ok(Value::Null) }
            "shell.inspect" => Ok(json!({"available":true,"architecture":std::env::consts::ARCH,"translated":false,"reconstructed":true,
                "fixedGraphicsProfile":crate::aoqi_diagnostics::fixed_profile(),"recovery":self.recovery.summary()})),
            "shell.payloadState" => Ok(json!({"embedded":aoqi_assets::ASSETS.iter().map(|a| a.path).collect::<Vec<_>>(),"reconstructed":true})),
            "shell.processMemory" => process_memory(),
            _ if name.starts_with("shell.") => {
                let (value,actions)=self.shell.call(name,args,&self.storage)?;
                for action in actions {self.queue(Command::Shell(action));}
                Ok(value)
            }
            _ => Err(format!("Unsupported native bridge: {name}")),
        }
    }
}

fn string(args: &[Value], index: usize) -> Result<&str, String> {
    args.get(index).and_then(Value::as_str).ok_or_else(|| format!("Argument {index} must be a string"))
}

pub(crate) fn process_memory() -> Result<Value, String> {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast(), counters.cb) };
    if ok == 0 { return Err(std::io::Error::last_os_error().to_string()); }
    Ok(json!({"available":true,"workingSet":counters.WorkingSetSize,"privateBytes":counters.PrivateUsage,"peakWorkingSet":counters.PeakWorkingSetSize,"pagefileUsage":counters.PagefileUsage}))
}

fn set_alpha(window:&Window,alpha:u8)->Result<(),String>{
    use winit::raw_window_handle::{HasWindowHandle,RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW,SetWindowLongPtrW,SetLayeredWindowAttributes,GWL_EXSTYLE,WS_EX_LAYERED,LWA_ALPHA};
    let RawWindowHandle::Win32(handle)=window.window_handle().map_err(|e|e.to_string())?.as_raw() else {return Err("Not a Windows window".into())};
    let hwnd=handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let style=GetWindowLongPtrW(hwnd,GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd,GWL_EXSTYLE,style | WS_EX_LAYERED as isize);
        if SetLayeredWindowAttributes(hwnd,0,alpha,LWA_ALPHA)==0{return Err(std::io::Error::last_os_error().to_string());}
    }
    Ok(())
}
fn number(args: &[Value], index: usize) -> Result<f64, String> {
    args.get(index).and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| format!("Argument {index} must be finite"))
}
fn to_json(value: &ExternalValue) -> Value {
    match value {
        ExternalValue::Undefined | ExternalValue::Null => Value::Null,
        ExternalValue::Bool(v) => json!(v), ExternalValue::Number(v) => json!(v),
        ExternalValue::String(v) => json!(v),
        ExternalValue::List(v) => Value::Array(v.iter().map(to_json).collect()),
        ExternalValue::Object(v) => Value::Object(v.iter().map(|(k,v)| (k.clone(),to_json(v))).collect()),
    }
}
fn from_json(value: Value) -> ExternalValue {
    match value {
        Value::Null => ExternalValue::Null,
        Value::Bool(v) => ExternalValue::Bool(v),
        Value::Number(v) => ExternalValue::Number(v.as_f64().unwrap_or(f64::NAN)),
        Value::String(v) => ExternalValue::String(v),
        Value::Array(v) => ExternalValue::List(v.into_iter().map(from_json).collect()),
        Value::Object(v) => ExternalValue::Object(v.into_iter().map(|(k,v)| (k,from_json(v))).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bridge_values_roundtrip() {
        // ExternalInterface has one f64 number variant, not a separate JSON integer.
        let value = json!({"array":[null,true,12.5,"中文"],"nested":{"x":9.0}});
        assert_eq!(to_json(&from_json(value.clone())),value);
    }
}
