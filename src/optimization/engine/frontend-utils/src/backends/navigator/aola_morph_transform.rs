use flate2::{Decompress, FlushDecompress, Status};
use lzma_rs::decompress::{Options, UnpackedSize};
use ruffle_core::swf::avm2::{read::Reader as AbcReader, write::Writer as AbcWriter};
use ruffle_core::swf::extensions::ReadSwfExt;
use ruffle_core::swf::read::Reader;
use ruffle_core::swf::{SymbolClassLink, Tag, TagCode};
use std::error::Error;
use std::io::{self, Cursor};
use std::ops::Range;
use tokio::sync::Semaphore;

static TRANSFORM_SEMAPHORE: Semaphore = Semaphore::const_new(2);
const MAX_MOVIE_SIZE: usize = 128 * 1024 * 1024;
const MAX_LZMA_DICTIONARY: usize = 64 * 1024 * 1024;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MorphKind {
    Backpack { index: u8 },
    Battle { face: u8, part: u8 },
    Poster,
}

pub(super) async fn transform(
    target_bytes: Vec<u8>,
    source_id: u32,
    target_id: u32,
    kind: MorphKind,
) -> Result<Vec<u8>> {
    let permit = TRANSFORM_SEMAPHORE.acquire().await?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        transform_movie(target_bytes, source_id, target_id, kind)
    })
    .await?
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn unpack_movie(bytes: Vec<u8>) -> Result<Vec<u8>> {
    let (header, body) = bytes
        .split_at_checked(8)
        .ok_or_else(|| invalid("truncated SWF header"))?;
    let size = u32::from_le_bytes(header[4..8].try_into()?) as usize;
    if header[3] == 0 || !(8..=MAX_MOVIE_SIZE).contains(&size) || bytes.len() > MAX_MOVIE_SIZE {
        return Err(invalid("invalid SWF version or length").into());
    }

    match &header[..3] {
        b"FWS" if bytes.len() == size => Ok(bytes),
        b"CWS" if header[3] >= 6 => {
            let mut movie = Vec::with_capacity(size + 1);
            movie.extend_from_slice(header);
            let mut decoder = Decompress::new(true);
            let status = decoder.decompress_vec(body, &mut movie, FlushDecompress::Finish)?;
            if status != Status::StreamEnd
                || movie.len() != size
                || decoder.total_in() != body.len() as u64
            {
                return Err(invalid("invalid CWS stream length").into());
            }
            movie[0] = b'F';
            Ok(movie)
        }
        b"ZWS" if header[3] >= 13 && body.len() >= 9 => {
            let packed_size = u32::from_le_bytes(body[..4].try_into()?) as usize;
            if packed_size != body.len() - 9 {
                return Err(invalid("invalid ZWS compressed length").into());
            }
            let mut movie = vec![0; size];
            movie[..8].copy_from_slice(header);
            movie[0] = b'F';
            let mut decoded = Cursor::new(&mut movie[8..]);
            lzma_rs::lzma_decompress_with_options(
                &mut &body[4..],
                &mut decoded,
                &Options {
                    unpacked_size: UnpackedSize::UseProvided(None),
                    memlimit: Some(MAX_LZMA_DICTIONARY),
                    allow_incomplete: false,
                },
            )?;
            if decoded.position() != (size - 8) as u64 {
                return Err(invalid("invalid ZWS decompressed length").into());
            }
            Ok(movie)
        }
        _ => Err(invalid("invalid SWF compression or length").into()),
    }
}

struct ClassNames {
    mappings: Vec<(String, String)>,
    config: Option<String>,
}

impl ClassNames {
    fn new(source: u32, target: u32, kind: MorphKind) -> Self {
        let (mut mappings, config) = match kind {
            MorphKind::Backpack { index } => (
                vec![
                    (format!("PetMovie{target}_"), format!("PetMovie{source}_")),
                    (format!("petmovie{target}_"), format!("petmovie{source}_")),
                ],
                (index == 0).then(|| {
                    (format!("PetMovie{target}Config"), format!("PetMovie{source}Config"))
                }),
            ),
            MorphKind::Battle { face, part } => (
                vec![(format!("pet{target}_{face}_"), format!("pet{source}_{face}_"))],
                (part == 0).then(|| {
                    (format!("Pet{target}Config_{face}"), format!("Pet{source}Config_{face}"))
                }),
            ),
            MorphKind::Poster => (
                vec![
                    (format!("Background{target}"), format!("Background{source}")),
                    (
                        format!("petskinbackground{target}"),
                        format!("petskinbackground{source}"),
                    ),
                ],
                None,
            ),
        };
        let config = config.map(|mapping| {
            let name = mapping.0.clone();
            mappings.push(mapping);
            name
        });
        Self { mappings, config }
    }

