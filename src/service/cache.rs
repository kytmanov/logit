use crate::config::load_config;
use crate::profile::resolve_profile_name;
use crate::service::types::{
    ClearCacheRequest, ClearCacheResult, ServiceMeta, ServiceOutput, resolve_scope,
};

pub fn clear_cache(
    request: ClearCacheRequest,
) -> Result<ServiceOutput<ClearCacheResult>, crate::service::types::ServiceError> {
    let resolved = resolve_scope(&request.scope)?;
    let config = load_config(&resolved.dirs).map_err(crate::service::types::ServiceError::from)?;
    let profile_name = resolve_profile_name(&config, &resolved.profile_name)
        .map_err(crate::service::types::ServiceError::from)?;
    let path = resolved.dirs.calendar_file(profile_name);
    let existed = path.exists();
    if existed {
        std::fs::remove_file(&path).map_err(|error| {
            crate::service::types::ServiceError::from(crate::error::AppError::config(format!(
                "remove {}: {error}",
                path.display()
            )))
        })?;
    }

    Ok(ServiceOutput {
        meta: ServiceMeta {
            profile_used: Some(profile_name.to_owned()),
            degraded: false,
            warnings: Vec::new(),
        },
        data: ClearCacheResult {
            profile_used: profile_name.to_owned(),
            path: path.display().to_string(),
            existed,
        },
    })
}
