use std::{env, fs, path::{Path, PathBuf}};
use sha2::{Digest, Sha256};

fn files(root: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("Run prepare_engine_assets.py first") {
        let path = entry.expect("resource entry").path();
        if path.is_dir() { files(&path, output); } else { output.push(path); }
    }
}

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("resources");
    println!("cargo:rerun-if-changed=resources");
    let mut entries = Vec::new();
    files(&root, &mut entries);
    entries.sort();
    let mut source = String::from("pub static ASSETS: &[Asset] = &[\n");
    for path in entries {
        let name = path.strip_prefix(&root).unwrap().to_str().unwrap().replace('\\', "/");
        let hash: [u8; 32] = Sha256::digest(fs::read(&path).unwrap()).into();
        source.push_str(&format!("Asset {{ path: {name:?}, bytes: include_bytes!({:?}), sha256: {hash:?} }},\n", path.to_str().unwrap()));
    }
    source.push_str("];\n");
    fs::write(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("assets.rs"), source).unwrap();
}
