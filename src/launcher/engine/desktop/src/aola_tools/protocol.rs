//! Read-only Aola framing and protocol decoding. Never produces socket writes.
//! Each decoder belongs to one connection; a lost observation invalidates it.
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::rc::Rc;

const MAX_FRAME: usize = 4 * 1024 * 1024;
const MAX_BUFFER: usize = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 65_536;
const INNER_MASK: u32 = 0x1fff_fffc;
type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Send,
    Receive,
}

#[derive(Clone, Debug)]
pub struct Packet {
    pub direction: Direction,
    pub command: String,
    pub extension_id: Option<u16>,
    pub parameters: Option<Value>,
    pub summary: String,
    pub export: Option<String>,
}

impl Packet {
    fn notice(direction: Direction, command: &str, summary: impl Into<String>) -> Self {
        Self {
            direction,
            command: command.into(),
            extension_id: None,
            parameters: None,
            summary: summary.into(),
            export: None,
        }
    }
}

pub struct Decoder {
    send: Vec<u8>,
    receive: Vec<u8>,
    user_id: Option<u32>,
    inner_sequences: BTreeMap<u16, u32>,
    export_valid: bool,
    framing_valid: bool,
}

impl Default for Decoder {
    fn default() -> Self {
        Self {
            send: Vec::new(),
            receive: Vec::new(),
            user_id: None,
            inner_sequences: BTreeMap::new(),
            export_valid: true,
            framing_valid: true,
        }
    }
}

impl Decoder {
    /// A gap can split a length prefix. Do not attempt to guess a new boundary.
    pub fn invalidate(&mut self) {
        self.export_valid = false;
        self.framing_valid = false;
        self.user_id = None;
        self.inner_sequences.clear();
        self.send.fill(0);
        self.receive.fill(0);
        self.send.clear();
        self.receive.clear();
    }

    pub fn feed(&mut self, direction: Direction, bytes: &[u8]) -> Vec<Packet> {
        if !self.framing_valid || bytes.is_empty() {
            return Vec::new();
        }
        let mut pending = match direction {
            Direction::Send => std::mem::take(&mut self.send),
            Direction::Receive => std::mem::take(&mut self.receive),
        };
        if bytes.len() > MAX_BUFFER.saturating_sub(pending.len()) {
            pending.fill(0);
            self.invalidate();
            return vec![Packet::notice(direction, "decode-error", "观察缓冲溢出，本连接解析已停止")];
        }
        pending.extend_from_slice(bytes);
        let mut offset = 0;
        let mut packets = Vec::new();
        while offset < pending.len() {
            if packets.len() >= MAX_ITEMS {
                self.invalidate();
                packets.push(Packet::notice(direction, "decode-error", "单次观察消息数超过上限，本连接解析已停止"));
                break;
            }
            let remaining = &pending[offset..];
            // Policy XML is consumed only by this observer. The actual transport
            // still forwards every original byte, including the terminating NUL.
            if remaining[0] == b'<' {
                match remaining.iter().position(|b| *b == 0) {
                    Some(end) if end < 65_536 => {
                        packets.push(Packet::notice(direction, "policy", "Flash 策略交换"));
                        offset += end + 1;
                        continue;
                    }
                    None if remaining.len() < 65_536 => break,
                    _ => {
                        self.invalidate();
                        packets.push(Packet::notice(direction, "decode-error", "策略响应长度异常，本连接解析已停止"));
                        break;
                    }
                }
            }
            if remaining.len() < 4 {
                break;
            }
            let length = u32::from_be_bytes(remaining[..4].try_into().unwrap()) as usize;
            if length == 0 || length > MAX_FRAME {
                self.invalidate();
                packets.push(Packet::notice(direction, "decode-error", "帧长度异常，本连接解析已停止"));
                break;
            }
            if remaining.len() < length + 4 {
                break;
            }
            let frame = &remaining[..length + 4];
            let result = match direction {
                Direction::Send => self.parse_client(frame),
                Direction::Receive => parse_server(frame),
            };
            match result {
                Ok(packet) => packets.push(packet),
                Err(reason) => {
                    self.export_valid = false;
                    packets.push(Packet::notice(direction, "decode-error", reason));
                }
            }
            offset += length + 4;
        }
        if self.framing_valid {
            let tail = pending[offset..].to_vec();
            match direction {
                Direction::Send => self.send = tail,
                Direction::Receive => self.receive = tail,
            }
        }
        // Avoid retaining the consumed login frame in the pending allocation.
        pending.fill(0);
        packets
    }

