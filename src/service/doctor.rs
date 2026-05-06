use std::path::Path;

use serde::Serialize;

use crate::config::SCHEMA_VERSION;
use crate::paths::AppDirs;
use crate::service::types::{RequestScope, ServiceMeta, ServiceOutput, resolve_scope};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorPathInfo {
    pub path: String,
    pub location_kind: &'static str,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorInfo {
    pub config: DoctorPathInfo,
    pub data: DoctorPathInfo,
    pub cache: DoctorPathInfo,
    pub schema_version: Option<u32>,
    pub supported_schema_version: u32,
    pub active_profile: Option<String>,
    pub profile_timezone: Option<String>,
}

pub fn collect_doctor_info(
    scope: &RequestScope,
) -> Result<ServiceOutput<DoctorInfo>, crate::service::types::ServiceError> {
    let dirs = crate::paths::resolve_dirs(&scope.paths)
        .map_err(crate::service::types::ServiceError::from)?;
    let resolved = resolve_scope(scope).ok();
    let schema_version = dirs.config_file().exists().then_some(SCHEMA_VERSION);

    Ok(ServiceOutput {
        meta: ServiceMeta {
            profile_used: resolved.as_ref().map(|value| value.profile_name.clone()),
            degraded: schema_version.is_none(),
            warnings: path_infos(&dirs)
                .iter()
                .filter_map(|(_, info)| {
                    info.warning
                        .as_ref()
                        .map(|warning| crate::service::types::ServiceWarning {
                            code: "path_warning",
                            message: format!("{}: {warning}", info.path),
                        })
                })
                .collect(),
        },
        data: DoctorInfo {
            config: path_info(&dirs.config),
            data: path_info(&dirs.data),
            cache: path_info(&dirs.cache),
            schema_version,
            supported_schema_version: SCHEMA_VERSION,
            active_profile: resolved.as_ref().map(|value| value.profile_name.clone()),
            profile_timezone: resolved.as_ref().map(|value| value.profile.tz.clone()),
        },
    })
}

fn path_infos(dirs: &AppDirs) -> [(&'static str, DoctorPathInfo); 3] {
    [
        ("config", path_info(&dirs.config)),
        ("data", path_info(&dirs.data)),
        ("cache", path_info(&dirs.cache)),
    ]
}

fn path_info(path: &Path) -> DoctorPathInfo {
    let (location_kind, flagged) = detect_path_kind(path);
    DoctorPathInfo {
        path: path.display().to_string(),
        location_kind,
        warning: flagged.then(|| String::from("dotfile-sync detected")),
    }
}

fn detect_path_kind(path: &Path) -> (&'static str, bool) {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate
            .file_name()
            .is_some_and(|name| name == "dotfiles" || name == ".dotfiles" || name == "dotfiles.git")
        {
            return ("dotfile-sync", true);
        }
        if candidate.join(".git").exists() {
            return ("dotfile-sync", true);
        }
        current = candidate.parent();
    }
    ("local-only", false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{default_config, save_config};
    use crate::paths::AppDirs;
    use crate::service::types::ProfileRef;

    #[test]
    fn doctor_info_uses_active_profile_by_default() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dirs = AppDirs {
            config: temp.path().join("config"),
            data: temp.path().join("data"),
            cache: temp.path().join("cache"),
        };
        let mut config = default_config("UTC");
        config.active = String::from("default");
        save_config(&dirs, &config).expect("save config");

        let output = collect_doctor_info(&RequestScope {
            profile: ProfileRef::Active,
            paths: crate::domain::PathOverrides {
                config_dir: Some(dirs.config.clone()),
                data_dir: Some(dirs.data.clone()),
                cache_dir: Some(dirs.cache.clone()),
            },
        })
        .expect("doctor info");

        assert_eq!(output.data.active_profile.as_deref(), Some("default"));
        assert_eq!(output.data.profile_timezone.as_deref(), Some("UTC"));
    }
}
