//! Reconstruction of src/swf_adapter.rs, native 0x1404f6ad0.
//! Named DoABC tags replace their matching tag. Unnamed shadow tags are
//! inserted before the first original DoABC, so their definitions win while
//! untouched classes and graphics remain available in the original movie.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::ops::Range;

use flate2::read::ZlibDecoder;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MAX_SWF: usize = 64 * 1024 * 1024;

struct Tag {
    code: u16,
    whole: Range<usize>,
    payload: Range<usize>,
    name: String,
}

fn unpack(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.len() < 9 { return Err("truncated SWF header".into()); }
    let size = u32::from_le_bytes(input[4..8].try_into().unwrap()) as usize;
    if !(9..=MAX_SWF).contains(&size) { return Err("invalid SWF size".into()); }
    let mut result = input[..8].to_vec();
    result[..3].copy_from_slice(b"FWS");
    match &input[..3] {
        b"FWS" => {
            if input.len() != size { return Err("SWF length does not match header".into()); }
            result.extend_from_slice(&input[8..]);
        }
        b"CWS" => {
            ZlibDecoder::new(&input[8..]).take(MAX_SWF as u64).read_to_end(&mut result).map_err(|e| e.to_string())?;
        }
        b"ZWS" => {
            // The original adapter also delegates ZWS decoding to Ruffle's SWF reader.
            let decoded = swf::decompress_swf(input).map_err(|e| e.to_string())?;
            let mut header = decoded.header.swf_header().clone();
            header.compression = swf::Compression::None;
            result.clear();
            swf::write::write_swf_raw_tags(&header, &decoded.data, &mut result).map_err(|e| e.to_string())?;
            // Re-encoding RECT may change its bit width, as it does in the original adapter.
            if result.len() > MAX_SWF { return Err("invalid SWF size".into()); }
            return Ok(result);
        }
        _ => return Err("unsupported SWF compression".into()),
    }
    if result.len() != size { return Err("SWF length does not match header".into()); }
    Ok(result)
}

fn tags(bytes: &[u8]) -> Result<(usize, Vec<Tag>), String> {
    let bits = bytes.get(8).ok_or("truncated SWF header")? >> 3;
    let first = 8 + (5 + usize::from(bits) * 4).div_ceil(8) + 4;
    if first > bytes.len() { return Err("truncated SWF header".into()); }
    let mut at = first;
    let mut tags = Vec::new();
    while at < bytes.len() {
        let start = at;
        let short = bytes.get(at..at + 2).ok_or("truncated tag header")?;
        let header = u16::from_le_bytes(short.try_into().unwrap());
        let code = header >> 6;
        let mut length = (header & 63) as usize;
        at += 2;
        if length == 63 {
            length = u32::from_le_bytes(bytes.get(at..at + 4).ok_or("truncated tag length")?.try_into().unwrap()) as usize;
            at += 4;
        }
        let end = at.checked_add(length).filter(|end| *end <= bytes.len()).ok_or("truncated tag")?;
        let name = if code == 82 {
            let body = bytes.get(at + 4..end).ok_or("invalid DoABC")?;
            let null = body.iter().position(|b| *b == 0).ok_or("unterminated ABC name")?;
            String::from_utf8_lossy(&body[..null]).into_owned()
        } else { String::new() };
        tags.push(Tag { code, whole: start..end, payload: at..end, name });
        at = end;
    }
    Ok((first, tags))
}

