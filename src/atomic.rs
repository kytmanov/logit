use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::AppError;

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let parent = resolved.parent().ok_or_else(|| {
        AppError::config(format!(
            "missing parent directory for {}",
            resolved.display()
        ))
    })?;

    fs::create_dir_all(parent)
        .map_err(|error| AppError::config(format!("create dir {}: {error}", parent.display())))?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp_path = parent.join(format!(".tmp.{}.{}", std::process::id(), nonce));

    let mut file = create_temp_file(&tmp_path)?;
    file.write_all(bytes)
        .map_err(|error| AppError::config(format!("write {}: {error}", tmp_path.display())))?;
    file.sync_all()
        .map_err(|error| AppError::config(format!("fsync {}: {error}", tmp_path.display())))?;
    drop(file);

    fs::rename(&tmp_path, &resolved).map_err(|error| {
        AppError::config(format!(
            "rename {} -> {}: {error}",
            tmp_path.display(),
            resolved.display()
        ))
    })
}

#[cfg(unix)]
fn create_temp_file(path: &Path) -> Result<fs::File, AppError> {
    use std::os::unix::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| AppError::config(format!("create {}: {error}", path.display())))
}

#[cfg(not(unix))]
fn create_temp_file(path: &Path) -> Result<fs::File, AppError> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| AppError::config(format!("create {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn atomic_write_creates_target_with_secure_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("secrets.toml");

        atomic_write(&path, b"secret = true\n").expect("atomic write succeeds");

        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
