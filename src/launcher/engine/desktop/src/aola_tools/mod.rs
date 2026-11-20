//! Aola-only desktop tools. Network observation never owns the game connection.
pub mod bridge;
pub mod network;
pub mod protocol;
mod logging;
mod send;
mod ui;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use ruffle_core::{Player, external::Value as ExternalValue};
use ruffle_render::quality::StageQuality;
use serde_json::json;
use protocol::{Decoder, Direction};
use ui::{Action, Row, ToolsUi, View};

static COMPLETIONS: OnceLock<Mutex<VecDeque<(String,String,String)>>> = OnceLock::new();
pub fn completed(args: &[ExternalValue]) {
    let get = |i| match args.get(i) { Some(ExternalValue::String(value)) => value.clone(), _ => String::new() };
    let (id,status,code) = (get(0),get(1),get(2));
    if id.len()!=32 || !id.bytes().all(|c|c.is_ascii_hexdigit()) { return; }
    if !matches!(status.as_str(),"ServerSucceeded"|"ServerRejected"|"ServerResponse") { return; }
    let code = code.chars().filter(|c|c.is_ascii_alphanumeric() || "_-".contains(*c)).take(64).collect();
    let mut queue=COMPLETIONS.get_or_init(Default::default).lock().unwrap();
    if queue.len()==256 {queue.pop_front();}
    queue.push_back((id,status,code));
}

enum FileResult { Open(Result<String,String>), Saved(Result<(),String>) }
pub struct Tools {
    ui: ToolsUi, root: PathBuf, decoders: HashMap<u64,Decoder>,
    send_rows: Vec<Row>, receive_rows: Vec<Row>, row_id:u64,
    active_connection:Option<u64>, runner:Option<send::Queue>,
    status:String, bridge_ready:bool, next_probe:Instant,
    logger:Option<logging::PacketLog>, file_result:Option<mpsc::Receiver<FileResult>>,
    submitted_ids:VecDeque<String>,
}