    fn parse_client(&mut self, frame: &[u8]) -> Result<Packet> {
        if frame.len() < 15 {
            return Err("客户端帧短于包头；本连接发送导出已停用");
        }
        let key = &frame[4..8];
        let mut body: Vec<u8> = frame[8..].iter().enumerate().map(|(i, b)| b ^ key[i & 3]).collect();
        let result = self.parse_client_body(&body);
        body.fill(0);
        result
    }

    fn parse_client_body(&mut self, body: &[u8]) -> Result<Packet> {
        let action = u16::from_be_bytes([body[1], body[2]]);
        if body[0] == 0 && action == 10016 {
            let mut reader = Bytes::new(&body[7..]);
            let _zone = reader.utf()?;
            let user_id = reader.utf()?.trim().parse::<u32>().map_err(|_| "登录字段格式异常")?;
            let _session = reader.utf()?;
            if reader.remaining() != 0 || user_id == 0 || user_id > i32::MAX as u32 {
                return Err("登录字段格式异常");
            }
            self.user_id = Some(user_id);
            self.inner_sequences.clear();
            // The SID is never copied into the session state or a Packet.
            return Ok(Packet::notice(Direction::Send, "logIn", "登录请求（凭据已隐藏）"));
        }
        if body[0] != 1 || action != 10033 {
            return Ok(Packet::notice(Direction::Send, &format!("system_{action}"), format!("系统发送 type={} action={action}", body[0])));
        }
        if !self.export_valid {
            return Ok(Packet::notice(Direction::Send, "unavailable", "本连接发送解码状态已失效，请重新连接后采集"));
        }
        let user_id = self.user_id.ok_or("缺少登录起点，不能解码发送消息")?;
        let mut reader = Bytes::new(&body[7..]);
        let extension_id = reader.u16()?;
        let command = reader.utf()?.to_owned();
        if command.trim() != command || command.is_empty() || command.chars().count() > 256 {
            return Err("发送命令格式异常");
        }
        let wrapper = AmfReader::decode(reader.rest())?;
        let Amf::Object(wrapper) = wrapper.as_ref() else { return Err("发送消息缺少 AMF3 包装对象") };
        let Some(data) = wrapper.get("data") else { return Err("发送消息缺少 data 字段") };
        let Amf::Bytes(data) = data.as_ref() else { return Err("发送消息 data 不是 ByteArray") };
        let expected = self.inner_sequences.get(&extension_id).map_or(18, |previous| {
            let next = ((previous & INNER_MASK) >> 2) * 2 + user_id % 108;
            if next < 123_216_728 { next } else { 204 }
        });
        let mut plain = data.clone();
        for block in (0..plain.len() / 4).rev() {
            let at = block * 4 + (block & 1);
            plain.swap(at, at + 2);
        }
        let keys = [(expected >> 22) as u8, (expected >> 18) as u8, (expected >> 9) as u8, (expected >> 2) as u8];
        for (index, byte) in plain.iter_mut().enumerate() {
            *byte ^= keys[index & 3];
        }
        let decoded = AmfReader::decode(&plain);
        plain.fill(0);
        let decoded = decoded?;
        let mut parameters = json_value(&decoded)?;
        let map = parameters.as_object_mut().ok_or("发送参数不是对象")?;
        let sequence = map.remove(":ext_seq;").and_then(sequence_i32).ok_or("发送参数缺少有效序号")? as u32;
        if (sequence & INNER_MASK) >> 2 != expected {
            return Err("发送序号不连续，本连接发送导出已停用");
        }
        self.inner_sequences.insert(extension_id, sequence);
        let redacted = redact_credentials(&mut parameters);
        let export = (!redacted).then(|| format!("#send={}|", json!({"id":extension_id,"param":parameters,"cmd":command})));
        Ok(Packet {
            direction: Direction::Send,
            summary: if redacted { format!("ext={extension_id} cmd={command}（敏感字段已隐藏，不可导出）") } else { format!("ext={extension_id} cmd={command}") },
            command,
            extension_id: Some(extension_id),
            parameters: Some(parameters),
            export,
        })
    }
}

