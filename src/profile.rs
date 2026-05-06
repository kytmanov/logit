use crate::domain::{Config, Profile};
use crate::error::AppError;

pub fn resolve_profile<'a>(config: &'a Config, requested: &str) -> Result<&'a Profile, AppError> {
    let name = resolve_profile_name(config, requested)?;
    config
        .profiles
        .get(name)
        .ok_or_else(|| AppError::config(format!("unknown profile: {requested}")))
}

pub fn resolve_profile_name<'a>(
    config: &'a Config,
    requested: &'a str,
) -> Result<&'a str, AppError> {
    config
        .profiles
        .get(requested)
        .map(|_| requested)
        .or_else(|| (requested == "default").then_some(config.active.as_str()))
        .ok_or_else(|| AppError::config(format!("unknown profile: {requested}")))
}

#[cfg(test)]
mod tests {
    use crate::config::default_config;

    use super::*;

    #[test]
    fn resolves_known_profile() {
        let config = default_config("UTC");
        let profile = resolve_profile(&config, "default").expect("profile resolves");

        assert_eq!(profile.tz, "UTC");
    }

    #[test]
    fn rejects_unknown_profile() {
        let config = default_config("UTC");
        let error = resolve_profile(&config, "work").expect_err("unknown profile rejected");

        assert!(error.to_string().contains("unknown profile"));
    }
}