    fn replace(&self, name: &[u8], matches: &mut ClassMatches) -> Option<Vec<u8>> {
        // ABC pools may contain a short name, a QName or a debug name separated by `:`.
        let start = name
            .iter()
            .rposition(|byte| matches!(byte, b'.' | b':' | b'/'))
            .map_or(0, |index| index + 1);
        let short_name = &name[start..];
        for (old, new) in &self.mappings {
            let Some(suffix) = short_name.strip_prefix(old.as_bytes()) else {
                continue;
            };
            if !old.ends_with('_') && !suffix.is_empty() && !suffix.starts_with(b"_") {
                continue;
            }
            if self.config.as_ref() == Some(old) {
                matches.config |= suffix.is_empty();
            } else {
                matches.asset = true;
            }
            let mut out = Vec::with_capacity(start + new.len() + suffix.len());
            out.extend_from_slice(&name[..start]);
            out.extend_from_slice(new.as_bytes());
            out.extend_from_slice(suffix);
            return Some(out);
        }
        None
    }
}

#[derive(Default)]
struct ClassMatches {
    asset: bool,
    config: bool,
}

fn rewrite_symbols(
    links: Vec<SymbolClassLink<'_>>,
    length: usize,
    names: &ClassNames,
    matches: &mut ClassMatches,
) -> Result<Option<Vec<u8>>> {
    if length != 2 + links.iter().map(|link| link.class_name.len() + 3).sum::<usize>() {
        return Err(invalid("invalid SymbolClass tag length").into());
    }
    let mut data = Vec::with_capacity(length);
    data.extend_from_slice(&(links.len() as u16).to_le_bytes());
    let mut changed = false;
    for link in links {
        data.extend_from_slice(&link.id.to_le_bytes());
        let name = link.class_name.as_bytes();
        if let Some(replacement) = names.replace(name, matches) {
            data.extend_from_slice(&replacement);
            changed = true;
        } else {
            data.extend_from_slice(name);
        }
        data.push(0);
    }
    Ok(changed.then_some(data))
}

