use crate::service::types::{ConfigPathRequest, ConfigPathResult, ServiceMeta, ServiceOutput};

pub fn config_path(
    request: ConfigPathRequest,
) -> Result<ServiceOutput<ConfigPathResult>, crate::service::types::ServiceError> {
    let dirs = crate::paths::resolve_dirs(&request.scope.paths)
        .map_err(crate::service::types::ServiceError::from)?;

    Ok(ServiceOutput {
        meta: ServiceMeta {
            profile_used: None,
            degraded: false,
            warnings: Vec::new(),
        },
        data: ConfigPathResult {
            config_path: dirs.config_file().display().to_string(),
        },
    })
}
