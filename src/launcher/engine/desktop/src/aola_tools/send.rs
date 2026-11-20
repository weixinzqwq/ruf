//! Text plans and a non-blocking queue. Only the host may submit returned requests.
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub extension_id: u16,
    pub command: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step { Send(Request), Delay(Duration) }

pub fn parse(input: &str) -> Result<Vec<Step>, String> {
    if input.len() > 1024 * 1024 { return Err("文本不能超过 1 MiB".into()); }
    let text = input.trim_start_matches('\u{feff}').lines()
        .map(|line| if line.trim_start().starts_with("//") { "" } else { line })
        .collect::<Vec<_>>().join("\n");
    let mut steps = Vec::new();
    let (mut quoted, mut escaped, mut start) = (false, false, 0);
    let (mut line, mut start_line) = (1, 1);
    for (at, c) in text.char_indices().chain(std::iter::once((text.len(), '|'))) {
        if at == text.len() || (c == '|' && !quoted) {
            let part = text[start..at].trim();
            if !part.is_empty() {
                let step_line = start_line + text[start..at].chars().take_while(|c| c.is_whitespace()).filter(|c| *c == '\n').count();
                steps.push(parse_step(part).map_err(|error| format!("第 {step_line} 行：{error}"))?);
            }
            start = at + c.len_utf8();
            start_line = line;
        } else if quoted {
            if escaped { escaped = false; }
            else if c == '\\' { escaped = true; }
            else if c == '"' { quoted = false; }
        } else if c == '"' { quoted = true; }
        if c == '\n' { line += 1; }
    }
    if steps.is_empty() { return Err("请输入 #send 或 #time 指令".into()); }
    Ok(steps)
}

fn parse_step(text: &str) -> Result<Step, String> {
    if text == "#wait" { return Err("首期尚未支持 #wait；它需要战斗结束识别".into()); }
    if let Some(delay) = text.strip_prefix("#time") {
        let seconds = delay.trim().strip_prefix('=').ok_or("延时格式为 #time=秒数|")?
            .trim().parse::<f64>().map_err(|_| "延时必须为非负秒数")?;
        let duration = Duration::try_from_secs_f64(seconds).map_err(|_| "延时必须为有限非负秒数")?;
        if Instant::now().checked_add(duration).is_none() { return Err("延时过大".into()); }
        return Ok(Step::Delay(duration));
    }
    let json = text.strip_prefix("#send=").ok_or("未知指令，支持 #send= 和 #time=")?;
    let mut value: Value = serde_json::from_str::<UniqueJson>(json).map_err(|e| format!("JSON 格式错误：{e}"))?.0;
    let object = value.as_object_mut().ok_or("#send 必须是 JSON 对象")?;
    if object.len() != 3 || !["id", "cmd", "param"].iter().all(|key| object.contains_key(*key)) {
        return Err("#send 必须且只能包含 id、cmd、param".into());
    }
    let extension_id = object["id"].as_u64().and_then(|id| u16::try_from(id).ok()).ok_or("id 必须是 0..65535 的整数")?;
    let command = object["cmd"].as_str().ok_or("cmd 必须是字符串")?.to_owned();
    if command.trim().is_empty() || command != command.trim() || command.encode_utf16().count() > 256 {
        return Err("cmd 必须非空、无首尾空白且不超过 256 个 UTF-16 单元".into());
    }
    let mut parameters = object.remove("param").unwrap();
    parameters.as_object_mut().ok_or("param 必须是 JSON 对象")?.remove(":ext_seq;");
    // Match ActionScript String.length before executing any step of the plan.
    if parameters.to_string().encode_utf16().count() > 262_144 {
        return Err("param 序列化后不能超过 262144 个 UTF-16 单元".into());
    }
    Ok(Step::Send(Request { extension_id, command, parameters }))
}

// Reject duplicate fields at every level instead of silently executing a different plan.
struct UniqueJson(Value);
impl<'de> serde::Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { f.write_str("JSON value") }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value,E> { Ok(UniqueJson(v.into())) }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value,E> { Ok(UniqueJson(v.into())) }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value,E> { Ok(UniqueJson(v.into())) }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value,E> { Ok(UniqueJson(v.into())) }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value,E> { Ok(UniqueJson(v.into())) }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value,E> { Ok(UniqueJson(Value::Null)) }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value,A::Error> {
                let mut values = Vec::new();
                while let Some(v) = seq.next_element::<UniqueJson>()? { values.push(v.0); }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value,A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key,v)) = map.next_entry::<String,UniqueJson>()? {
                    if values.insert(key,v.0).is_some() { return Err(serde::de::Error::custom("重复 JSON 字段")); }
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        d.deserialize_any(Visitor)
    }
}

pub struct Queue {
    steps: Vec<Step>, index: usize, groups_left: u32,
    interval: Duration, group_pause: Duration, due: Instant, next_send: Instant,
    pending: bool, pub submitted: usize,
}

