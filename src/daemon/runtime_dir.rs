//! Validate the private daemon namespace before opening any lock or socket.
use anyhow::{bail, Context, Result};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::Path;

pub fn validate(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!("runtime directory must be absolute: {}", path.display());
    }
    if !path.try_exists()? {
        // A dangling symlink must be refused, too. Never chmod a symlink target.
        if std::fs::symlink_metadata(path).is_ok() {
            bail!("runtime directory must not be a symlink");
        }
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(path)
            .context("creating private runtime directory")?;
    }
    validate_metadata(&std::fs::symlink_metadata(path)?, unsafe {
        libc::geteuid()
    })
}

fn validate_metadata(metadata: &std::fs::Metadata, uid: u32) -> Result<()> {
    if metadata.file_type().is_symlink() {
        bail!("runtime directory must not be a symlink");
    }
    if !metadata.is_dir() {
        bail!("runtime path must be a directory");
    }
    if metadata.uid() != uid {
        bail!("runtime directory has a foreign owner (expected uid {uid})");
    }
    if metadata.permissions().mode() & 0o7777 != 0o700 {
        bail!("runtime directory must have mode 0700");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn validation_table() {
        let root = std::env::temp_dir().join(format!("fbt-runtime-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let valid = root.join("private");
        validate(&valid).unwrap();
        assert!(validate(&valid).is_ok());
        let public = root.join("public");
        std::fs::create_dir_all(&public).unwrap();
        std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = root.join("symlink");
        symlink(&valid, &link).unwrap();
        let dangling = root.join("dangling");
        symlink(root.join("absent"), &dangling).unwrap();
        for (label, path, reason) in [
            ("relative", Path::new("relative"), "absolute"),
            ("0755", public.as_path(), "0700"),
            ("symlink", link.as_path(), "symlink"),
            ("dangling symlink", dangling.as_path(), "symlink"),
        ] {
            let error = validate(path).unwrap_err().to_string();
            println!("runtime {label}: refused {error}");
            assert!(error.contains(reason));
        }
        // No chown privilege is needed to exercise the exact owner predicate.
        let metadata = std::fs::symlink_metadata(&valid).unwrap();
        let foreign = validate_metadata(&metadata, metadata.uid().wrapping_add(1)).unwrap_err();
        println!("runtime foreign uid: refused {foreign}");
        assert!(foreign.to_string().contains("foreign owner"));
        assert_eq!(
            std::fs::metadata(&public).unwrap().permissions().mode() & 0o777,
            0o755
        );
        println!("runtime owned 0700: accepted {}", valid.display());
        std::fs::remove_dir_all(root).unwrap();
    }
}
