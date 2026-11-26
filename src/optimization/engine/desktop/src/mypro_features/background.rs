use base64::{Engine, engine::general_purpose::STANDARD};
use image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, codecs::jpeg::JpegEncoder,
    imageops::FilterType,
};
use serde_json::{Value, json};
use std::fs;
use std::io::{self, BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 560;
const CACHE: &str = "battle-background.jpg";
static PUBLISHED_REVISION: AtomicU32 = AtomicU32::new(0);

#[derive(Default)]
struct Background {
    root: PathBuf,
    jpeg: String,
    revision: u32,
    stage: &'static str,
    error: Option<String>,
}

impl Background {
    fn snapshot(&self) -> Value {
        json!({"revision":self.revision,"configured":!self.jpeg.is_empty(),
            "stage":self.stage,"error":self.error})
    }

    fn finish(&mut self, result: Result<Option<String>, String>) {
        self.stage = "ready";
        self.error = None;
        match result {
            Ok(Some(jpeg)) if jpeg != self.jpeg => {
                self.jpeg = jpeg;
                self.revision = self.revision.wrapping_add(1);
            }
            Ok(_) => {}
            Err(e) => {
                self.stage = "failed";
                self.error = Some(e);
            }
        }
    }

    fn publish(&mut self, result: Result<Option<String>, String>) {
        let previous = self.revision;
        self.finish(result);
        if self.revision != previous {
            PUBLISHED_REVISION.store(self.revision, Ordering::Release);
        }
    }
}

fn state() -> &'static Mutex<Background> {
    static STATE: OnceLock<Mutex<Background>> = OnceLock::new();
    STATE.get_or_init(Default::default)
}

pub fn revision() -> u32 {
    PUBLISHED_REVISION.load(Ordering::Acquire)
}
pub fn jpeg() -> String {
    state().lock().unwrap().jpeg.clone()
}

pub fn init(root: &Path) {
    let mut s = state().lock().unwrap();
    if s.root == root {
        return;
    }
    s.publish(Ok(Some(String::new())));
    *s = Background {
        root: root.to_owned(),
        revision: s.revision,
        stage: "loading",
        ..Default::default()
    };
    let root = s.root.clone();
    std::thread::spawn(move || {
        let result = read_cache(&root.join(CACHE));
        let mut s = state().lock().unwrap();
        if s.root == root {
            s.publish(result);
        }
    });
}

fn read_cache(path: &Path) -> Result<Option<String>, String> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let dimensions = ImageReader::with_format(Cursor::new(&bytes), ImageFormat::Jpeg)
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if dimensions != (WIDTH, HEIGHT) {
        return Err("已保存的图片不是 960×560 JPEG。".into());
    }
    Ok(Some(STANDARD.encode(bytes)))
}

fn encode(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = ImageReader::with_format(BufReader::new(file), ImageFormat::Jpeg)
        .into_decoder()
        .map_err(|_| "请选择有效的 JPEG 图片。".to_string())?;
    let (width, height) = decoder.dimensions();
    if u64::from(width) * u64::from(height) > 64 * 1024 * 1024 {
        return Err("JPEG 超过 6710 万像素，请选择较小的图片。".into());
    }
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    decoded.apply_orientation(orientation);
    let (w, h) = (decoded.width(), decoded.height());
    // Crop before scaling so even a very wide panorama produces a bounded output.
    let cw = u64::from(w)
        .min(u64::from(h) * u64::from(WIDTH) / u64::from(HEIGHT))
        .max(1) as u32;
    let ch = u64::from(h)
        .min(u64::from(w) * u64::from(HEIGHT) / u64::from(WIDTH))
        .max(1) as u32;
    let frame = decoded
        .crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch)
        .resize_exact(WIDTH, HEIGHT, FilterType::CatmullRom);
    let mut output = Vec::new();
    JpegEncoder::new_with_quality(&mut output, 88)
        .encode_image(&frame)
        .map_err(|e| e.to_string())?;
    Ok(output)
}

fn choose(root: &Path) -> Result<Option<String>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_title("选择战斗背景")
        .add_filter("JPEG", &["jpg", "jpeg"])
        .pick_file()
    else {
        return Ok(None);
    };
    {
        let mut s = state().lock().unwrap();
        if s.root != root {
            return Ok(None);
        }
        s.stage = "saving";
    }
    let bytes = encode(&path)?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    crate::aoqi_storage::atomic_write(&root.join(CACHE), &bytes)?;
    Ok(Some(STANDARD.encode(bytes)))
}

pub fn command(name: &str) -> Option<Result<Value, String>> {
    if !matches!(
        name,
        "background.read" | "background.choose" | "background.clear"
    ) {
        return None;
    }
    let mut s = state().lock().unwrap();
    if name == "background.read" {
        return Some(Ok(s.snapshot()));
    }
    if matches!(s.stage, "loading" | "choosing" | "saving") {
        return Some(Err("Busy: 正在处理背景图片".into()));
    }
    let pick = name == "background.choose";
    s.stage = if pick { "choosing" } else { "saving" };
    s.error = None;
    let root = s.root.clone();
    let snapshot = s.snapshot();
    std::thread::spawn(move || {
        let result = if pick {
            choose(&root)
        } else {
            match fs::remove_file(root.join(CACHE)) {
                Ok(()) => Ok(Some(String::new())),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Some(String::new())),
                Err(e) => Err(e.to_string()),
            }
        };
        let mut s = state().lock().unwrap();
        if s.root == root {
            s.publish(result);
        }
    });
    Some(Ok(snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_is_cropped_at_selection_and_reloaded_without_reencoding() {
        let dir = std::env::temp_dir().join(format!(
            "aola-bg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source.jpg");
        let image = image::RgbImage::from_fn(1920, 560, |x, _| {
            if (460..1460).contains(&x) {
                image::Rgb([225, 40, 60])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        JpegEncoder::new_with_quality(fs::File::create(&source).unwrap(), 88)
            .encode_image(&image)
            .unwrap();
        let out = encode(&source).unwrap();
        let decoded = image::load_from_memory_with_format(&out, ImageFormat::Jpeg)
            .unwrap()
            .into_rgb8();
        assert_eq!(decoded.dimensions(), (960, 560));
        for x in [0, 480, 959] {
            assert!(decoded.get_pixel(x, 280)[0] > 200);
        }
        let cache = dir.join(CACHE);
        crate::aoqi_storage::atomic_write(&cache, &out).unwrap();
        let mut bg = Background::default();
        bg.finish(read_cache(&cache));
        assert_eq!(STANDARD.decode(&bg.jpeg).unwrap(), out);
        assert_eq!(bg.revision, 1);
        bg.finish(read_cache(&cache));
        assert_eq!(bg.revision, 1);
        fs::write(&source, b"invalid").unwrap();
        assert!(encode(&source).is_err());
        bg.finish(Err("invalid JPEG".into()));
        assert_eq!(bg.revision, 1);
        assert!(!bg.jpeg.is_empty());
        bg.finish(Ok(Some(String::new())));
        assert_eq!(bg.revision, 2);
        assert!(bg.jpeg.is_empty());
        fs::remove_file(source).unwrap();
        fs::remove_file(cache).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
