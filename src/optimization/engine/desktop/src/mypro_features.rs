//! Display policies already implemented by the versioned notice and battle SWFs.
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
pub mod morph;
pub mod network;
pub mod background;
pub(crate) mod workshop;
pub mod game_ui;
pub mod lineups;
mod battle_state;
mod packet_log;
mod protocol;

pub fn battle_event(event:&str,id:u64) {
    match event {"battle-enter"=>morph::battle(true,id),"battle-exit"=>morph::battle(false,id),_=>{}}
}

const OPTIONS: [(&str, Option<&str>); 8] = [
    ("hideNotice", Some("AOLA_HIDE_NOTICE")),
    ("blockWanyu", Some("AOLA_BLOCK_SKILL")),
    ("hideTransformPortrait", Some("AOLA_HIDE_TRANSFORM_PORTRAIT")),
    ("skipBattleBackdrop", Some("AOLA_SKIP_BATTLE_BACKDROP")),
    ("hideBattleDamage", Some("AOLA_HIDE_BATTLE_DAMAGE")),
    ("hideBattleHealing", Some("AOLA_HIDE_BATTLE_HEALING")),
    ("skipBattleAnimations", Some("AOLA_SKIP_BATTLE_ANIMATIONS")),
    ("battleInputNormalFps", None),
];
const BATTLE_INPUT_NORMAL_FPS: u16 = 1 << 7;
const INITIALIZED: u16 = 1 << 15;
static DISPLAY: AtomicU16 = AtomicU16::new(0);

fn publish(bits: u16) {
    DISPLAY.store(INITIALIZED | bits, Ordering::Relaxed);
    ruffle_core::mypro_observer::set_battle_input_normal_fps(bits & BATTLE_INPUT_NORMAL_FPS != 0);
}

pub fn setting(index: usize, fallback: bool) -> bool {
    let bits = DISPLAY.load(Ordering::Relaxed);
    if bits & INITIALIZED == 0 { fallback } else { bits & (1 << index) != 0 }
}

fn values(bits: u16) -> Value {
    Value::Object(OPTIONS.iter().enumerate().map(|(i, (name, _))|
        ((*name).to_owned(), Value::Bool(bits & (1 << i) != 0))).collect())
}

fn edit(bits: u16, v: &Value) -> Result<u16, String> {
    let object = v.as_object().filter(|o| !o.is_empty()).ok_or("InvalidArguments: values")?;
    let mut next=bits;
    for (key, value) in object {
        let index=OPTIONS.iter().position(|(k,_)| k==key).ok_or("InvalidArguments: unknown setting")?;
        if value.as_bool().ok_or("InvalidArguments: expected boolean")? {next |= 1 << index;} else {next &= !(1 << index);}
    }
    Ok(next)
}

fn read_preferences(root: &Path, defaults: u16) -> Result<(u64,u16), String> {
    let file=match fs::File::open(root.join("display.json")) {
        Ok(f)=>f, Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok((0,defaults)), Err(e)=>return Err(e.to_string()),
    };
    let mut buf=Vec::new();
    file.take(8193).read_to_end(&mut buf).map_err(|e|e.to_string())?;
    if buf.len()>8192 {return Err("Preferences too large".into());}
    let v:Value=serde_json::from_slice(&buf).map_err(|e|e.to_string())?;
    let count=match v["schemaVersion"].as_u64() {Some(1)=>4,Some(2)=>7,Some(3)=>OPTIONS.len(),_=>return Err("Invalid preferences format".into())};
    let fields=v["values"].as_object().ok_or("Invalid preferences format")?;
    if fields.len()!=count || !OPTIONS[..count].iter().all(|(key,_)|fields.contains_key(*key)) {return Err("Invalid preferences format".into());}
    Ok((v["revision"].as_u64().ok_or("Invalid revision")?,edit(defaults,&v["values"])?))
}

fn save(root: &Path, revision:u64,bits:u16,defaults:u16) -> Result<(),String> {
    fs::create_dir_all(root).map_err(|e|e.to_string())?;
    let guard=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(root.join("display.lock")).map_err(|e|e.to_string())?;
    guard.try_lock().map_err(|_|"Busy: another writer")?;
    if read_preferences(root,defaults)?.0 != revision {return Err("StaleContext: display revision".into());}
    let tmp=root.join(format!("display-{}.tmp",std::process::id()));
    let path=root.join("display.json");
    let write=|| -> io::Result<()> {
        let mut f=fs::File::create(&tmp)?;
        serde_json::to_writer_pretty(&mut f,&json!({"schemaVersion":3,"revision":revision+1,"values":values(bits)}))?;
        f.write_all(b"\n")?; f.sync_all()?; drop(f);
        if path.exists() {fs::copy(&path,root.join("display.previous.json"))?;}
        fs::rename(&tmp,&path)
    };
    let result=write(); if result.is_err() {let _=fs::remove_file(tmp);}
    result.map_err(|e|e.to_string())
}