fn rewrite_abc(
    prefix: &[u8],
    abc_bytes: &[u8],
    names: &ClassNames,
    matches: &mut ClassMatches,
) -> Result<Option<Vec<u8>>> {
    let mut reader = AbcReader::new(abc_bytes);
    let mut abc = reader.read()?;
    if !reader.as_slice().is_empty() {
        return Err(invalid("trailing data in DoABC tag").into());
    }
    let mut changed = false;
    for string in &mut abc.constant_pool.strings {
        if let Some(replacement) = names.replace(string, matches) {
            *string = replacement;
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    let mut data = Vec::with_capacity(prefix.len() + abc_bytes.len());
    data.extend_from_slice(prefix);
    AbcWriter::new(&mut data).write(abc)?;
    Ok(Some(data))
}

fn encode_tag(code: u16, data: &[u8]) -> Vec<u8> {
    let mut tag = Vec::with_capacity(data.len() + 6);
    let length = data.len() as u32;
    tag.extend_from_slice(&((code << 6) | length.min(63) as u16).to_le_bytes());
    if length >= 63 {
        tag.extend_from_slice(&length.to_le_bytes());
    }
    tag.extend_from_slice(data);
    tag
}

fn collect_edits(movie: &[u8], names: &ClassNames) -> Result<Vec<(Range<usize>, Vec<u8>)>> {
    let version = movie[3];
    let mut reader = Reader::new(&movie[8..], version);
    reader.read_rectangle()?;
    reader.read_u16()?; // Frame rate
    reader.read_u16()?; // Frame count
    let mut symbol_matches = ClassMatches::default();
    let mut abc_matches = ClassMatches::default();
    let mut edits = Vec::new();
    loop {
        let tag_start = reader.as_slice();
        let (code, length) = reader.read_tag_code_and_length()?;
        let payload = reader.read_slice(length)?;
        if code == TagCode::End as u16 {
            if length != 0 || !reader.as_slice().is_empty() {
                return Err(invalid("invalid end of SWF tag stream").into());
            }
            break;
        }
        if !matches!(
            TagCode::from_u16(code),
            Some(TagCode::SymbolClass | TagCode::DoAbc | TagCode::DoAbc2)
        ) {
            continue;
        }
        let start = movie.len() - tag_start.len();
        let end = movie.len() - reader.as_slice().len();
        let updated = match Reader::new(&movie[start..end], version).read_tag()? {
            Tag::SymbolClass(links) => rewrite_symbols(links, length, names, &mut symbol_matches)?,
            Tag::DoAbc(abc) => rewrite_abc(&[], abc, names, &mut abc_matches)?,
            Tag::DoAbc2(abc) => {
                let prefix = &payload[..payload.len() - abc.data.len()];
                rewrite_abc(prefix, abc.data, names, &mut abc_matches)?
            }
            _ => unreachable!(),
        };
        if let Some(data) = updated {
            edits.push((start..end, encode_tag(code, &data)));
        }
    }
    if !symbol_matches.asset || !abc_matches.asset {
        return Err(invalid("target model class missing from SymbolClass or DoABC").into());
    }
    if names.config.is_some() && (!symbol_matches.config || !abc_matches.config) {
        return Err(invalid("target model config class missing from SymbolClass or DoABC").into());
    }
    Ok(edits)
}

fn transform_movie(
    target_bytes: Vec<u8>,
    source_id: u32,
    target_id: u32,
    kind: MorphKind,
) -> Result<Vec<u8>> {
    let mut movie = unpack_movie(target_bytes)?;
    let edits = collect_edits(&movie, &ClassNames::new(source_id, target_id, kind))?;
    if edits.iter().all(|(range, tag)| range.len() == tag.len()) {
        for (range, tag) in edits {
            movie[range].copy_from_slice(&tag);
        }
        return Ok(movie);
    }

    // A different-width ID changes tag lengths. Copy each untouched range once and
    // emit FWS so Ruffle can consume it without recompressing the bitmaps.
    let size = edits
        .iter()
        .fold(movie.len(), |size, (range, tag)| size - range.len() + tag.len());
    if size > MAX_MOVIE_SIZE {
        return Err(invalid("adapted SWF exceeds maximum length").into());
    }
    let mut adapted = Vec::with_capacity(size);
    let mut next = 0;
    for (range, tag) in edits {
        adapted.extend_from_slice(&movie[next..range.start]);
        adapted.extend_from_slice(&tag);
        next = range.end;
    }
    adapted.extend_from_slice(&movie[next..]);
    adapted[4..8].copy_from_slice(&(size as u32).to_le_bytes());
    Ok(adapted)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use ruffle_core::swf::avm2::types::{AbcFile, ConstantPool};
    use ruffle_core::swf::{self, Compression, Header};

    const OPAQUE: &[u8] = b"\x01\x00\x00\x00\x00\x00PetMovie1234_1 pet1234_2_5";
    const ABC_PREFIX: &[u8] = b"\x01\x00\x00\x00abc_label\x00";

    pub(in crate::backends::navigator) fn fixture(
        compression: Compression,
        code: TagCode,
        names: &[&str],
    ) -> Vec<u8> {
        let mut symbols = (names.len() as u16).to_le_bytes().to_vec();
        for (index, name) in names.iter().enumerate() {
            symbols.extend_from_slice(&(index as u16 + 2).to_le_bytes());
            symbols.extend_from_slice(name.as_bytes());
            symbols.push(0);
        }

        let mut abc = if code == TagCode::DoAbc2 {
            ABC_PREFIX.to_vec()
        } else {
            Vec::new()
        };
        AbcWriter::new(&mut abc)
            .write(AbcFile {
                major_version: 46,
                minor_version: 16,
                constant_pool: ConstantPool {
                    ints: Vec::new(),
                    uints: Vec::new(),
                    doubles: Vec::new(),
                    strings: names
                        .iter()
                        .map(|name| name.replace('.', ":").into_bytes())
                        .collect(),
                    namespaces: Vec::new(),
                    namespace_sets: Vec::new(),
                    multinames: Vec::new(),
                },
                methods: Vec::new(),
                metadata: Vec::new(),
                instances: Vec::new(),
                classes: Vec::new(),
                scripts: Vec::new(),
                method_bodies: Vec::new(),
            })
            .unwrap();
        let mut raw = encode_tag(TagCode::DefineBinaryData as u16, OPAQUE);
        raw.extend(encode_tag(code as u16, &abc));
        raw.extend(encode_tag(TagCode::SymbolClass as u16, &symbols));
        raw.extend(encode_tag(TagCode::End as u16, &[]));
        let mut header = Header::default_with_swf_version(18);
        header.compression = compression;
        let mut out = Vec::new();
        swf::write::write_swf_raw_tags(&header, &raw, &mut out).unwrap();
        out
    }

    fn check(movie: &[u8], names: &[&str]) {
        assert!(movie.starts_with(b"FWS"));
        assert_eq!(
            u32::from_le_bytes(movie[4..8].try_into().unwrap()) as usize,
            movie.len()
        );
        let buf = swf::decompress_swf(movie).unwrap();
        let parsed = swf::parse_swf(&buf).unwrap();
        let [Tag::DefineBinaryData(binary), abc, Tag::SymbolClass(symbols)] = parsed.tags.as_slice()
        else {
            panic!("unexpected tag layout");
        };
        assert_eq!(binary.data, &OPAQUE[6..]);
        let abc = match abc {
            Tag::DoAbc(bytes) => bytes,
            Tag::DoAbc2(abc) => {
                assert_eq!(abc.flags.bits(), 1);
                assert_eq!(abc.name.as_bytes(), b"abc_label");
                abc.data
            }
            _ => panic!("unexpected ABC tag"),
        };
        let abc = AbcReader::new(abc).read().unwrap();
        for ((index, symbol), name) in symbols.iter().enumerate().zip(names) {
            assert_eq!(symbol.id, index as u16 + 2);
            assert_eq!(symbol.class_name.as_bytes(), name.as_bytes());
        }
        assert_eq!(symbols.len(), names.len());
        assert_eq!(
            abc.constant_pool.strings,
            names
                .iter()
                .map(|name| name.replace('.', ":").into_bytes())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn backpack_uses_variable_width_names_and_keeps_opaque_data() {
        let kind = MorphKind::Backpack { index: 0 };
        for (target, source) in [(123, 1234), (1234, 12345), (12345, 123)] {
            let names = [
                format!("mmo.peticon.PetMovie{target}_1"),
                format!("mmo.peticon.petmovie{target}_1"),
                format!("mmo.peticon.PetMovie{target}Config"),
                format!("mmo.peticon.PetMovie{target}Config_MovieConfig"),
                format!("mmo.peticon.Unrelated{target}"),
            ];
            let before = names.each_ref().map(String::as_str);
            let data = fixture(Compression::None, TagCode::DoAbc2, &before);
            let rewritten = transform_movie(data, source, target, kind).unwrap();
            let expected = [
                format!("mmo.peticon.PetMovie{source}_1"),
                format!("mmo.peticon.petmovie{source}_1"),
                format!("mmo.peticon.PetMovie{source}Config"),
                format!("mmo.peticon.PetMovie{source}Config_MovieConfig"),
                names[4].clone(),
            ];
            check(&rewritten, &expected.each_ref().map(String::as_str));
        }
    }

    #[test]
    fn battle_rewrites_config_only_in_first_part() {
        let before = [
            "mmo.bitmap.pet1234_2_5",
            "Pet1234Config_2",
            "Pet1234Config_2_MovieConfig",
            "pet1234_1_5",
        ];
        for part in 0..=2 {
            let data = fixture(Compression::Zlib, TagCode::DoAbc, &before);
            let output = transform_movie(data, 123, 1234, MorphKind::Battle { face: 2, part }).unwrap();
            let mut expected = before;
            expected[0] = "mmo.bitmap.pet123_2_5";
            if part == 0 {
                expected[1] = "Pet123Config_2";
                expected[2] = "Pet123Config_2_MovieConfig";
            }
            check(&output, &expected);
        }
    }

    #[test]
    fn poster_rewrites_exact_id_in_zws_and_keeps_other_ids() {
        let before = [
            "mmo.petskin.view.Background1234",
            "Background12345",
            "petskinbackground1234",
        ];
        let data = fixture(Compression::Lzma, TagCode::DoAbc2, &before);
        let output = transform_movie(data, 123, 1234, MorphKind::Poster).unwrap();
        check(
            &output,
            &[
                "mmo.petskin.view.Background123",
                "Background12345",
                "petskinbackground123",
            ],
        );
    }

    #[test]
    fn rejects_bad_headers_tags_and_missing_target_classes() {
        let kind = MorphKind::Battle { face: 2, part: 0 };
        let names = ["pet1234_2_5", "Pet1234Config_2"];
        let valid = fixture(Compression::None, TagCode::DoAbc2, &names);
        let mut missing_end = valid[..valid.len() - 2].to_vec();
        let length = missing_end.len() as u32;
        missing_end[4..8].copy_from_slice(&length.to_le_bytes());
        let mut malformed = valid.clone();
        malformed[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut truncated_cws = fixture(Compression::Zlib, TagCode::DoAbc2, &names);
        truncated_cws.pop();
        let missing_config = fixture(Compression::None, TagCode::DoAbc2, &["pet1234_2_5"]);
        for bytes in [
            Vec::new(),
            valid[..30].to_vec(),
            malformed,
            missing_end,
            truncated_cws,
            missing_config,
        ] {
            assert!(transform_movie(bytes, 123, 1234, kind).is_err());
        }
    }
}