fn sequence_i32(value: Value) -> Option<i32> {
    if let Some(value) = value.as_i64() { return i32::try_from(value).ok(); }
    // AMF3 carries int32 values outside its signed 29-bit range as doubles.
    let value = value.as_f64()?;
    (value.is_finite() && value.fract() == 0.0 && value >= i32::MIN as f64 && value <= i32::MAX as f64)
        .then_some(value as i32)
}

fn parse_server(frame: &[u8]) -> Result<Packet> {
    let mut reader = Bytes::new(&frame[4..]);
    match reader.byte()? {
        1 => {
            let decoded = AmfReader::decode(reader.rest())?;
            let mut parameters = json_value(&decoded)?;
            let command = match parameters.get("_cmd") {
                Some(Value::String(v)) => v.clone(),
                Some(Value::Number(v)) => v.to_string(),
                _ => String::new(),
            };
            if matches!(command.as_str(), "logOK" | "logKO" | "logIn") {
                return Ok(Packet::notice(Direction::Receive, &command, "登录响应（字段已隐藏）"));
            }
            redact_credentials(&mut parameters);
            Ok(Packet {
                direction: Direction::Receive,
                summary: if command.is_empty() { "EXT 服务端推送".into() } else { format!("接收 {command}") },
                command,
                extension_id: None,
                parameters: Some(parameters),
                export: None,
            })
        }
        0 => {
            let action = reader.u16()?;
            let room = i32::from_be_bytes(reader.take(4)?.try_into().unwrap());
            let command = match action {
                20015 => "createRmKO", 20016 => "dataObj", 20017 => "dmnMsg",
                20018 => "joinKO", 20019 => "joinOK", 20020 => "leaveRoom",
                20021 => "logKO", 20023 => "logout", 20026 => "pubMsg",
                20034 => "rVarsUpdate", 20036 => "uER", 20037 => "userGone",
                20039 => "uVarsUpdate", 20041 => "xtRes", 20042 => "crossDomain",
                20043 => "xtResB", _ => "system",
            };
            let mut packet = Packet::notice(Direction::Receive, command, format!("系统接收 action={action} room={room}（仅包头）"));
            packet.parameters = Some(json!({"action":action,"headerRoomId":room}));
            Ok(packet)
        }
        _ => Err("不支持的服务端消息类型"),
    }
}

fn redact_credentials(value: &mut Value) -> bool {
    let mut changed = false;
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.to_ascii_lowercase().as_str(), "sid" | "sessionid" | "session_id" | "game_auth_sid" | "password" | "passwd" | "authorization" | "cookie" | "cookies" | "access_token" | "refresh_token") {
                    *value = Value::String("[已隐藏]".into());
                    changed = true;
                } else {
                    changed |= redact_credentials(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values { changed |= redact_credentials(value); }
        }
        _ => {}
    }
    changed
}

struct Bytes<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Bytes<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data, position: 0 } }
    fn remaining(&self) -> usize { self.data.len() - self.position }
    fn rest(&self) -> &'a [u8] { &self.data[self.position..] }
    fn take(&mut self, size: usize) -> Result<&'a [u8]> {
        if size > self.remaining() { return Err("协议数据被截断") }
        let result = &self.data[self.position..self.position + size];
        self.position += size;
        Ok(result)
    }
    fn byte(&mut self) -> Result<u8> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16> { Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap())) }
    fn utf(&mut self) -> Result<&'a str> {
        let length = self.u16()? as usize;
        std::str::from_utf8(self.take(length)?).map_err(|_| "协议文本不是 UTF-8")
    }
    fn u29(&mut self) -> Result<u32> {
        let mut value = 0;
        for _ in 0..3 {
            let byte = self.byte()?;
            value = (value << 7) | (byte & 127) as u32;
            if byte < 128 { return Ok(value) }
        }
        Ok((value << 8) | self.byte()? as u32)
    }
}