struct DisplayState { revision:u64, bits:u16, stage:&'static str, error:Option<String> }
impl DisplayState {
    fn snapshot(&self) -> Value {json!({"revision":self.revision,"values":values(self.bits),"stage":self.stage,"error":self.error})}
    fn finish(&mut self, result:Result<(u64,u16),String>) {
        match result {
            Ok((rev,bits))=>{self.revision=rev; self.bits=bits; self.stage="ready"; self.error=None; publish(bits);}
            Err(err)=>{self.stage="failed"; self.error=Some(err);}
        }
    }
}

pub struct Features {
    root:PathBuf, defaults:u16, state:Arc<Mutex<DisplayState>>,
}
impl Features {
    pub fn new(root:PathBuf)->Self {
        let root=root.join("mypro");
        morph::init(root.clone());
        background::init(&root);
        let defaults=OPTIONS.iter().enumerate().fold(0,|bits,(i,(_,env))|
            bits | (u16::from(env.is_none_or(|env|std::env::var(env).as_deref()==Ok("1"))) << i));
        publish(defaults);
        let state=Arc::new(Mutex::new(DisplayState{revision:0,bits:defaults,stage:"loading",error:None}));
        let worker=state.clone(); let path=root.clone();
        std::thread::spawn(move || {let r=read_preferences(&path,defaults);worker.lock().unwrap().finish(r);});
        Self {root,defaults,state}
    }

    pub fn capabilities(&self)->Value {
        json!({"settings.read":{"available":true,"reason":""},"settings.patch":{"available":true,"reason":""},
            "workshop.validate":{"available":true,"reason":""},
            "background.read":{"available":true},"background.choose":{"available":true},"background.clear":{"available":true},
            "morph.rules.read":{"available":true,"reason":""},"morph.rules.replace":{"available":true,"reason":""},
            "morph.restore":{"available":true,"reason":""}})
    }
    pub fn set_session(&mut self,_session:&str) {morph::session();}

    pub fn command(&mut self,name:&str,payload:&Value)->Option<Result<Value,String>> {
        match name {
            "settings.read"=>Some(Ok(self.state.lock().unwrap().snapshot())),
            "settings.patch"=>Some(self.patch(payload)),
            "workshop.validate"=>Some(workshop::validate(payload)),
            _=>background::command(name).or_else(||morph::command(&self.root,name,payload)),
        }
    }
    fn patch(&mut self,payload:&Value)->Result<Value,String> {
        let mut s=self.state.lock().unwrap();
        if matches!(s.stage,"loading"|"saving") {return Err("Busy: display settings unavailable".into());}
        if payload["expectedRevision"].as_u64()!=Some(s.revision) {return Err("StaleContext: display revision".into());}
        let bits=edit(s.bits,&payload["values"])?;
        if bits==s.bits {return Ok(s.snapshot());}
        let next_revision=s.revision.checked_add(1).ok_or("InvalidArguments: revision")?;
        let revision=s.revision; let root=self.root.clone(); let defaults=self.defaults; let state=self.state.clone();
        let previous=s.bits;
        s.bits=bits; publish(bits);
        s.stage="saving";s.error=None;let pending=s.snapshot();drop(s);
        std::thread::spawn(move || {
            let result=save(&root,revision,bits,defaults).map(|()|(next_revision,bits));
            let latest=result.as_ref().err().filter(|e|e.starts_with("StaleContext")).and_then(|_|read_preferences(&root,defaults).ok());
            let mut s=state.lock().unwrap();
            if result.is_err() {s.finish(Ok(latest.unwrap_or((revision,previous))));}
            s.finish(result);
        });
        Ok(pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_mypro_display_settings_preserve_other_bits_and_revision() {
        let root=std::env::temp_dir().join(format!("aola-mypro-prefs-{}-{}",std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let bits=edit(4,&json!({"skipBattleBackdrop":true})).unwrap();
        assert_eq!(bits,12);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("display.json"),r#"{"schemaVersion":1,"revision":0,"values":{"hideNotice":false,"blockWanyu":false,"hideTransformPortrait":true,"skipBattleBackdrop":true}}"#).unwrap();
        let defaults = 64 | BATTLE_INPUT_NORMAL_FPS;
        assert_eq!(read_preferences(&root,defaults).unwrap(),(0,76 | BATTLE_INPUT_NORMAL_FPS));
        let mut legacy = values(bits);
        legacy.as_object_mut().unwrap().remove("battleInputNormalFps");
        fs::write(root.join("display.json"),json!({"schemaVersion":2,"revision":0,"values":legacy}).to_string()).unwrap();
        assert_eq!(read_preferences(&root,defaults).unwrap(),(0,bits | BATTLE_INPUT_NORMAL_FPS));
        assert!(edit(bits,&json!({"blockWanyu":1})).is_err());
        let disabled = edit(bits | BATTLE_INPUT_NORMAL_FPS,&json!({"battleInputNormalFps":false})).unwrap();
        assert_eq!(disabled,bits);
        save(&root,0,disabled,defaults).unwrap();
        assert_eq!(read_preferences(&root,defaults).unwrap(),(1,bits));
        assert!(save(&root,0,4,defaults).is_err());
        let enabled = edit(4,&json!({"battleInputNormalFps":true})).unwrap();
        save(&root,1,enabled,defaults).unwrap();
        assert_eq!(read_preferences(&root,defaults).unwrap(),(2,4 | BATTLE_INPUT_NORMAL_FPS));
        assert!(root.join("display.previous.json").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
