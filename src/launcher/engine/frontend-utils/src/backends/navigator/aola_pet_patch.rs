//! Explicit A/B test of one exact, public game resource. Other URLs use HTTP.
use ruffle_core::backend::navigator::{NavigationMethod, Request};
use sha2::{Digest, Sha256};
use std::{io, path::Path, sync::Arc};
use url::Url;

const RESOURCE: &str = "/play/pet/newpetpackage~2026102936286521.swf";
const ORIGINAL: &str = "5011d9a09e6b5cd1e82b1bae36ffc6ad0ce84a1f89db3b998a61b308cbfdbd87";
const PATCHED: &str = "308cf278d40ce80001cbbf2e9d2e4bc35ecbf120cc379c17fe4767cd926f122c";

pub(super) struct PetPatch { bytes: Arc<[u8]>, optimized: bool }

impl PetPatch {
    pub fn load(root: &Path, optimized: bool) -> io::Result<Self> {
        let file = if optimized { "newpetpackage-detail.swf" } else { "newpetpackage-original.swf" };
        let bytes = std::fs::read(root.join(file))?;
        let expected = if optimized { PATCHED } else { ORIGINAL };
        if format!("{:x}", Sha256::digest(&bytes)) != expected {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Aola pet SWF hash mismatch"));
        }
        Ok(Self { bytes: bytes.into(), optimized })
    }
    pub fn accepts(request: &Request, url: &Url) -> bool {
        request.method() == NavigationMethod::Get && request.body().is_none()
            && matches!(url.scheme(), "http" | "https")
            && url.host_str() == Some("aola.100bt.com") && url.port().is_none()
            && url.username().is_empty() && url.password().is_none()
            && url.path() == RESOURCE
    }
    pub fn bytes(&self) -> Vec<u8> {
        tracing::info!(target: "aola_pet_patch", optimized=self.optimized, resource=RESOURCE, "Aola pet SWF replacement hit");
        self.bytes.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aola_pet_patch_is_exact_version_and_origin() {
        let url = format!("https://aola.100bt.com{RESOURCE}");
        assert!(PetPatch::accepts(&Request::get(url.clone()), &Url::parse(&url).unwrap()));
        assert!(!PetPatch::accepts(&Request::post(url.clone(), None), &Url::parse(&url).unwrap()));
        let mut with_body = Request::get(url.clone());
        with_body.set_body((vec![1], "application/octet-stream".into()));
        assert!(!PetPatch::accepts(&with_body, &Url::parse(&url).unwrap()));
        for bad in [url.replace("100bt.com", "100bt.com.invalid"), url.replace("2026102936286521", "999"), url.replace("https:", "file:"), url.replace("/play/pet/", "/play/")] {
            assert!(!PetPatch::accepts(&Request::get(bad.clone()), &Url::parse(&bad).unwrap()));
        }
    }
}