#[derive(Debug)]
enum Amf {
    Json(Value),
    Array(Vec<Rc<Amf>>),
    Object(BTreeMap<String, Rc<Amf>>),
    Bytes(Vec<u8>),
}

#[derive(Clone)]
struct Traits {
    dynamic: bool,
    externalizable: bool,
    properties: Vec<String>,
}

struct AmfReader<'a> {
    bytes: Bytes<'a>,
    strings: Vec<String>,
    objects: Vec<Option<Rc<Amf>>>,
    traits: Vec<Traits>,
    items: usize,
    allocation_budget: usize,
}

impl<'a> AmfReader<'a> {
    fn decode(data: &'a [u8]) -> Result<Rc<Amf>> {
        let mut reader = Self { bytes: Bytes::new(data), strings: Vec::new(), objects: Vec::new(), traits: Vec::new(), items: 0, allocation_budget: MAX_BUFFER };
        let value = reader.read(0)?;
        if reader.bytes.remaining() != 0 { return Err("AMF3 值后存在额外数据") }
        Ok(value)
    }
    fn count(&mut self) -> Result<()> {
        self.items += 1;
        if self.items > MAX_ITEMS { return Err("AMF3 项数超过上限") }
        Ok(())
    }
    fn string(&mut self) -> Result<String> {
        self.count()?;
        let header = self.bytes.u29()?;
        if header & 1 == 0 {
            let value = self.strings.get((header >> 1) as usize).ok_or("AMF3 字符串引用无效")?;
            spend(&mut self.allocation_budget, value.len())?;
            return Ok(value.clone());
        }
        let bytes = self.bytes.take((header >> 1) as usize)?;
        spend(&mut self.allocation_budget, bytes.len().saturating_mul(2))?;
        let value = std::str::from_utf8(bytes).map_err(|_| "AMF3 文本不是 UTF-8")?.to_owned();
        if !value.is_empty() { self.strings.push(value.clone()); }
        Ok(value)
    }
    fn read(&mut self, depth: usize) -> Result<Rc<Amf>> {
        self.count()?;
        if depth > 64 { return Err("AMF3 嵌套超过上限") }
        let kind = self.bytes.byte()?;
        let scalar = match kind {
            0 | 1 => Some(Value::Null),
            2 => Some(Value::Bool(false)),
            3 => Some(Value::Bool(true)),
            4 => {
                let value = self.bytes.u29()?;
                let signed = if value >= 0x1000_0000 { value as i64 - 0x2000_0000 } else { value as i64 };
                Some(signed.into())
            }
            5 => {
                let value = f64::from_be_bytes(self.bytes.take(8)?.try_into().unwrap());
                Some(serde_json::Number::from_f64(value).ok_or("AMF3 非有限数字不能导出 JSON")?.into())
            }
            6 => Some(self.string()?.into()),
            9 | 10 | 12 => None,
            _ => return Err("不支持的 AMF3 类型"),
        };
        if let Some(value) = scalar { return Ok(Rc::new(Amf::Json(value))) }
        let header = self.bytes.u29()?;
        if header & 1 == 0 {
            let value = self.objects.get((header >> 1) as usize).and_then(|v| v.as_ref()).cloned().ok_or("AMF3 对象引用无效或形成循环")?;
            if kind == 12 && !matches!(value.as_ref(), Amf::Bytes(_)) { return Err("AMF3 ByteArray 引用类型错误") }
            return Ok(value);
        }
        let slot = self.objects.len();
        self.objects.push(None);
        let value = match kind {
            12 => Amf::Bytes(self.bytes.take((header >> 1) as usize)?.to_vec()),
            9 => {
                let count = (header >> 1) as usize;
                if count > MAX_ITEMS || count > self.bytes.remaining() { return Err("AMF3 数组长度异常") }
                let mut key = self.string()?;
                if key.is_empty() {
                    let mut values = Vec::with_capacity(count);
                    for _ in 0..count { values.push(self.read(depth + 1)?); }
                    Amf::Array(values)
                } else {
                    let mut values = BTreeMap::new();
                    while !key.is_empty() {
                        let value = self.read(depth + 1)?;
                        if values.insert(key, value).is_some() { return Err("AMF3 关联数组属性重复") }
                        key = self.string()?;
                    }
                    for index in 0..count {
                        if values.insert(index.to_string(), self.read(depth + 1)?).is_some() { return Err("AMF3 关联数组下标冲突") }
                    }
                    Amf::Object(values)
                }
            }
            10 => {
                let traits = if header & 3 == 1 {
                    let traits = self.traits.get((header >> 2) as usize).ok_or("AMF3 traits 引用无效")?;
                    spend(&mut self.allocation_budget, traits.properties.iter().map(|s| s.len() + 24).sum())?;
                    traits.clone()
                } else {
                    let count = (header >> 4) as usize;
                    if count > MAX_ITEMS || count > self.bytes.remaining() { return Err("AMF3 属性数异常") }
                    let _class = self.string()?;
                    let mut properties = Vec::with_capacity(count);
                    for _ in 0..count { properties.push(self.string()?); }
                    let traits = Traits { dynamic: header & 8 != 0, externalizable: header & 4 != 0, properties };
                    spend(&mut self.allocation_budget, traits.properties.iter().map(|s| s.len() + 24).sum())?;
                    self.traits.push(traits.clone());
                    traits
                };
                if traits.externalizable {
                    let value = self.read(depth + 1)?;
                    self.objects[slot] = Some(value.clone());
                    return Ok(value);
                }
                let mut values = BTreeMap::new();
                for key in traits.properties {
                    if values.insert(key, self.read(depth + 1)?).is_some() { return Err("AMF3 对象属性重复") }
                }
                if traits.dynamic {
                    loop {
                        let key = self.string()?;
                        if key.is_empty() { break; }
                        if values.insert(key, self.read(depth + 1)?).is_some() { return Err("AMF3 对象属性重复") }
                    }
                }
                Amf::Object(values)
            }
            _ => unreachable!(),
        };
        let value = Rc::new(value);
        self.objects[slot] = Some(value.clone());
        Ok(value)
    }
}