pub(super) fn splice(source: &[u8], shadow: &[u8]) -> Result<(Vec<u8>, Value), String> {
    let source = unpack(source)?;
    let shadow = unpack(shadow)?;
    let (first, original_tags) = tags(&source)?;
    let (_, shadow_tags) = tags(&shadow)?;
    let mut named = BTreeMap::new();
    let mut anonymous = Vec::new();
    let mut pins = Value::Null;
    for tag in &shadow_tags {
        match tag.code {
            82 if tag.name.is_empty() => anonymous.push(tag),
            82 => { named.insert(tag.name.as_str(), tag); }
            77 => {
                let text = shadow[tag.payload.clone()].split(|b| *b == 0).next().unwrap_or_default();
                let metadata: Value = serde_json::from_slice(text).map_err(|e| e.to_string())?;
                pins = metadata.get("pins").cloned().unwrap_or(Value::Null);
            }
            _ => {}
        }
    }
    let mut output = source[..first].to_vec();
    let mut changed = Vec::new();
    let mut seen = BTreeSet::new();
    let mut applied = 0;
    let mut inserted = false;
    for tag in &original_tags {
        if tag.code == 82 && !inserted {
            for addition in &anonymous {
                output.extend_from_slice(&shadow[addition.whole.clone()]);
                applied += 1;
            }
            if !anonymous.is_empty() {
                if let Some(pin) = pins.get("").and_then(Value::as_str) {
                    if pin != format!("{:x}", Sha256::digest(&source[tag.payload.clone()])) { changed.push("(module code)".to_string()); }
                }
            }
            inserted = true;
        }
        if tag.code == 82 && !tag.name.is_empty() && let Some(replacement) = named.get(tag.name.as_str()) {
            seen.insert(tag.name.as_str());
            let digest = format!("{:x}", Sha256::digest(&source[tag.payload.clone()]));
            if pins.get(&tag.name).and_then(Value::as_str) != Some(digest.as_str()) { changed.push(tag.name.clone()); }
            output.extend_from_slice(&shadow[replacement.whole.clone()]);
            applied += 1;
        } else {
            output.extend_from_slice(&source[tag.whole.clone()]);
        }
    }
    if output.len() > MAX_SWF { return Err("adapted SWF too large".into()); }
    let length = output.len() as u32;
    output[4..8].copy_from_slice(&length.to_le_bytes());
    let missing: Vec<_> = named.keys().copied().filter(|name| !seen.contains(name)).collect();
    Ok((output, json!({"applied":applied,"changed":changed,"missing":missing})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(code: u16, payload: &[u8]) -> Vec<u8> {
        let mut output = ((code << 6) | 63).to_le_bytes().to_vec();
        output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        output.extend_from_slice(payload);
        output
    }
    fn abc(name: &str, body: &[u8]) -> Vec<u8> {
        let mut data = vec![0,0,0,0];
        data.extend_from_slice(name.as_bytes()); data.push(0); data.extend_from_slice(body);
        tag(82, &data)
    }
    fn movie(tags: &[Vec<u8>]) -> Vec<u8> {
        let mut data = b"FWS\x0a\0\0\0\0\x08\0\0\x18\x01\0".to_vec();
        for tag in tags { data.extend_from_slice(tag); }
        data.extend_from_slice(&[0,0]);
        let size = data.len() as u32; data[4..8].copy_from_slice(&size.to_le_bytes()); data
    }

    #[test]
    fn named_replacement_preserves_unrelated_tags_and_reports_missing() {
        let original = movie(&[tag(9, &[1,2,3]), abc("a", b"old"), abc("b", b"keep")]);
        let patch = movie(&[abc("a", b"new"), abc("absent", b"unused")]);
        let (result, report) = splice(&original, &patch).unwrap();
        assert_eq!(result, movie(&[tag(9, &[1,2,3]), abc("a", b"new"), abc("b", b"keep")]));
        assert_eq!(report, json!({"applied":1,"changed":["a"],"missing":["absent"]}));
    }

    #[test]
    fn anonymous_shadow_precedes_first_abc_and_pin_checks_original_payload() {
        let original_abc = abc("", b"original");
        let digest = format!("{:x}", Sha256::digest(&original_abc[6..]));
        let metadata = json!({"shadow":1,"pins":{"":digest}}).to_string();
        let patch = movie(&[tag(77,metadata.as_bytes()), abc("", b"overlay")]);
        let original = movie(&[tag(9,&[1,2,3]),original_abc.clone()]);
        let (result, report) = splice(&original,&patch).unwrap();
        assert_eq!(result,movie(&[tag(9,&[1,2,3]),abc("",b"overlay"),original_abc]));
        assert_eq!(report,json!({"applied":1,"changed":[],"missing":[]}));
    }

    #[test]
    fn compressed_swf_and_malformed_input() {
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;
        let original = movie(&[abc("a", b"old")]);
        let mut compressed = original[..8].to_vec(); compressed[..3].copy_from_slice(b"CWS");
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&original[8..]).unwrap(); compressed.extend(encoder.finish().unwrap());
        assert_eq!(splice(&compressed,&movie(&[abc("a",b"new")])).unwrap().0,movie(&[abc("a",b"new")]));
        assert!(splice(b"FWS",&original).is_err());
        let mut bad = original.clone(); bad.pop(); assert!(splice(&bad,&original).is_err());
        let mut bad_name = movie(&[tag(82,b"\0\0\0\0nonterminated")]);
        assert!(splice(&bad_name,&original).is_err());
        bad_name[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(splice(&bad_name,&original).is_err());
    }

    #[test]
    fn lzma_swf() {
        let original = movie(&[abc("a", b"old")]);
        let decoded = swf::decompress_swf(&original[..]).unwrap();
        let mut header = decoded.header.swf_header().clone();
        header.version = 13;
        header.compression = swf::Compression::Lzma;
        let mut compressed = Vec::new();
        swf::write::write_swf_raw_tags(&header, &decoded.data, &mut compressed).unwrap();
        let (result, report) = splice(&compressed, &movie(&[abc("a", b"new")])).unwrap();
        assert_eq!(report["applied"], 1);
        assert_eq!(&result[..3], b"FWS");
        assert_eq!(result[3], 13);
        let (_, result_tags) = tags(&result).unwrap();
        let replacement = result_tags.iter().find(|tag| tag.code == 82).unwrap();
        assert!(result[replacement.payload.clone()].ends_with(b"new"));
    }
}
