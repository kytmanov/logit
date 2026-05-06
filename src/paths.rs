use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::domain::PathOverrides;
use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppDirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl AppDirs {
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn secrets_file(&self) -> PathBuf {
        self.data.join("secrets.toml")
    }

    pub fn calendar_file(&self, profile: &str) -> PathBuf {
        self.cache.join(profile).join("calendar.json")
    }
}

pub fn default_dirs() -> Result<AppDirs, AppError> {
    resolve_dirs(&PathOverrides::default())
}

pub fn resolve_dirs(overrides: &PathOverrides) -> Result<AppDirs, AppError> {
    let project_dirs = ProjectDirs::from("", "", "logit")
        .ok_or_else(|| AppError::config("unable to resolve default application directories"))?;

    let config = overrides
        .config_dir
        .clone()
        .or_else(|| std::env::var_os("LOGIT_CONFIG_DIR").map(PathBuf::from))
        .unwrap_or_else(|| default_config_dir(&project_dirs));
    let data = overrides
        .data_dir
        .clone()
        .or_else(|| std::env::var_os("LOGIT_DATA_DIR").map(PathBuf::from))
        .unwrap_or_else(|| default_data_dir(&project_dirs));
    let cache = overrides
        .cache_dir
        .clone()
        .or_else(|| std::env::var_os("LOGIT_CACHE_DIR").map(PathBuf::from))
        .unwrap_or_else(|| project_dirs.cache_dir().to_path_buf());

    let dirs = AppDirs {
        config,
        data,
        cache,
    };
    validate_policy(&dirs)?;
    Ok(dirs)
}

fn default_config_dir(project_dirs: &ProjectDirs) -> PathBuf {
    if cfg!(target_os = "macos") {
        project_dirs.preference_dir().to_path_buf()
    } else {
        project_dirs.config_dir().to_path_buf()
    }
}

fn default_data_dir(project_dirs: &ProjectDirs) -> PathBuf {
    if cfg!(target_os = "macos") {
        project_dirs.data_local_dir().to_path_buf()
    } else {
        project_dirs.data_dir().to_path_buf()
    }
}

pub fn validate_policy(dirs: &AppDirs) -> Result<(), AppError> {
    ensure_not_descendant(&dirs.data, &dirs.config, "data", "--data-dir")?;
    ensure_not_descendant(&dirs.cache, &dirs.config, "cache", "--cache-dir")?;
    Ok(())
}

fn ensure_not_descendant(
    candidate: &Path,
    config_root: &Path,
    label: &str,
    flag: &str,
) -> Result<(), AppError> {
    if candidate.starts_with(config_root) {
        return Err(AppError::config(format!(
            "resolved {label} path {} is inside config path {}; use {flag} to move it outside config",
            candidate.display(),
            config_root.display()
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_data_inside_config() {
        let dirs = AppDirs {
            config: PathBuf::from("/tmp/logit/config"),
            data: PathBuf::from("/tmp/logit/config/secrets"),
            cache: PathBuf::from("/tmp/logit/cache"),
        };

        let error = validate_policy(&dirs).expect_err("policy should reject nested data");
        assert!(error.to_string().contains("--data-dir"));
    }

    #[test]
    fn flag_overrides_env_and_default() {
        let overrides = PathOverrides {
            config_dir: Some(PathBuf::from("/tmp/flag-config")),
            data_dir: Some(PathBuf::from("/tmp/flag-data")),
            cache_dir: Some(PathBuf::from("/tmp/flag-cache")),
        };

        let dirs = resolve_dirs(&overrides).expect("dirs resolve");

        assert_eq!(dirs.config, PathBuf::from("/tmp/flag-config"));
        assert_eq!(dirs.data, PathBuf::from("/tmp/flag-data"));
        assert_eq!(dirs.cache, PathBuf::from("/tmp/flag-cache"));
    }

    #[test]
    fn distinct_default_dirs_do_not_violate_policy() {
        let dirs = default_dirs().expect("default dirs resolve");

        validate_policy(&dirs).expect("default policy is valid");
    }
}