fn spend(budget: &mut usize, count: usize) -> Result<()> {
    *budget = budget.checked_sub(count).ok_or("AMF3 展开后数据超过上限")?;
    Ok(())
}

fn json_value(value: &Amf) -> Result<Value> {
    let mut budget = MAX_BUFFER;
    to_json(value, &mut budget, 0)
}

fn to_json(value: &Amf, budget: &mut usize, depth: usize) -> Result<Value> {
    if depth > 64 { return Err("AMF3 引用展开超过嵌套上限") }
    spend(budget, 32)?;
    match value {
        Amf::Json(value) => {
            if let Value::String(text) = value { spend(budget, text.len().saturating_mul(6))?; }
            Ok(value.clone())
        }
        Amf::Array(values) => values.iter().map(|v| to_json(v, budget, depth + 1)).collect::<Result<Vec<_>>>().map(Value::Array),
        Amf::Object(values) => {
            let mut map = Map::new();
            for (key, value) in values {
                spend(budget, key.len().saturating_mul(6))?;
                map.insert(key.clone(), to_json(value, budget, depth + 1)?);
            }
            Ok(Value::Object(map))
        }
        Amf::Bytes(bytes) => {
            // Match Newtonsoft's byte[] representation for display/export.
            spend(budget, bytes.len().div_ceil(3).saturating_mul(4))?;
            const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
            for chunk in bytes.chunks(3) {
                let b0 = chunk[0] as usize;
                let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
                let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
                text.push(ALPHABET[b0 >> 2] as char);
                text.push(ALPHABET[((b0 & 3) << 4) | (b1 >> 4)] as char);
                text.push(if chunk.len() > 1 { ALPHABET[((b1 & 15) << 2) | (b2 >> 6)] as char } else { '=' });
                text.push(if chunk.len() > 2 { ALPHABET[b2 & 63] as char } else { '=' });
            }
            Ok(text.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut frame = (body.len() as u32).to_be_bytes().to_vec();
        frame.extend(body);
        frame
    }
    fn client(body: &[u8]) -> Vec<u8> {
        let key = 79u32.to_be_bytes();
        let mut wire = key.to_vec();
        wire.extend(body.iter().enumerate().map(|(i, b)| b ^ key[i & 3]));
        frame(&wire)
    }
    fn utf(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend((value.len() as u16).to_be_bytes());
        bytes.extend(value.as_bytes());
    }
    fn login() -> Vec<u8> {
        let mut body = vec![0, 0x27, 0x20, 255, 255, 255, 255]; // action 10016
        utf(&mut body, "synthetic-zone");
        utf(&mut body, "12345");
        utf(&mut body, "synthetic-secret-never-export");
        client(&body)
    }
    fn u29(bytes: &mut Vec<u8>, value: u32) {
        if value < 0x80 { bytes.push(value as u8); }
        else if value < 0x4000 { bytes.extend([(value >> 7) as u8 | 0x80, (value & 127) as u8]); }
        else if value < 0x20_0000 { bytes.extend([(value >> 14) as u8 | 0x80, ((value >> 7) & 127) as u8 | 0x80, (value & 127) as u8]); }
        else { bytes.extend([((value >> 22) & 127) as u8 | 0x80, ((value >> 15) & 127) as u8 | 0x80, ((value >> 8) & 127) as u8 | 0x80, value as u8]); }
    }
    fn amf_string(bytes: &mut Vec<u8>, text: &str) {
        u29(bytes, (text.len() as u32) * 2 + 1);
        bytes.extend(text.as_bytes());
    }
    fn extension(base: u32, extension_id: u16) -> Vec<u8> {
        extension_with_sequence(base, extension_id, None)
    }
    fn extension_with_sequence(base: u32, extension_id: u16, wide_sequence: Option<i32>) -> Vec<u8> {
        let mut plain = vec![10, 11, 1]; // anonymous dynamic object
        amf_string(&mut plain, ":ext_seq;");
        if let Some(sequence) = wide_sequence {
            plain.push(5);
            plain.extend((sequence as f64).to_be_bytes());
        } else {
            plain.push(4);
            u29(&mut plain, base << 2);
        }
        amf_string(&mut plain, "text");
        plain.push(6);
        amf_string(&mut plain, "quoted | value");
        plain.push(1);
        let keys = [(base >> 22) as u8, (base >> 18) as u8, (base >> 9) as u8, (base >> 2) as u8];
        for (i, byte) in plain.iter_mut().enumerate() { *byte ^= keys[i & 3]; }
        for block in 0..plain.len() / 4 { let at = block * 4 + (block & 1); plain.swap(at, at + 2); }
        let mut body = vec![1, 0x27, 0x31, 255, 255, 255, 255]; // action 10033
        body.extend(extension_id.to_be_bytes());
        utf(&mut body, "fixture");
        body.extend([10, 11, 1]);
        amf_string(&mut body, "data");
        body.push(12);
        u29(&mut body, (plain.len() as u32) * 2 + 1);
        body.extend(plain);
        body.push(1);
        client(&body)
    }

    #[test]
    fn fragmented_login_and_coalesced_extensions_preserve_sequence_and_privacy() {
        let mut decoder = Decoder::default();
        let login = login();
        for b in &login[..login.len() - 1] { assert!(decoder.feed(Direction::Send, &[*b]).is_empty()); }
        let mut last = vec![*login.last().unwrap()];
        last.extend(extension(18, 42));
        last.extend(extension(36 + 12345 % 108, 42));
        last.extend(extension(18, 13));
        let packets = decoder.feed(Direction::Send, &last);
        assert_eq!(packets.len(), 4);
        assert_eq!(packets[0].command, "logIn");
        assert!(packets[0].parameters.is_none());
        assert!(!format!("{packets:?}").contains("synthetic-secret"));
        for packet in &packets[1..] {
            let export = packet.export.as_ref().expect("valid EXT export");
            assert!(!export.contains(":ext_seq;"));
            assert_eq!(packet.parameters.as_ref().unwrap()["text"], "quoted | value");
        }
        assert!(decoder.send.is_empty());
    }

    #[test]
    fn sequence_gap_disables_export_until_new_decoder() {
        let mut decoder = Decoder::default();
        decoder.feed(Direction::Send, &login());
        assert!(decoder.feed(Direction::Send, &extension(19, 42))[0].export.is_none());
        assert!(decoder.feed(Direction::Send, &extension(18, 42))[0].export.is_none());
        decoder.feed(Direction::Send, &login());
        assert!(decoder.feed(Direction::Send, &extension(18, 42))[0].export.is_none());
        decoder.invalidate();
        assert!(decoder.feed(Direction::Receive, &frame(&[0, 0x4e, 0x37, 255, 255, 255, 255])).is_empty());
    }

    #[test]
    fn randomized_int32_sequence_uses_amf_double_without_losing_continuity() {
        let mut decoder = Decoder::default();
        decoder.feed(Direction::Send, &login());
        let sequence = 0xc000_0049u32 as i32;
        let packets = decoder.feed(Direction::Send, &extension_with_sequence(18, 42, Some(sequence)));
        assert!(packets[0].export.is_some(), "{packets:?}");
        let next = decoder.feed(Direction::Send, &extension(36 + 12345 % 108, 42));
        assert!(next[0].export.is_some());
        assert!(sequence_i32(json!(72.5)).is_none());
        assert!(sequence_i32(json!(2147483648.0)).is_none());
    }

    #[test]
    fn policy_and_server_json_can_share_a_chunk() {
        let mut decoder = Decoder::default();
        assert!(decoder.feed(Direction::Receive, b"<cross-domain-").is_empty());
        let mut chunk = b"policy/>\0".to_vec();
        let mut body = vec![1, 10, 11, 1];
        amf_string(&mut body, "_cmd");
        body.push(6);
        amf_string(&mut body, "logOK");
        body.push(1);
        chunk.extend(frame(&body));
        let packets = decoder.feed(Direction::Receive, &chunk);
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].command, "policy");
        assert_eq!(packets[1].command, "logOK");
        assert!(packets[1].parameters.is_none());
        assert!(packets.iter().all(|p| p.export.is_none()));
    }

    #[test]
    fn invalid_length_stops_framing_and_never_echoes_input() {
        let mut decoder = Decoder::default();
        let packets = decoder.feed(Direction::Send, b"\xff\xff\xff\xffsecret");
        assert_eq!(packets[0].command, "decode-error");
        assert!(!format!("{packets:?}").contains("secret"));
        assert!(decoder.feed(Direction::Send, &login()).is_empty());
    }

    #[test]
    fn amf_references_signed_integer_and_byte_array_match_json_contract() {
        // Array: string, same string by reference, -1, bytes, same bytes by reference.
        let mut bytes = vec![9, 11, 1, 6];
        amf_string(&mut bytes, "hello");
        bytes.extend([6, 0, 4, 255, 255, 255, 255, 12, 7, 1, 2, 3, 12, 2]);
        let value = AmfReader::decode(&bytes).unwrap();
        assert_eq!(json_value(&value).unwrap(), json!(["hello","hello",-1,"AQID","AQID"]));
        assert!(AmfReader::decode(&[9, 3, 1, 9, 0]).is_err()); // self reference
        assert!(AmfReader::decode(&[6, 0]).is_err()); // no prior string
    }

    #[test]
    fn amf_trait_and_object_references_preserve_values() {
        // Two sealed objects share traits; final item references the first object.
        let mut bytes = vec![9, 7, 1, 10, 19, 1];
        amf_string(&mut bytes, "x");
        bytes.extend([4, 5, 10, 1, 4, 7, 10, 2]);
        let value = AmfReader::decode(&bytes).unwrap();
        assert_eq!(json_value(&value).unwrap(), json!([{"x":5},{"x":7},{"x":5}]));
    }

    #[test]
    fn credentials_are_redacted_in_nested_business_values() {
        let mut value = json!({"items":[{"SID":"secret","x":4}],"SessionId":"secret2","command":"fixture"});
        assert!(redact_credentials(&mut value));
        assert!(!value.to_string().contains("secret"));
        assert_eq!(value["items"][0]["x"], 4);
    }
}
