//! Parse a complete workshop script for the local rehearsal page.
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    id: u16,
    cmd: String,
    param: Value,
}

fn step(text: &str) -> Result<Value, String> {
    if text == "#wait" {
        return Ok(json!({"kind":"wait"}));
    }
    if let Some(value) = text
        .strip_prefix("#time")
        .and_then(|rest| rest.trim_start().strip_prefix('='))
    {
        let value = value.trim();
        let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
        if digits.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
            if let Ok(seconds) = value.parse::<f64>() {
                if seconds.is_finite() && seconds >= 0.0 {
                    return Ok(json!({"kind":"delay","seconds":seconds}));
                }
            }
        }
        return Err("#time 需要有限、非负的十进制秒数".into());
    }
    let raw = text.strip_prefix("#send=").ok_or("无法识别的指令")?;
    let message: Message = serde_json::from_str(raw)
        .map_err(|e| format!("#send 需要 id、cmd 和 param（{e}）"))?;
    let param = message.param.as_object();
    if param.is_none() && !message.param.is_null() {
        return Err("param 需要对象或 null".into());
    }
    if message.cmd.is_empty()
        || message.cmd.trim() != message.cmd
        || message.cmd.encode_utf16().count() > 256
    {
        return Err("cmd 需要 1～256 个字符，首尾不能有空白".into());
    }
    let mut row = json!({"kind":"message","id":message.id,"cmd":message.cmd,
        "fields":param.map_or(0, |p| p.keys().filter(|k| k.as_str() != ":ext_seq;").count())});
    if message.id == 13 && message.cmd == "1212" && param.is_some_and(|p| p.is_empty()) {
        row["action"] = json!("readOwnBag");
    }
    Ok(row)
}

pub(crate) fn validate(payload: &Value) -> Result<Value, String> {
    let script = payload["text"]
        .as_str()
        .ok_or("InvalidArguments: 缺少脚本文本")?;
    let mut steps = Vec::new();
    let mut segment = String::new();
    let (mut quoted, mut escaped) = (false, false);
    let mut start = 1;
    let mut finish = |text: &str, line: usize| -> Result<(), String> {
        if text.trim().is_empty() {
            return Ok(());
        }
        if steps.len() >= 1024 {
            return Err("InvalidArguments: 单次演练最多 1024 步".into());
        }
        let mut result =
            step(text.trim()).map_err(|e| format!("InvalidArguments: 第 {line} 行，{e}"))?;
        result["line"] = json!(line);
        steps.push(result);
        Ok(())
    };
    for (index, line) in script.trim_start_matches('\u{feff}').lines().enumerate() {
        if line.trim_start().starts_with("//") && !quoted {
            continue;
        }
        for ch in line.chars().chain(std::iter::once('\n')) {
            if ch == '|' && !quoted {
                finish(&segment, start)?;
                segment.clear();
                continue;
            }
            if segment.is_empty() {
                if ch.is_whitespace() {
                    continue;
                }
                start = index + 1;
            }
            segment.push(ch);
            if escaped {
                escaped = false;
            } else if quoted && ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = !quoted;
            }
        }
    }
    finish(&segment, start)?;
    if steps.is_empty() {
        return Err("InvalidArguments: 请输入至少一条指令".into());
    }
    let own_bag = steps.len() == 1 && steps[0]["action"] == "readOwnBag";
    Ok(json!({"steps":steps,"canReadOwnBag":own_bag}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_null_param_rehearses_without_granting_the_native_action() {
        let result = validate(&json!({"text": "#send={\"id\":13,\"cmd\":\"1212\",\"param\":null}|"})).unwrap();
        assert_eq!(result["steps"][0]["fields"], 0);
        assert_eq!(result["canReadOwnBag"], false);
    }

    #[test]
    fn only_a_single_targetless_own_bag_read_offers_the_native_action() {
        let single = r#"#send={"id":13,"cmd":"1212","param":{}}|"#;
        assert_eq!(
            validate(&json!({"text":single})).unwrap()["canReadOwnBag"],
            true
        );
        for text in [
            single.replace("1212", "other"),
            single.replace("{}", r#"{"owner":7}"#),
            format!("{single} #wait"),
            format!("{single}{single}"),
        ] {
            assert_eq!(
                validate(&json!({"text":text})).unwrap()["canReadOwnBag"],
                false
            );
        }
    }

    #[test]
    fn rehearsal_validates_all_steps_and_preserves_source_lines() {
        let text = "\u{feff}// note\r\n#send={\"cmd\":\"demo\",\"id\":3,\"param\":{\"x\":\"a|\\\"b\",\"y\":7}} |\n #time = +0.5 | #wait";
        let steps = validate(&json!({"text":text})).unwrap();
        assert_eq!(
            steps["steps"],
            json!([
                {"line":2,"kind":"message","id":3,"cmd":"demo","fields":2},
                {"line":3,"kind":"delay","seconds":0.5}, {"line":3,"kind":"wait"}
            ])
        );
        for invalid in [
            "// only",
            "#wait |\n #time = -3",
            "#time=1e5",
            r#"#send={"id":1,"cmd":"x","param":[]}"#,
            r#"#send={"id":1,"id":2,"cmd":"x","param":{}}"#,
        ] {
            assert!(validate(&json!({"text":invalid})).is_err(), "{invalid}");
        }
    }
}