impl Queue {
    pub fn new(steps: Vec<Step>, groups: u32, interval: Duration, group_pause: Duration, now: Instant) -> Self {
        Self { steps, index: 0, groups_left: groups, interval, group_pause, due: now, next_send: now, pending: false, submitted: 0 }
    }
    pub fn done(&self) -> bool { self.groups_left == 0 }
    pub fn poll(&mut self, now: Instant) -> Option<Request> {
        if self.done() || self.pending || now < self.due { return None; }
        // Bound work per GUI frame even for a long sequence of zero delays.
        for _ in 0..256 {
            if self.index == self.steps.len() {
                self.finish_group(now);
                return None;
            }
            let step = &self.steps[self.index];
            match step {
                Step::Send(request) => {
                    if now < self.next_send { return None; }
                    self.index += 1; self.pending = true;
                    return Some(request.clone());
                }
                Step::Delay(duration) => {
                    self.index += 1;
                    self.due = now + *duration;
                    if !duration.is_zero() { return None; }
                }
            }
        }
        None
    }
    pub fn submitted(&mut self, now: Instant) {
        self.pending = false; self.submitted += 1;
        self.due = now;
        self.next_send = now + self.interval;
        // Anchor the group pause to submission, not an arbitrarily delayed GUI poll.
        if self.index == self.steps.len() { self.finish_group(now); }
    }
    fn finish_group(&mut self, now: Instant) {
        self.groups_left = self.groups_left.saturating_sub(1);
        if !self.done() {
            self.index = 0;
            self.due = now + self.group_pause;
            self.next_send = self.due;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_text_quotes_delays_and_sequence_removal() {
        let steps = parse("\u{feff}// note\n#send={\"id\":42,\"cmd\":\"a|b\",\"param\":{\"v\":\"中\\\"文\",\":ext_seq;\":4}}|#time=0.25|").unwrap();
        let Step::Send(request) = &steps[0] else { panic!() };
        assert_eq!(request.command,"a|b"); assert!(request.parameters.get(":ext_seq;").is_none());
        assert_eq!(steps[1],Step::Delay(Duration::from_millis(250)));
    }
    #[test]
    fn aola_text_rejects_bad_input_before_execution() {
        for text in ["#wait|", "#time=-1|", "#time=NaN|", "#send={\"id\":1,\"id\":2,\"cmd\":\"a\",\"param\":{}}|", "#send={\"id\":1,\"cmd\":\"a\",\"param\":{\"x\":1,\"x\":2}}|", "#send={\"id\":1.5,\"cmd\":\"a\",\"param\":{}}|"] {
            assert!(parse(text).is_err(),"{text}");
        }
    }
    #[test]
    fn aola_queue_interval_group_pause_and_completion() {
        let steps=parse("#send={\"id\":1,\"cmd\":\"a\",\"param\":{}}|").unwrap();
        let now=Instant::now();
        let mut q=Queue::new(steps,2,Duration::from_millis(200),Duration::from_secs(1),now);
        assert!(q.poll(now).is_some()); assert!(q.poll(now).is_none()); q.submitted(now);
        assert!(q.poll(now).is_none()); assert!(q.poll(now+Duration::from_millis(999)).is_none());
        assert!(q.poll(now+Duration::from_secs(1)).is_some()); q.submitted(now+Duration::from_secs(1));
        assert!(q.poll(now+Duration::from_secs(1)).is_none()); assert!(q.done());
    }
    #[test]
    fn aola_text_reports_line_after_many_segments() {
        let mut text="#time=0|\n".repeat(8192);
        text.push_str("\n#unknown|");
        assert!(parse(&text).unwrap_err().starts_with("第 8194 行："));
    }
    #[test]
    fn aola_text_matches_bridge_utf16_and_parameter_limits() {
        let line=|command:String,parameters:Value|format!("#send={}|",serde_json::json!({"id":1,"cmd":command,"param":parameters}));
        assert!(parse(&line("😀".repeat(128),serde_json::json!({}))).is_ok());
        assert!(parse(&line("😀".repeat(129),serde_json::json!({}))).is_err());
        assert!(parse(&line("a".into(),serde_json::json!({"v":"x".repeat(262_136)}))).is_ok());
        assert!(parse(&line("a".into(),serde_json::json!({"v":"x".repeat(262_137)}))).is_err());
    }
    #[test]
    fn aola_group_pause_does_not_restart_after_a_stalled_frame() {
        let steps=parse("#send={\"id\":1,\"cmd\":\"a\",\"param\":{}}|").unwrap();
        let now=Instant::now();
        let mut queue=Queue::new(steps,2,Duration::from_millis(200),Duration::from_secs(1),now);
        assert!(queue.poll(now).is_some());
        queue.submitted(now);
        assert!(queue.poll(now+Duration::from_secs(5)).is_some());
    }
    #[test]
    fn aola_explicit_delay_overlaps_the_send_interval() {
        for (seconds,expected_ms) in [(1.0,1000),(0.05,200)] {
            let steps=parse(&format!("#send={{\"id\":1,\"cmd\":\"a\",\"param\":{{}}}}|#time={seconds}|#send={{\"id\":1,\"cmd\":\"b\",\"param\":{{}}}}|")).unwrap();
            let now=Instant::now();
            let mut queue=Queue::new(steps,1,Duration::from_millis(200),Duration::ZERO,now);
            assert!(queue.poll(now).is_some()); queue.submitted(now);
            assert!(queue.poll(now).is_none());
            assert!(queue.poll(now+Duration::from_millis(expected_ms-1)).is_none());
            assert_eq!(queue.poll(now+Duration::from_millis(expected_ms)).unwrap().command,"b");
        }
    }
}