impl Tools {
    pub fn new(root: &Path, frame_rate: Option<f64>) -> Self {
        let mut ui=ToolsUi::default();
        if let Ok(text)=std::fs::read_to_string(root.join("launcher-settings.json"))
            && let Ok(settings)=serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(fps)=settings["fps"].as_f64().filter(|v|v.is_finite() && (1.0..=120.0).contains(v)) {ui.fps=fps;}
            if let Some(q)=settings["quality"].as_str().filter(|q|matches!(*q,"low"|"medium"|"high"|"best")) {ui.quality=q.into();}
            ui.mute=settings["mute"].as_bool().unwrap_or(false);
        }
        if let Some(fps)=frame_rate.filter(|v|v.is_finite() && (1.0..=120.0).contains(v)) {ui.fps=fps;}
        Self { ui, root:root.into(), decoders:HashMap::new(), send_rows:Vec::new(),receive_rows:Vec::new(),row_id:0,
            active_connection:None,runner:None,status:"等待官方登录；日志默认关闭".into(),bridge_ready:false,
            next_probe:Instant::now(),logger:None,file_result:None,submitted_ids:VecDeque::new() }
    }
    pub fn destroyed(&mut self) {
        self.runner=None; self.active_connection=None; self.decoders.clear(); self.bridge_ready=false;
        self.submitted_ids.clear();
        if let Some(q)=COMPLETIONS.get() {q.lock().unwrap().clear();}
        self.status="游戏会话已关闭，发送队列已停止".into();
    }
    pub fn created(&mut self, player:&mut Player) {
        self.destroyed(); self.next_probe=Instant::now();
        self.apply_playback(player);
        self.status="等待游戏连接与发送桥就绪".into();
    }
    fn apply_playback(&self,player:&mut Player) {
        player.mutate_with_update_context(|context| *context.frame_rate=self.ui.fps);
        player.set_quality(match self.ui.quality.as_str() {"low"=>StageQuality::Low,"medium"=>StageQuality::Medium,"best"=>StageQuality::Best,_=>StageQuality::High});
        player.set_volume(if self.ui.mute {0.0} else {1.0});
    }
    fn observe(&mut self) {
        use network::NetworkEvent;
        for event in network::drain() {
            match event {
                NetworkEvent::Opened{connection} => { self.decoders.entry(connection).or_default(); }
                NetworkEvent::Data{connection,direction,bytes} => {
                    let packets=self.decoders.entry(connection).or_default().feed(direction,&bytes);
                    for packet in packets {
                        if packet.direction==Direction::Send && packet.command=="logIn" {
                            self.runner=None;self.submitted_ids.clear();
                            self.active_connection=Some(connection);
                        }
                        self.row_id+=1;
                        let row=Row {id:self.row_id,time:chrono::Local::now().format("%H:%M:%S%.3f").to_string(),connection,
                            command:packet.command,extension_id:packet.extension_id,
                            parameters:packet.parameters.map(|p|p.to_string()).unwrap_or_default(),summary:packet.summary,export:packet.export};
                        if let Some(log)=&mut self.logger {
                            let mut value=json!({"time":row.time,"connection":row.connection,"direction":if packet.direction==Direction::Send {"send"} else {"receive"},"command":row.command,"extensionId":row.extension_id,"summary":row.summary,
                                "parameters":serde_json::from_str::<serde_json::Value>(&row.parameters).unwrap_or(serde_json::Value::Null)});
                            redact(&mut value);
                            if let Err(error)=log.record(value) { self.status=error; }
                        }
                        let rows=if packet.direction==Direction::Send {&mut self.send_rows} else {&mut self.receive_rows};
                        rows.push(row);
                        // Bound both rows and retained text; large packets do not multiply without limit.
                        let mut bytes:usize=rows.iter().map(row_bytes).sum();
                        let mut remove=0;
                        while rows.len()-remove>1000 || bytes>8*1024*1024 {
                            bytes=bytes.saturating_sub(row_bytes(&rows[remove])); remove+=1;
                        }
                        if remove>0 {rows.drain(..remove);}
                    }
                }
                NetworkEvent::Closed{connection} => {
                    self.decoders.remove(&connection);
                    if self.active_connection==Some(connection) {
                        self.active_connection=None;self.runner=None;self.bridge_ready=false;self.submitted_ids.clear();
                        self.status="游戏连接关闭，已停止发送".into();
                    }
                }
                NetworkEvent::Gap{connection} => {
                    self.decoders.entry(connection).or_default().invalidate();
                    if self.active_connection==Some(connection) {self.runner=None;self.active_connection=None;self.submitted_ids.clear();}
                    self.status="观察队列发生丢失，当前连接停止导出和发送；重新登录后恢复".into();
                }
            }
        }
        if let Some(q)=COMPLETIONS.get() {
            for (id,status,code) in q.lock().unwrap().drain(..) {
                if self.submitted_ids.contains(&id) {
                    self.status=format!("{} {}",match status.as_str() {"ServerSucceeded"=>"服务器明确返回成功","ServerRejected"=>"服务器明确拒绝",_=>"收到服务器响应，未判定业务成功"},code);
                    if status=="ServerRejected" {self.runner=None;}
                }
            }
        }
    }
    pub fn tick(&mut self,mut player:Option<&mut Player>) {
        self.observe();
        let file_result=self.file_result.as_ref().map(|rx|rx.try_recv());
        if let Some(Ok(result))=file_result {
            match result {
                FileResult::Open(Ok(text))=>{self.ui.editor=text;self.status="文本已打开，尚未执行".into();},
                FileResult::Saved(Ok(()))=>self.status="文件已保存".into(),
                FileResult::Open(Err(e))|FileResult::Saved(Err(e))=>self.status=e,
            }
            self.file_result=None;
        } else if matches!(file_result,Some(Err(mpsc::TryRecvError::Disconnected))) {
            self.file_result=None;
        }
        let now=Instant::now();
        if now>=self.next_probe {
            self.bridge_ready=player.as_deref_mut().is_some_and(bridge::ready);
            self.next_probe=now+Duration::from_secs(1);
        }
        if let Some(queue)=&mut self.runner {
            if let Some(request)=queue.poll(now) {
                let id=format!("{:032x}",rand::random::<u128>());
                let result=player.as_deref_mut().map(|player|bridge::invoke(player,&id,request.extension_id,&request.command,&request.parameters))
                    .unwrap_or_else(||"GameClosed".into());
                if result=="SubmittedToFlash" {
                    queue.submitted(Instant::now());
                    self.status=format!("已提交给游戏 {} 条；尚未据此判定服务器成功",queue.submitted);
                    self.submitted_ids.push_back(id);
                    if self.submitted_ids.len()>256 {self.submitted_ids.pop_front();}
                } else {
                    self.status=if result=="BattleBridgeRequired" {"1409 需要战斗自动操作桥，首期暂不支持".into()} else {format!("提交停止：{result}")};
                    self.runner=None;
                }
            }
            if self.runner.as_ref().is_some_and(send::Queue::done) {self.runner=None;}
        }
    }
    pub fn update(&mut self,ctx:&egui::Context,ui:&mut egui::Ui,mut player:Option<&mut Player>) {
        let actions=self.ui.show(ctx,ui,&View {send_rows:&self.send_rows,receive_rows:&self.receive_rows,status:&self.status,
            running:self.runner.is_some(),logging:self.logger.is_some(),bridge_ready:self.bridge_ready && self.active_connection.is_some(),
            login_visible:crate::aola_host::login_visible()});
        for action in actions { self.action(action,player.as_deref_mut()); }
        ctx.request_repaint_after(Duration::from_millis(50));
    }
    fn plan(&self)->Result<Vec<send::Step>,String> {
        let steps=send::parse(&self.ui.editor)?;
        for step in &steps {
            if let send::Step::Send(request)=step {
                if bridge::extension_name(request.extension_id).is_none() {return Err(format!("未确认的扩展 ID：{}",request.extension_id));}
                if request.extension_id==13 && request.command=="1409" {return Err("1409 需要战斗自动操作桥，首期暂不支持".into());}
            }
        }
        Ok(steps)
    }
    fn action(&mut self,action:Action,player:Option<&mut Player>) {
        match action {
            Action::Validate=>{self.status=match self.plan(){Ok(steps)=>format!("格式检查通过，共 {} 步；尚未执行",steps.len()),Err(e)=>e};},
            Action::Start=>{
                if self.runner.is_some() {return;}
                if self.file_result.is_some() {self.status="请先关闭文件选择窗口".into();return;}
                if !self.bridge_ready || self.active_connection.is_none() {self.status="请先完成官方登录并等待发送桥就绪".into();return;}
                match self.plan() {
                    Ok(steps)=>{self.submitted_ids.clear();self.runner=Some(send::Queue::new(steps,self.ui.groups.max(1),Duration::from_millis(self.ui.interval_ms),Duration::from_millis(self.ui.group_pause_ms),Instant::now()));self.status="执行已开始".into();},
                    Err(e)=>self.status=e,
                }
            },
            Action::Stop=>{self.runner=None;self.status="已停止后续提交；已提交的命令无法撤回".into();},
            Action::Clear=>{self.send_rows.clear();self.receive_rows.clear();},
            Action::OpenEditor=>if self.runner.is_none(){self.file_dialog(None,false);},
            Action::SaveEditor=>self.file_dialog(Some(self.ui.editor.clone()),false),
            Action::Export(text)=>self.file_dialog(Some(text),true),
            Action::Logging(enabled)=>{
                if enabled {match logging::PacketLog::start(&self.root) {Ok(log)=>{self.status=format!("从现在开始保留脱敏日志：{}",log.path().display());self.logger=Some(log);},Err(e)=>self.status=format!("无法创建日志：{e}")}}
                else if let Some(mut log)=self.logger.take() {self.status=match log.stop(){Ok(())=>format!("日志已关闭；累计丢弃 {} 条",log.dropped()),Err(error)=>error};}
            },
            Action::OpenLogs=>{let path=self.root.join("logs");match std::fs::create_dir_all(&path){Ok(())=>{if let Err(e)=std::process::Command::new("explorer.exe").arg(path).spawn(){self.status=format!("无法打开日志目录：{e}");}},Err(e)=>self.status=e.to_string()}},
            Action::Playback{fps,quality,mute}=>{
                self.ui.fps=fps;self.ui.quality=quality;self.ui.mute=mute;
                if let Some(player)=player {self.apply_playback(player);}
                let settings=json!({"fps":self.ui.fps,"quality":self.ui.quality,"mute":self.ui.mute});
                self.status=match std::fs::write(self.root.join("launcher-settings.json"),settings.to_string()){Ok(())=>"播放设置已保存".into(),Err(e)=>format!("设置未保存：{e}")};
            }
        }
    }
    fn file_dialog(&mut self,text:Option<String>,export:bool) {
        if self.file_result.is_some() {self.status="已有文件选择窗口打开".into();return;}
        let (tx,rx)=mpsc::channel();self.file_result=Some(rx);
        std::thread::spawn(move||{
            let dialog=rfd::FileDialog::new().add_filter("文本",&["txt"]);
            if let Some(text)=text {
                if let Some(path)=dialog.set_file_name(if export {"aola-send.txt"} else {"aola-plan.txt"}).save_file(){let _=tx.send(FileResult::Saved(std::fs::write(path,text).map_err(|e|e.to_string())));}
            } else if let Some(path)=dialog.pick_file() {
                let result=(||{let metadata=std::fs::metadata(&path).map_err(|e|e.to_string())?;if metadata.len()>1024*1024{return Err("文本不能超过 1 MiB".into());}std::fs::read_to_string(path).map_err(|e|e.to_string())})();
                let _=tx.send(FileResult::Open(result));
            }
        });
    }
}
fn row_bytes(row:&Row)->usize {row.parameters.len()+row.summary.len()+row.command.len()+row.export.as_ref().map_or(0,String::len)}
fn redact(value:&mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map)=>for(key,value)in map.iter_mut(){if matches!(key.to_ascii_lowercase().as_str(),"sid"|"sessionid"|"game_auth_sid"|"password"|"passwd"|"cookie"|"authorization"|"access_token"|"refresh_token"){*value="<redacted>".into();}else{redact(value);}},
        serde_json::Value::Array(values)=>for value in values {redact(value);},
        _=>{}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruffle_core::{PlayerBuilder, context::UpdateContext, external::ExternalInterfaceProvider, tag_utils::SwfMovieData};
    struct OfflineProvider;
    impl ExternalInterfaceProvider for OfflineProvider {
        fn call_method(&self,_:&mut UpdateContext<'_>,name:&str,args:&[ExternalValue])->ExternalValue {
            if name=="aolaCommandCompleted" {completed(args);}
            ExternalValue::Undefined
        }
        fn on_callback_available(&self,_:&str) {}
        fn get_id(&self)->Option<String>{Some("offline-tools-test".into())}
    }
    #[test]
    fn aola_tools_run_and_receive_results_without_rendering() {
        network::reset();
        let bytes=include_bytes!("../../../../source/launcher-bridge/fixtures/start-with-mock.swf");
        let movie=SwfMovieData::from_data(bytes,"https://aola.100bt.com/play/start~2107300001.swf".into(),None,None).unwrap();
        let player=PlayerBuilder::new().with_movie(movie).with_external_interface(Box::new(OfflineProvider)).build();
        let mut player=player.lock().unwrap(); player.run_frame();
        let mut tools=Tools::new(Path::new("unused-offline-tools-settings"),None);
        tools.created(&mut player); tools.active_connection=Some(1);
        tools.ui.editor="#send={\"id\":42,\"cmd\":\"fixture-success\",\"param\":{}}|".into();
        tools.tick(Some(&mut player));
        tools.action(Action::Start,Some(&mut player));
        tools.tick(Some(&mut player));
        assert!(tools.status.contains("已提交给游戏"));
        tools.tick(Some(&mut player));
        assert!(tools.status.contains("服务器明确返回成功"));
        assert!(tools.runner.is_none());
        tools.ui.editor="#time=20|".into();
        tools.action(Action::Start,Some(&mut player));
        assert!(tools.runner.is_some());
        tools.destroyed();
        assert!(tools.runner.is_none());assert!(tools.submitted_ids.is_empty());assert!(tools.active_connection.is_none());
    }

    #[test]
    fn aola_tools_saved_rate_and_explicit_override() {
        let root=std::env::temp_dir().join(format!("aola-settings-test-{:032x}",rand::random::<u128>()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(Tools::new(&root,None).ui.fps,50.0);
        std::fs::write(root.join("launcher-settings.json"),r#"{"fps":40,"mute":true}"#).unwrap();
        assert_eq!(Tools::new(&root,None).ui.fps,40.0);
        let mut tools=Tools::new(&root,Some(60.0));
        assert_eq!(tools.ui.fps,60.0);assert!(tools.ui.mute);
        let player=PlayerBuilder::new().with_frame_rate(Some(50.0)).build();
        let mut player=player.lock().unwrap();
        tools.created(&mut player);
        assert!(player.is_frame_rate_forced());assert_eq!(player.frame_rate(),60.0);
        std::fs::remove_file(root.join("launcher-settings.json")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
