//! Display policies already implemented by the versioned notice and battle SWFs.
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
pub mod morph;

pub fn battle_event(event:&str,id:u64) {
    match event {"battle-enter"=>morph::battle(true,id),"battle-exit"=>morph::battle(false,id),_=>{}}
}

const OPTIONS: [(&str, &str); 4] = [
    ("hideNotice", "AOLA_HIDE_NOTICE"),
    ("blockWanyu", "AOLA_BLOCK_SKILL"),
    ("hideTransformPortrait", "AOLA_HIDE_TRANSFORM_PORTRAIT"),
    ("skipBattleBackdrop", "AOLA_SKIP_BATTLE_BACKDROP"),
];
const INITIALIZED: u8 = 128;
static DISPLAY: AtomicU8 = AtomicU8::new(0);

pub fn setting(index: usize, fallback: bool) -> bool {
    let bits = DISPLAY.load(Ordering::Relaxed);
    if bits & INITIALIZED == 0 { fallback } else { bits & (1 << index) != 0 }
}

fn values(bits: u8) -> Value {
    Value::Object(OPTIONS.iter().enumerate().map(|(i, (name, _))|
        ((*name).to_owned(), Value::Bool(bits & (1 << i) != 0))).collect())
}

fn edit(bits: u8, v: &Value) -> Result<u8, String> {
    let object = v.as_object().filter(|o| !o.is_empty()).ok_or("InvalidArguments: values")?;
    let mut next=bits;
    for (key, value) in object {
        let index=OPTIONS.iter().position(|(k,_)| k==key).ok_or("InvalidArguments: unknown setting")?;
        if value.as_bool().ok_or("InvalidArguments: expected boolean")? {next |= 1 << index;} else {next &= !(1 << index);}
    }
    Ok(next)
}

fn read_preferences(root: &Path, defaults: u8) -> Result<(u64,u8), String> {
    let file=match fs::File::open(root.join("display.json")) {
        Ok(f)=>f, Err(e) if e.kind()==io::ErrorKind::NotFound=>return Ok((0,defaults)), Err(e)=>return Err(e.to_string()),
    };
    let mut buf=Vec::new();
    file.take(8193).read_to_end(&mut buf).map_err(|e|e.to_string())?;
    if buf.len()>8192 {return Err("Preferences too large".into());}
    let v:Value=serde_json::from_slice(&buf).map_err(|e|e.to_string())?;
    if v["schemaVersion"]!=1 || v["values"].as_object().map(|m|m.len())!=Some(OPTIONS.len()) {return Err("Invalid preferences format".into());}
    Ok((v["revision"].as_u64().ok_or("Invalid revision")?,edit(0,&v["values"])?))
}

fn save(root: &Path, revision:u64,bits:u8,defaults:u8) -> Result<(),String> {
    fs::create_dir_all(root).map_err(|e|e.to_string())?;
    let guard=fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(root.join("display.lock")).map_err(|e|e.to_string())?;
    guard.try_lock().map_err(|_|"Busy: another writer")?;
    if read_preferences(root,defaults)?.0 != revision {return Err("StaleContext: display revision".into());}
    let tmp=root.join(format!("display-{}.tmp",std::process::id()));
    let path=root.join("display.json");
    let write=|| -> io::Result<()> {
        let mut f=fs::File::create(&tmp)?;
        serde_json::to_writer_pretty(&mut f,&json!({"schemaVersion":1,"revision":revision+1,"values":values(bits)}))?;
        f.write_all(b"\n")?; f.sync_all()?; drop(f);
        if path.exists() {fs::copy(&path,root.join("display.previous.json"))?;}
        fs::rename(&tmp,&path)
    };
    let result=write(); if result.is_err() {let _=fs::remove_file(tmp);}
    result.map_err(|e|e.to_string())
}

struct DisplayState { revision:u64, bits:u8, stage:&'static str, error:Option<String> }
impl DisplayState {
    fn snapshot(&self) -> Value {json!({"revision":self.revision,"values":values(self.bits),"stage":self.stage,"error":self.error})}
    fn finish(&mut self, result:Result<(u64,u8),String>) {
        match result {
            Ok((rev,bits))=>{self.revision=rev; self.bits=bits; self.stage="ready"; self.error=None; DISPLAY.store(INITIALIZED|bits,Ordering::Relaxed);}
            Err(err)=>{self.stage="failed"; self.error=Some(err);}
        }
    }
}

pub struct Features {
    root:PathBuf, defaults:u8, state:Arc<Mutex<DisplayState>>,
}
impl Features {
    pub fn new(root:PathBuf)->Self {
        let root=root.join("mypro");
        morph::init(root.clone());
        let defaults=OPTIONS.iter().enumerate().fold(0,|bits,(i,(_,env))|
            bits | (u8::from(std::env::var(env).as_deref()==Ok("1")) << i));
        DISPLAY.store(INITIALIZED|defaults,Ordering::Relaxed);
        let state=Arc::new(Mutex::new(DisplayState{revision:0,bits:defaults,stage:"loading",error:None}));
        let worker=state.clone(); let path=root.clone();
        std::thread::spawn(move || {let r=read_preferences(&path,defaults);worker.lock().unwrap().finish(r);});
        Self {root,defaults,state}
    }

    pub fn capabilities(&self)->Value {
        json!({"settings.read":{"available":true,"reason":""},"settings.patch":{"available":true,"reason":""},
            "morph.rules.read":{"available":true,"reason":""},"morph.rules.replace":{"available":true,"reason":""},
            "morph.restore":{"available":true,"reason":""}})
    }
    pub fn set_session(&mut self,_session:&str) {morph::session();}

    pub fn command(&mut self,name:&str,payload:&Value)->Option<Result<Value,String>> {
        match name {
            "settings.read"=>Some(Ok(self.state.lock().unwrap().snapshot())),
            "settings.patch"=>Some(self.patch(payload)),
            _=>morph::command(&self.root,name,payload),
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
        s.stage="saving";let pending=s.snapshot();drop(s);
        std::thread::spawn(move || {
            let result=save(&root,revision,bits,defaults).map(|()|(next_revision,bits));
            let latest=result.as_ref().err().filter(|e|e.starts_with("StaleContext")).and_then(|_|read_preferences(&root,defaults).ok());
            let mut s=state.lock().unwrap();
            if let Some(snapshot)=latest {s.finish(Ok(snapshot));}
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
        assert!(edit(bits,&json!({"blockWanyu":1})).is_err());
        save(&root,0,bits,4).unwrap();
        assert_eq!(read_preferences(&root,4).unwrap(),(1,12));
        assert!(save(&root,0,4,4).is_err());
        save(&root,1,4,4).unwrap();
        assert!(root.join("display.previous.json").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
