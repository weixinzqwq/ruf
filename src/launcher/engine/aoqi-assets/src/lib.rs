//! Build-time resource bundle. Paths are exact logical names, never filesystem paths.
pub struct Asset {
    pub path: &'static str,
    pub bytes: &'static [u8],
    pub sha256: [u8; 32],
}
include!(concat!(env!("OUT_DIR"), "/assets.rs"));

pub fn get(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|asset| asset.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn every_embedded_resource_matches_its_build_digest() {
        assert_eq!(ASSETS.len(), 27);
        for asset in ASSETS {
            assert_eq!(<[u8; 32]>::from(Sha256::digest(asset.bytes)), asset.sha256, "{}", asset.path);
        }
        assert!(get("host.swf").is_some());
        assert!(get("../host.swf").is_none());
        assert!(get("HOST.SWF").is_none());
        assert!(get("missing").is_none());
    }
}
