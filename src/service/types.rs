use serde::Serialize;

use chrono::NaiveDate;

use crate::domain::{
    Alias as DomainAlias, PathOverrides, Profile, StatReport, StatSelector, WorklogDraft,
    WorklogResult,
};
use crate::paths::{AppDirs, resolve_dirs};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum ProfileRef {
    Active,
    Named(String),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RequestScope {
    pub profile: ProfileRef,
    pub paths: PathOverrides,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServiceWarning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServiceMeta {
    pub profile_used: Option<String>,
    pub degraded: bool,
    pub warnings: Vec<ServiceWarning>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServiceOutput<T> {
    pub meta: ServiceMeta,
    pub data: T,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServiceError {
    pub code: &'static str,
    pub category: &'static str,
    pub message: String,
    pub remediation: Option<String>,
    pub retryable: bool,
}

impl From<crate::error::AppError> for ServiceError {
    fn from(value: crate::error::AppError) -> Self {
        let (code, remediation, retryable) = match value.category {
            "auth" => (
                "auth_failed",
                Some(String::from(
                    "Check your Jira or Tempo credentials and run `logit setup` if needed.",
                )),
                false,
            ),
            "validation" => ("invalid_input", None, false),
            "config" => (
                "config_error",
                Some(String::from(
                    "Inspect `logit doctor` and `logit config path` to fix local configuration.",
                )),
                false,
            ),
            "network" => (
                "network_error",
                Some(String::from(
                    "Retry the request. If it keeps failing, check network access and upstream service status.",
                )),
                true,
            ),
            "not_found" => ("not_found", None, false),
            "conflict" => ("conflict", None, false),
            _ => ("error", None, false),
        };

        Self {
            code,
            category: value.category,
            message: value.message,
            remediation,
            retryable,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum StatsCacheMode {
    UseCache,
    NoWrite,
    Refresh,
    NoCalendar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedScope {
    pub dirs: AppDirs,
    pub profile_name: String,
    pub profile: Profile,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GetStatsRequest {
    pub scope: RequestScope,
    pub selector: StatSelector,
    pub details: bool,
    pub cache_mode: StatsCacheMode,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GetStatsResult {
    pub selector: StatSelector,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub details: bool,
    pub report: StatReport,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AliasInfo {
    pub name: String,
    pub issue_key: String,
    pub default_duration: Option<u32>,
    pub default_message: Option<String>,
}

impl AliasInfo {
    pub fn from_domain(name: String, alias: DomainAlias) -> Self {
        Self {
            name,
            issue_key: alias.key,
            default_duration: alias.default_duration,
            default_message: alias.default_message,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListAliasesRequest {
    pub scope: RequestScope,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListAliasesResult {
    pub aliases: Vec<AliasInfo>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConfigPathRequest {
    pub scope: RequestScope,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConfigPathResult {
    pub config_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ClearCacheRequest {
    pub scope: RequestScope,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ClearCacheResult {
    pub profile_used: String,
    pub path: String,
    pub existed: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PreviewLogRequest {
    pub scope: RequestScope,
    pub issue_or_alias: String,
    pub duration_seconds: Option<u32>,
    pub date: Option<NaiveDate>,
    pub message: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PreviewLogResult {
    pub draft: WorklogDraft,
    pub issue_key: String,
    pub alias_used: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogTimeRequest {
    pub scope: RequestScope,
    pub issue_or_alias: String,
    pub duration_seconds: Option<u32>,
    pub date: Option<NaiveDate>,
    pub message: Option<String>,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogTimeResult {
    pub worklog: WorklogResult,
}

pub fn resolve_scope(scope: &RequestScope) -> Result<ResolvedScope, ServiceError> {
    let dirs = resolve_dirs(&scope.paths).map_err(ServiceError::from)?;
    let config = crate::config::load_config(&dirs).map_err(ServiceError::from)?;

    let (profile_name, profile) = match &scope.profile {
        ProfileRef::Active => {
            let name = config.active.clone();
            let profile = config
                .profiles
                .get(&name)
                .cloned()
                .ok_or_else(|| ServiceError {
                    code: "unknown_profile",
                    category: "config",
                    message: format!("unknown active profile: {name}"),
                    remediation: Some(String::from(
                        "Run `logit setup` or fix the active profile in your config.",
                    )),
                    retryable: false,
                })?;
            (name, profile)
        }
        ProfileRef::Named(name) => {
            let profile = config
                .profiles
                .get(name)
                .cloned()
                .ok_or_else(|| ServiceError {
                    code: "unknown_profile",
                    category: "config",
                    message: format!("unknown profile: {name}"),
                    remediation: Some(String::from("Choose an existing profile name or omit the profile to use the active one.")),
                    retryable: false,
                })?;
            (name.clone(), profile)
        }
    };

    Ok(ResolvedScope {
        dirs,
        profile_name,
        profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{default_config, save_config};

    #[test]
    fn named_profile_requires_exact_match() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dirs = crate::paths::AppDirs {
            config: temp.path().join("config"),
            data: temp.path().join("data"),
            cache: temp.path().join("cache"),
        };
        let mut config = default_config("UTC");
        config.profiles.insert(
            String::from("work"),
            crate::config::default_profile("Europe/Berlin"),
        );
        config.active = String::from("work");
        save_config(&dirs, &config).expect("save config");

        let resolved = resolve_scope(&RequestScope {
            profile: ProfileRef::Named(String::from("default")),
            paths: crate::domain::PathOverrides {
                config_dir: Some(dirs.config),
                data_dir: Some(dirs.data),
                cache_dir: Some(dirs.cache),
            },
        })
        .expect("resolve scope");

        assert_eq!(resolved.profile_name, "default");
        assert_eq!(resolved.profile.tz, "UTC");
    }
}
