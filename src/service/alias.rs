use crate::service::types::{
    AliasInfo, ListAliasesRequest, ListAliasesResult, ServiceMeta, ServiceOutput, resolve_scope,
};

pub fn list_aliases(
    request: ListAliasesRequest,
) -> Result<ServiceOutput<ListAliasesResult>, crate::service::types::ServiceError> {
    let resolved = resolve_scope(&request.scope)?;
    let aliases = resolved
        .profile
        .aliases
        .into_iter()
        .map(|(name, alias)| AliasInfo::from_domain(name, alias))
        .collect();

    Ok(ServiceOutput {
        meta: ServiceMeta {
            profile_used: Some(resolved.profile_name),
            degraded: false,
            warnings: Vec::new(),
        },
        data: ListAliasesResult { aliases },
    })
}
