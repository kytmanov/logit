use std::collections::BTreeMap;
use std::fmt;
use std::fs;

use serde::{Deserialize, Serialize};

use crate::atomic::atomic_write;
use crate::error::AppError;
use crate::paths::{AppDirs, validate_policy};

pub trait SecretStore {
    fn load_profile(&self, profile: &str) -> Result<Option<ProfileSecrets>, AppError>;
    fn save_profile(&self, profile: &str, secrets: &ProfileSecrets) -> Result<(), AppError>;
}

#[derive(Debug, Clone)]
pub struct FileSecretStore {
    dirs: AppDirs,
}

impl FileSecretStore {
    pub fn new(dirs: AppDirs) -> Result<Self, AppError> {
        validate_policy(&dirs)?;
        Ok(Self { dirs })
    }
}

impl SecretStore for FileSecretStore {
    fn load_profile(&self, profile: &str) -> Result<Option<ProfileSecrets>, AppError> {
        validate_policy(&self.dirs)?;
        let path = self.dirs.secrets_file();

        if !path.exists() {
            return Ok(None);
        }

        ensure_secure_permissions(&path)?;

        let raw = fs::read_to_string(&path)
            .map_err(|error| AppError::config(format!("read {}: {error}", path.display())))?;
        let secrets_file: SecretsFile = toml_edit::de::from_str(&raw)
            .map_err(|error| AppError::config(format!("parse {}: {error}", path.display())))?;

        Ok(secrets_file.profiles.get(profile).cloned())
    }

    fn save_profile(&self, profile: &str, secrets: &ProfileSecrets) -> Result<(), AppError> {
        validate_policy(&self.dirs)?;
        let path = self.dirs.secrets_file();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AppError::config(format!("create dir {}: {error}", parent.display()))
            })?;
        }

        let mut secrets_file = if path.exists() {
            let raw = fs::read_to_string(&path)
                .map_err(|error| AppError::config(format!("read {}: {error}", path.display())))?;
            toml_edit::de::from_str::<SecretsFile>(&raw)
                .map_err(|error| AppError::config(format!("parse {}: {error}", path.display())))?
        } else {
            SecretsFile::default()
        };

        secrets_file
            .profiles
            .insert(profile.to_owned(), secrets.clone());

        let serialized = toml_edit::ser::to_string(&secrets_file)
            .map_err(|error| AppError::config(format!("serialize secrets: {error}")))?;

        atomic_write(&path, serialized.as_bytes())?;
        set_secure_permissions(&path)?;
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSecrets {
    pub tempo_token: String,
    pub jira_token: String,
}

impl fmt::Debug for ProfileSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileSecrets")
            .field("tempo_token", &"[REDACTED]")
            .field("jira_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SecretsFile {
    profiles: BTreeMap<String, ProfileSecrets>,
}

#[cfg(unix)]
fn ensure_secure_permissions(path: &std::path::Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path)
        .map_err(|error| AppError::config(format!("stat {}: {error}", path.display())))?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(AppError::config(format!(
            "refusing to use {} with permissions {:o}; expected 600",
            path.display(),
            mode
        )));
    }

    Ok(())
}

#[cfg(not(unix))]
fn ensure_secure_permissions(_path: &std::path::Path) -> Result<(), AppError> {
    Ok(())
}

#[cfg(unix)]
fn set_secure_permissions(path: &std::path::Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| AppError::config(format!("chmod 600 {}: {error}", path.display())))
}

#[cfg(not(unix))]
fn set_secure_permissions(_path: &std::path::Path) -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppDirs;

    fn temp_dirs() -> (tempfile::TempDir, AppDirs) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        (
            temp,
            AppDirs {
                config: root.join("config"),
                data: root.join("data"),
                cache: root.join("cache"),
            },
        )
    }

    #[test]
    fn round_trips_profile_secrets() {
        let (_temp, dirs) = temp_dirs();
        let store = FileSecretStore::new(dirs).expect("store");
        let secrets = ProfileSecrets {
            tempo_token: String::from("tempo-secret"),
            jira_token: String::from("jira-secret"),
        };

        store
            .save_profile("default", &secrets)
            .expect("secrets save");
        let loaded = store
            .load_profile("default")
            .expect("secrets load")
            .expect("secrets exist");

        assert_eq!(loaded, secrets);
    }

    #[test]
    fn debug_redacts_secret_values() {
        let secrets = ProfileSecrets {
            tempo_token: String::from("tempo-secret"),
            jira_token: String::from("jira-secret"),
        };

        let debug = format!("{secrets:?}");
        assert!(!debug.contains("tempo-secret"));
        assert!(!debug.contains("jira-secret"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_wide_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (_temp, dirs) = temp_dirs();
        let store = FileSecretStore::new(dirs.clone()).expect("store");
        let path = dirs.secrets_file();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir");
        }
        fs::write(
            &path,
            "[profiles.default]\ntempo_token = \"tempo-secret\"\njira_token = \"jira-secret\"\n",
        )
        .expect("write secrets file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod 644");

        let error = store
            .load_profile("default")
            .expect_err("wide perms should fail");

        assert!(error.to_string().contains("expected 600"));
    }

    #[test]
    fn policy_violation_is_rejected() {
        let dirs = AppDirs {
            config: std::path::PathBuf::from("/tmp/logit/config"),
            data: std::path::PathBuf::from("/tmp/logit/config/data"),
            cache: std::path::PathBuf::from("/tmp/logit/cache"),
        };

        let error = FileSecretStore::new(dirs).expect_err("nested data dir should fail");
        assert!(error.to_string().contains("--data-dir"));
    }
}
