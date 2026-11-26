//! Save only completed official movie responses selected by the navigator.
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub(super) fn write(directory: &Path, resource: &str, version: &str, bytes: &[u8]) -> io::Result<()> {
    let name = match resource {
        "pet/newpetpackage" => "newpetpackage",
        "library/util" => "library_util",
        _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "Unknown resource")),
    };
    if version.is_empty() || version.len() > 32 || !version.bytes().all(|c| c.is_ascii_digit()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid resource version"));
    }
    if bytes.len() < 8 || !matches!(&bytes[..3], b"FWS" | b"CWS" | b"ZWS") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Response has no SWF header"));
    }
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("{name}~{version}.swf"));
    let mut pending = tempfile::NamedTempFile::new_in(directory)?;
    pending.write_all(bytes)?;
    pending.flush()?;
    match pending.persist_noclobber(&path) {
        Ok(_) => Ok(()),
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
            if fs::read(path)? == bytes {
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::InvalidData, "Different responses for the same version"))
            }
        }
        Err(error) => Err(error.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aola_swf_capture_preserves_complete_bytes_and_rejects_conflicts() {
        let directory = tempfile::tempdir().unwrap();
        let bytes = b"FWS\x0a\x08\x00\x00\x00";
        let pet = "pet/newpetpackage";
        write(directory.path(), pet, "20261121", bytes).unwrap();
        write(directory.path(), pet, "20261121", bytes).unwrap();
        assert!(write(directory.path(), pet, "20261121", b"CWS\x0a\x08\x00\x00\x00").is_err());
        assert!(write(directory.path(), pet, "../bad", bytes).is_err());
        assert!(write(directory.path(), pet, "12", b"<html>").is_err());
        assert!(write(directory.path(), "any", "12", bytes).is_err());
        write(directory.path(), "library/util", "20261121", bytes).unwrap();
        assert_eq!(fs::read(directory.path().join("newpetpackage~20261121.swf")).unwrap(), bytes);
        assert_eq!(fs::read(directory.path().join("library_util~20261121.swf")).unwrap(), bytes);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}
