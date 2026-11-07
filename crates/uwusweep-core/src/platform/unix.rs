//! What Linux and macOS share.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use crate::{Error, Result};

/// Creates `path` if needed and makes sure only root can change it: not a
/// link, owned by root, mode 0755 (0700 when `private`). The parent must be
/// root's already, so nobody can swap the folder between check and use.
pub fn secure_dir(path: &Path, private: bool) -> Result<()> {
    match fs::create_dir(path) {
        Err(e) if e.kind() != ErrorKind::AlreadyExists => return Err(e.into()),
        _ => {}
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != 0 {
        return Err(Error::Failed(format!(
            "{} is not a folder owned by root",
            path.display()
        )));
    }
    let mode = if private { 0o700 } else { 0o755 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    // Manifests must stay readable for the app's unelevated list.
    // SAFETY: umask only swaps the process's file creation mask.
    unsafe { libc::umask(0o022) };
    Ok(())
}
