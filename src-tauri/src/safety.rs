//! Small, testable file-system boundaries for original-photo safety.

use crate::{hash_file, KeepframeError, Result};
use std::{fs, path::Path};

pub(crate) fn ensure_capacity(required_bytes: u64, available_bytes: u64) -> Result<()> {
    let safety_margin = required_bytes / 20 + 256 * 1024 * 1024;
    let minimum = required_bytes.saturating_add(safety_margin);
    if available_bytes < minimum {
        return Err(KeepframeError::Message(format!(
            "Not enough free space for a verified import. Required at least {minimum} bytes including the safety margin; {available_bytes} bytes are available."
        )));
    }
    Ok(())
}

pub(crate) fn copy_and_verify(source: &Path, staged: &Path, expected_hash: &str) -> Result<()> {
    fs::copy(source, staged)?;
    fs::OpenOptions::new().write(true).open(staged)?.sync_all()?;
    if hash_file(staged)? != expected_hash {
        let _ = fs::remove_file(staged);
        return Err(KeepframeError::Message(
            "Staged copy did not match the source hash; the source was retained.".into(),
        ));
    }
    Ok(())
}

pub(crate) fn promote_and_verify(staged: &Path, target: &Path, expected_hash: &str) -> Result<()> {
    fs::rename(staged, target)?;
    if hash_file(target)? != expected_hash {
        let _ = fs::remove_file(target);
        return Err(KeepframeError::Message(
            "Managed copy did not match the verified staged hash; the source was retained.".into(),
        ));
    }
    Ok(())
}

/// Deletes a Move-import source only after both source and managed destination
/// still match the original digest.  Files inside the library are never removed.
pub(crate) fn delete_verified_external_source(
    source: &Path,
    managed: &Path,
    expected_hash: &str,
    library_root: &Path,
) -> Result<()> {
    if hash_file(managed)? != expected_hash {
        return Err(KeepframeError::Message(
            "Managed copy changed before source deletion; the source was retained.".into(),
        ));
    }
    if hash_file(source)? != expected_hash {
        return Err(KeepframeError::Message(
            "Source changed after verification and was retained.".into(),
        ));
    }
    let source = source.canonicalize().unwrap_or_else(|_| source.to_path_buf());
    let library_root = library_root
        .canonicalize()
        .unwrap_or_else(|_| library_root.to_path_buf());
    if source.starts_with(&library_root) {
        return Err(KeepframeError::Message(
            "A source inside the master library was retained.".into(),
        ));
    }
    if !source.is_file() {
        return Err(KeepframeError::Message(
            "The source file is no longer available and was not deleted.".into(),
        ));
    }
    fs::remove_file(source)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

    #[test]
    fn insufficient_space_fails_before_any_copy() {
        assert!(ensure_capacity(1024, 0).is_err());
        assert!(ensure_capacity(1024, 300 * 1024 * 1024).is_ok());
    }

    #[test]
    fn checksum_mismatch_removes_staging_and_retains_source() {
        let root = std::env::temp_dir().join(format!("keepframe-safety-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.jpg");
        let staged = root.join("staged.jpg");
        fs::write(&source, b"original pixels").unwrap();
        assert!(copy_and_verify(&source, &staged, &digest(b"different pixels")).is_err());
        assert!(source.is_file());
        assert!(!staged.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verified_copy_and_promotion_preserve_the_source_and_digest() {
        let root = std::env::temp_dir().join(format!("keepframe-copy-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.jpg");
        let staged = root.join("staging").join("source.jpg");
        let managed = root.join("Originals").join("source.jpg");
        fs::create_dir_all(staged.parent().unwrap()).unwrap();
        fs::create_dir_all(managed.parent().unwrap()).unwrap();
        fs::write(&source, b"original pixels").unwrap();
        let expected = digest(b"original pixels");
        copy_and_verify(&source, &staged, &expected).unwrap();
        promote_and_verify(&staged, &managed, &expected).unwrap();
        assert!(source.is_file());
        assert!(!staged.exists());
        assert_eq!(hash_file(&managed).unwrap(), expected);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn move_deletes_only_an_unchanged_verified_external_source() {
        let root = std::env::temp_dir().join(format!("keepframe-source-{}", Uuid::new_v4()));
        let library = root.join("library");
        fs::create_dir_all(&library).unwrap();
        let source = root.join("camera.jpg");
        let managed = library.join("managed.jpg");
        fs::write(&source, b"original pixels").unwrap();
        fs::copy(&source, &managed).unwrap();
        let expected = digest(b"original pixels");
        delete_verified_external_source(&source, &managed, &expected, &library).unwrap();
        assert!(!source.exists());
        assert!(managed.is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
