#[test]
fn cache_inside_config_is_rejected() {
    let dirs = logit::paths::AppDirs {
        config: std::path::PathBuf::from("/tmp/logit/config"),
        data: std::path::PathBuf::from("/tmp/logit/data"),
        cache: std::path::PathBuf::from("/tmp/logit/config/cache"),
    };

    let error = logit::paths::validate_policy(&dirs).expect_err("nested cache should fail");

    assert!(error.to_string().contains("--cache-dir"));
}
