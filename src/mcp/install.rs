use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use directories::BaseDirs;
use serde_json::{Map, Value, json};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue, value};

use crate::atomic::atomic_write;
use crate::domain::{McpInstallInput, McpInstallTarget, ProfileSource};
use crate::error::AppError;

const SERVER_NAME: &str = "logit";
const OPENCODE_SCHEMA_URL: &str = "https://opencode.ai/config.json";

pub fn install_target(input: McpInstallInput) -> Result<String, AppError> {
    let context = InstallContext::from_env()?;
    let spec = build_install_spec(&input, &context)?;

    match input.target {
        McpInstallTarget::Claude => install_claude(&context, &spec, run_claude_command),
        McpInstallTarget::Codex => install_codex(&context, &spec),
        McpInstallTarget::OpenCode => install_opencode(&context, &spec),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallContext {
    current_exe: PathBuf,
    current_dir: PathBuf,
    home_dir: PathBuf,
    codex_home: Option<PathBuf>,
    opencode_config: Option<PathBuf>,
}

impl InstallContext {
    fn from_env() -> Result<Self, AppError> {
        let base_dirs =
            BaseDirs::new().ok_or_else(|| AppError::config("unable to resolve home directory"))?;

        Ok(Self {
            current_exe: std::env::current_exe().map_err(|error| {
                AppError::config(format!("resolve current executable: {error}"))
            })?,
            current_dir: std::env::current_dir()
                .map_err(|error| AppError::config(format!("resolve current directory: {error}")))?,
            home_dir: base_dirs.home_dir().to_path_buf(),
            codex_home: std::env::var_os("CODEX_HOME").map(PathBuf::from),
            opencode_config: std::env::var_os("OPENCODE_CONFIG").map(PathBuf::from),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallSpec {
    command: PathBuf,
    args: Vec<String>,
    config_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallOutcome {
    headline: String,
    warnings: Vec<String>,
}

impl InstallOutcome {
    fn render(self) -> String {
        let mut lines = vec![self.headline];
        for warning in self.warnings {
            lines.push(format!("Warning: {warning}"));
        }
        lines.join("\n")
    }
}

fn build_install_spec(
    input: &McpInstallInput,
    context: &InstallContext,
) -> Result<InstallSpec, AppError> {
    let dirs = crate::paths::resolve_dirs(&input.paths)?;
    let mut args = vec![
        String::from("--config-dir"),
        dirs.config.display().to_string(),
        String::from("--data-dir"),
        dirs.data.display().to_string(),
        String::from("--cache-dir"),
        dirs.cache.display().to_string(),
    ];

    if !matches!(input.profile_source, ProfileSource::Default) {
        args.push(String::from("--profile"));
        args.push(input.profile.clone());
    }

    if input.enable_write_tools {
        args.push(String::from("--enable-write-tools"));
    }

    Ok(InstallSpec {
        command: resolve_logit_mcp_path(&context.current_exe)?,
        args,
        config_exists: dirs.config_file().exists(),
    })
}

fn resolve_logit_mcp_path(current_exe: &Path) -> Result<PathBuf, AppError> {
    let parent = current_exe.parent().ok_or_else(|| {
        AppError::config(format!(
            "unable to resolve sibling logit-mcp for {}",
            current_exe.display()
        ))
    })?;
    let binary_name = if cfg!(windows) {
        "logit-mcp.exe"
    } else {
        "logit-mcp"
    };
    let candidate = parent.join(binary_name);
    if !candidate.exists() {
        return Err(AppError::config(format!(
            "missing {} next to {}; build or install `logit-mcp` first",
            binary_name,
            current_exe.display()
        )));
    }

    Ok(candidate.canonicalize().unwrap_or(candidate))
}

fn discover_project_root(start: &Path) -> PathBuf {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut current = Some(start.as_path());
    while let Some(candidate) = current {
        if candidate.join(".git").exists() {
            return candidate.to_path_buf();
        }
        current = candidate.parent();
    }
    start
}

fn install_claude<F>(
    context: &InstallContext,
    spec: &InstallSpec,
    mut runner: F,
) -> Result<String, AppError>
where
    F: FnMut(&Path, &[String]) -> Result<(), AppError>,
{
    let project_root = discover_project_root(&context.current_dir);
    let config_path = context.home_dir.join(".claude.json");
    let expected_entry = render_claude_entry(spec);

    if let Some(existing) = read_claude_local_entry(&config_path, &project_root)? {
        if claude_entry_matches(&existing, &expected_entry) {
            return Ok(build_outcome(
                format!(
                    "Claude already has matching local MCP config for `{SERVER_NAME}` in {}",
                    config_path.display()
                ),
                spec,
                Vec::new(),
            )
            .render());
        }

        if claude_entry_can_be_updated(&existing, &expected_entry) {
            let remove_args = render_claude_remove_args();
            runner(&project_root, &remove_args)?;

            let add_args = render_claude_cli_args(spec);
            runner(&project_root, &add_args)?;
            let warnings = claude_shadow_warnings(&config_path, &project_root)?;

            return Ok(build_outcome(
                format!(
                    "Updated `{SERVER_NAME}` MCP server for Claude in local scope for {}",
                    project_root.display()
                ),
                spec,
                warnings,
            )
            .render());
        }

        return Err(AppError::config(format!(
            "Claude already has a different local MCP server named `{SERVER_NAME}` for {}; remove it first",
            project_root.display()
        )));
    }

    let args = render_claude_cli_args(spec);
    runner(&project_root, &args)?;
    let warnings = claude_shadow_warnings(&config_path, &project_root)?;

    Ok(build_outcome(
        format!(
            "Installed `{SERVER_NAME}` MCP server for Claude in local scope for {}",
            project_root.display()
        ),
        spec,
        warnings,
    )
    .render())
}

fn render_claude_entry(spec: &InstallSpec) -> Value {
    json!({
        "command": spec.command.display().to_string(),
        "args": spec.args,
    })
}

fn read_claude_local_entry(
    config_path: &Path,
    project_root: &Path,
) -> Result<Option<Value>, AppError> {
    if !config_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(config_path)
        .map_err(|error| AppError::config(format!("read {}: {error}", config_path.display())))?;
    let root: Value = serde_json::from_str(&raw)
        .map_err(|error| AppError::config(format!("parse {}: {error}", config_path.display())))?;
    Ok(root
        .get("projects")
        .and_then(Value::as_object)
        .and_then(|projects| projects.get(&project_root.display().to_string()))
        .and_then(|project| project.get("mcpServers"))
        .and_then(Value::as_object)
        .and_then(|servers| servers.get(SERVER_NAME))
        .cloned())
}

fn claude_entry_matches(existing: &Value, expected: &Value) -> bool {
    let Some(existing_object) = existing.as_object() else {
        return false;
    };
    let Some(expected_object) = expected.as_object() else {
        return false;
    };
    existing_object.get("command") == expected_object.get("command")
        && existing_object.get("args") == expected_object.get("args")
}

fn claude_entry_can_be_updated(existing: &Value, expected: &Value) -> bool {
    let Some(existing_object) = existing.as_object() else {
        return false;
    };
    let Some(expected_object) = expected.as_object() else {
        return false;
    };

    existing_object.get("command") == expected_object.get("command")
}

fn claude_shadow_warnings(
    config_path: &Path,
    project_root: &Path,
) -> Result<Vec<String>, AppError> {
    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let raw = fs::read_to_string(config_path)
        .map_err(|error| AppError::config(format!("read {}: {error}", config_path.display())))?;
    let root: Value = serde_json::from_str(&raw)
        .map_err(|error| AppError::config(format!("parse {}: {error}", config_path.display())))?;
    let mut warnings = Vec::new();

    if root
        .get("mcpServers")
        .and_then(Value::as_object)
        .and_then(|servers| servers.get(SERVER_NAME))
        .is_some()
    {
        warnings.push(format!(
            "{} already defines a user-scope `{SERVER_NAME}` server; the new local install will take precedence in {}",
            config_path.display(),
            project_root.display()
        ));
    }

    let project_config = project_root.join(".mcp.json");
    if project_config.exists() && read_claude_project_entry(&project_config)?.is_some() {
        warnings.push(format!(
            "{} already defines a project-scope `{SERVER_NAME}` server; the new local install will take precedence",
            project_config.display()
        ));
    }

    Ok(warnings)
}

fn read_claude_project_entry(config_path: &Path) -> Result<Option<Value>, AppError> {
    let raw = fs::read_to_string(config_path)
        .map_err(|error| AppError::config(format!("read {}: {error}", config_path.display())))?;
    let root: Value = serde_json::from_str(&raw)
        .map_err(|error| AppError::config(format!("parse {}: {error}", config_path.display())))?;
    Ok(root
        .get("mcpServers")
        .and_then(Value::as_object)
        .and_then(|servers| servers.get(SERVER_NAME))
        .cloned())
}

fn render_claude_cli_args(spec: &InstallSpec) -> Vec<String> {
    let mut args = vec![
        String::from("mcp"),
        String::from("add"),
        String::from("--scope"),
        String::from("local"),
        String::from("--transport"),
        String::from("stdio"),
        String::from(SERVER_NAME),
        String::from("--"),
        spec.command.display().to_string(),
    ];
    args.extend(spec.args.clone());
    args
}

fn render_claude_remove_args() -> Vec<String> {
    vec![
        String::from("mcp"),
        String::from("remove"),
        String::from("--scope"),
        String::from("local"),
        String::from(SERVER_NAME),
    ]
}

fn run_claude_command(project_root: &Path, args: &[String]) -> Result<(), AppError> {
    let status = Command::new("claude")
        .args(args)
        .current_dir(project_root)
        .status()
        .map_err(|error| {
            AppError::config(format!(
                "launch `claude`: {error}; run manually from {} with `{}`",
                project_root.display(),
                shell_command("claude", args)
            ))
        })?;
    if !status.success() {
        return Err(AppError::config(format!(
            "`claude mcp add` failed with status {status}; run manually from {} with `{}`",
            project_root.display(),
            shell_command("claude", args)
        )));
    }
    Ok(())
}

fn install_codex(context: &InstallContext, spec: &InstallSpec) -> Result<String, AppError> {
    let project_root = discover_project_root(&context.current_dir);
    let config_path = codex_config_path(context);
    let mut document = read_toml_document(&config_path)?;

    if let Some(existing) = codex_entry(&document) {
        if codex_entry_matches(existing, spec) {
            return Ok(build_outcome(
                format!(
                    "Codex already has matching MCP config for `{SERVER_NAME}` in {}",
                    config_path.display()
                ),
                spec,
                codex_override_warnings(&project_root),
            )
            .render());
        }

        return Err(AppError::config(format!(
            "Codex already has a different MCP server named `{SERVER_NAME}` in {}; remove or edit that table first",
            config_path.display()
        )));
    }

    insert_codex_entry(&mut document, spec)?;
    let serialized = document.to_string();
    atomic_write(&config_path, serialized.as_bytes())?;

    Ok(build_outcome(
        format!(
            "Installed `{SERVER_NAME}` MCP server for Codex in {}",
            config_path.display()
        ),
        spec,
        codex_override_warnings(&project_root),
    )
    .render())
}

fn codex_config_path(context: &InstallContext) -> PathBuf {
    context
        .codex_home
        .clone()
        .unwrap_or_else(|| context.home_dir.join(".codex"))
        .join("config.toml")
}

fn read_toml_document(path: &Path) -> Result<DocumentMut, AppError> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }

    let raw = fs::read_to_string(path)
        .map_err(|error| AppError::config(format!("read {}: {error}", path.display())))?;
    raw.parse::<DocumentMut>()
        .map_err(|error| AppError::config(format!("parse {}: {error}", path.display())))
}

fn codex_entry(document: &DocumentMut) -> Option<&Item> {
    document
        .get("mcp_servers")
        .and_then(Item::as_table_like)
        .and_then(|servers| servers.get(SERVER_NAME))
}

fn codex_entry_matches(existing: &Item, spec: &InstallSpec) -> bool {
    let Some(table) = existing.as_table_like() else {
        return false;
    };
    let command_matches = table
        .get("command")
        .and_then(Item::as_str)
        .map(|command| command == spec.command.display().to_string())
        .unwrap_or(false);
    let args_matches = table
        .get("args")
        .and_then(Item::as_array)
        .map(|args| {
            args.iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
                == spec.args
        })
        .unwrap_or(false);

    command_matches && args_matches
}

fn insert_codex_entry(document: &mut DocumentMut, spec: &InstallSpec) -> Result<(), AppError> {
    if document.get("mcp_servers").is_some() && document["mcp_servers"].as_table_like().is_none() {
        return Err(AppError::config(
            "expected `mcp_servers` to be a TOML table",
        ));
    }

    if document.get("mcp_servers").is_none() {
        document["mcp_servers"] = Item::Table(Table::new());
    }

    let servers = document["mcp_servers"]
        .as_table_like_mut()
        .ok_or_else(|| AppError::config("expected `mcp_servers` to be a TOML table"))?;

    let mut server = Table::new();
    server.insert("command", value(spec.command.display().to_string()));

    let mut args = Array::new();
    for arg in &spec.args {
        args.push(arg.as_str());
    }
    server.insert("args", Item::Value(TomlValue::Array(args)));

    servers.insert(SERVER_NAME, Item::Table(server));
    Ok(())
}

fn codex_override_warnings(project_root: &Path) -> Vec<String> {
    let project_config = project_root.join(".codex").join("config.toml");
    if !project_config.exists() {
        return Vec::new();
    }

    match read_toml_document(&project_config) {
        Ok(document) if codex_entry(&document).is_some() => vec![format!(
            "{} also defines `{SERVER_NAME}` and may override the user-level install",
            project_config.display()
        )],
        _ => Vec::new(),
    }
}

fn install_opencode(context: &InstallContext, spec: &InstallSpec) -> Result<String, AppError> {
    let project_root = discover_project_root(&context.current_dir);
    let config_path = opencode_config_path(context);
    let mut root = read_opencode_config(&config_path, Some(spec))?;

    let existing = root
        .get("mcp")
        .and_then(Value::as_object)
        .and_then(|mcp| mcp.get(SERVER_NAME));
    let expected_entry = render_opencode_entry(spec);

    if let Some(existing) = existing {
        if existing == &expected_entry {
            return Ok(build_outcome(
                format!(
                    "OpenCode already has matching MCP config for `{SERVER_NAME}` in {}",
                    config_path.display()
                ),
                spec,
                opencode_override_warnings(&project_root),
            )
            .render());
        }

        return Err(AppError::config(format!(
            "OpenCode already has a different MCP server named `{SERVER_NAME}` in {}; merge this manually:\n{}",
            config_path.display(),
            render_opencode_snippet(spec)
        )));
    }

    let root_object = root.as_object_mut().ok_or_else(|| {
        AppError::config(format!(
            "expected {} to contain a JSON object",
            config_path.display()
        ))
    })?;
    if !root_object.contains_key("$schema") {
        root_object.insert(
            String::from("$schema"),
            Value::String(String::from(OPENCODE_SCHEMA_URL)),
        );
    }

    let mcp = root_object
        .entry(String::from("mcp"))
        .or_insert_with(|| Value::Object(Map::new()));
    let mcp_object = mcp.as_object_mut().ok_or_else(|| {
        AppError::config(format!(
            "expected `mcp` in {} to be a JSON object",
            config_path.display()
        ))
    })?;
    mcp_object.insert(String::from(SERVER_NAME), expected_entry);

    let serialized = serde_json::to_string_pretty(&root).map_err(|error| {
        AppError::config(format!("serialize {}: {error}", config_path.display()))
    })?;
    atomic_write(&config_path, format!("{serialized}\n").as_bytes())?;

    Ok(build_outcome(
        format!(
            "Installed `{SERVER_NAME}` MCP server for OpenCode in {}",
            config_path.display()
        ),
        spec,
        opencode_override_warnings(&project_root),
    )
    .render())
}

fn opencode_config_path(context: &InstallContext) -> PathBuf {
    context.opencode_config.clone().unwrap_or_else(|| {
        context
            .home_dir
            .join(".config")
            .join("opencode")
            .join("opencode.json")
    })
}

fn read_opencode_config(path: &Path, spec: Option<&InstallSpec>) -> Result<Value, AppError> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| AppError::config(format!("read {}: {error}", path.display())))?;
    let snippet = spec
        .map(render_opencode_snippet)
        .unwrap_or_else(render_opencode_default_snippet);
    serde_json::from_str::<Value>(&raw).map_err(|error| {
        AppError::config(format!(
            "parse {}: {error}; OpenCode auto-install only supports strict JSON files. Merge this manually:\n{}",
            path.display(),
            snippet
        ))
    })
}

fn render_opencode_entry(spec: &InstallSpec) -> Value {
    let mut command = vec![Value::String(spec.command.display().to_string())];
    command.extend(spec.args.iter().cloned().map(Value::String));
    json!({
        "type": "local",
        "command": command,
        "enabled": true,
    })
}

fn render_opencode_snippet(spec: &InstallSpec) -> String {
    render_opencode_snippet_from_args(&render_opencode_entry(spec))
}

fn render_opencode_default_snippet() -> String {
    String::from(
        "{\n  \"mcp\": {\n    \"logit\": {\n      \"type\": \"local\",\n      \"command\": [\"/absolute/path/to/logit-mcp\"]\n    }\n  }\n}",
    )
}

fn render_opencode_snippet_from_args(entry: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(&json!({
        "mcp": {
            SERVER_NAME: entry,
        }
    }))
    .unwrap_or_else(|_| String::from("{\n  \"mcp\": {\n    \"logit\": {}\n  }\n}"))
}

fn opencode_override_warnings(project_root: &Path) -> Vec<String> {
    let project_config = project_root.join("opencode.json");
    if !project_config.exists() {
        return Vec::new();
    }

    match read_opencode_config(&project_config, None) {
        Ok(root)
            if root
                .get("mcp")
                .and_then(Value::as_object)
                .and_then(|mcp| mcp.get(SERVER_NAME))
                .is_some() =>
        {
            vec![format!(
                "{} also defines `{SERVER_NAME}` and may override the global install",
                project_config.display()
            )]
        }
        _ => Vec::new(),
    }
}

fn build_outcome(
    headline: String,
    spec: &InstallSpec,
    mut warnings: Vec<String>,
) -> InstallOutcome {
    if !spec.config_exists {
        warnings.push(String::from(
            "run `logit setup` before using the installed MCP server",
        ));
    }

    InstallOutcome { headline, warnings }
}

fn shell_command(program: &str, args: &[String]) -> String {
    let mut parts = vec![shell_quote(program)];
    parts.extend(args.iter().map(|arg| shell_quote(arg)));
    parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':'))
    {
        return value.to_owned();
    }

    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{McpInstallInput, McpInstallTarget, PathOverrides};

    fn temp_context() -> (tempfile::TempDir, InstallContext) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        let bin_dir = root.join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        fs::write(bin_dir.join("logit"), b"").expect("logit binary");
        fs::write(bin_dir.join("logit-mcp"), b"").expect("logit-mcp binary");

        (
            temp,
            InstallContext {
                current_exe: bin_dir.join("logit"),
                current_dir: root.join("project"),
                home_dir: root.join("home"),
                codex_home: None,
                opencode_config: None,
            },
        )
    }

    fn install_input(target: McpInstallTarget) -> McpInstallInput {
        McpInstallInput {
            target,
            profile: String::from("default"),
            profile_source: ProfileSource::Default,
            enable_write_tools: false,
            paths: PathOverrides::default(),
        }
    }

    #[test]
    fn discovers_git_project_root_from_nested_directory() {
        let (_temp, mut context) = temp_context();
        let root = context.current_dir.parent().expect("parent").to_path_buf();
        let project_root = root.join("project");
        let nested = project_root.join("src").join("subdir");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::create_dir_all(project_root.join(".git")).expect("git dir");
        let expected_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());

        context.current_dir = nested;

        assert_eq!(discover_project_root(&context.current_dir), expected_root);
    }

    #[test]
    fn discovers_git_file_project_root() {
        let (_temp, mut context) = temp_context();
        let root = context.current_dir.parent().expect("parent").to_path_buf();
        let project_root = root.join("project");
        let nested = project_root.join("packages").join("api");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::write(
            project_root.join(".git"),
            b"gitdir: ../.git/worktrees/project\n",
        )
        .expect("git file");
        let expected_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());

        context.current_dir = nested;

        assert_eq!(discover_project_root(&context.current_dir), expected_root);
    }

    #[test]
    fn resolves_sibling_logit_mcp_binary() {
        let (_temp, context) = temp_context();

        let resolved = resolve_logit_mcp_path(&context.current_exe).expect("mcp path resolves");

        assert!(resolved.ends_with("logit-mcp"));
    }

    #[test]
    fn builds_args_without_implicit_default_profile() {
        let (_temp, context) = temp_context();
        let input = install_input(McpInstallTarget::Claude);

        let spec = build_install_spec(&input, &context).expect("spec builds");

        assert!(!spec.args.iter().any(|arg| arg == "--profile"));
    }

    #[test]
    fn builds_args_with_explicit_profile() {
        let (_temp, context) = temp_context();
        let mut input = install_input(McpInstallTarget::Claude);
        input.profile = String::from("work");
        input.profile_source = ProfileSource::Flag;

        let spec = build_install_spec(&input, &context).expect("spec builds");

        assert!(
            spec.args
                .windows(2)
                .any(|window| window == ["--profile", "work"])
        );
    }

    #[test]
    fn builds_args_with_enable_write_tools() {
        let (_temp, context) = temp_context();
        let mut input = install_input(McpInstallTarget::Claude);
        input.enable_write_tools = true;

        let spec = build_install_spec(&input, &context).expect("spec builds");

        assert!(spec.args.iter().any(|arg| arg == "--enable-write-tools"));
    }

    #[test]
    fn renders_claude_cli_args() {
        let (_temp, context) = temp_context();
        let spec = build_install_spec(&install_input(McpInstallTarget::Claude), &context)
            .expect("spec builds");

        let args = render_claude_cli_args(&spec);

        assert_eq!(args[0], "mcp");
        assert_eq!(args[1], "add");
        assert_eq!(args[6], SERVER_NAME);
        assert_eq!(args[7], "--");
    }

    #[test]
    fn claude_install_uses_project_root_as_cwd() {
        let (_temp, mut context) = temp_context();
        let project_root = context.current_dir.clone();
        let nested = project_root.join("crates").join("logit");
        fs::create_dir_all(&nested).expect("nested dir");
        fs::create_dir_all(project_root.join(".git")).expect("git dir");
        let expected_root = project_root
            .canonicalize()
            .unwrap_or_else(|_| project_root.clone());
        context.current_dir = nested;
        let spec = build_install_spec(&install_input(McpInstallTarget::Claude), &context)
            .expect("spec builds");
        let mut observed = None;

        let result = install_claude(&context, &spec, |cwd, args| {
            observed = Some((cwd.to_path_buf(), args.to_vec()));
            Ok(())
        })
        .expect("install succeeds");

        let (cwd, args) = observed.expect("command captured");
        assert_eq!(cwd, expected_root);
        assert!(args.contains(&String::from("--scope")));
        assert!(result.contains("Installed `logit` MCP server for Claude"));
    }

    #[test]
    fn claude_install_warns_when_user_scope_will_be_shadowed() {
        let (_temp, context) = temp_context();
        let project_root = context.current_dir.clone();
        fs::create_dir_all(&project_root).expect("project dir");
        fs::create_dir_all(project_root.join(".git")).expect("git dir");
        fs::create_dir_all(&context.home_dir).expect("home dir");
        fs::write(
            context.home_dir.join(".claude.json"),
            format!(
                r#"{{
  "mcpServers": {{
    "{SERVER_NAME}": {{
      "command": "/tmp/other-logit-mcp"
    }}
  }}
}}"#
            ),
        )
        .expect("claude config");

        let spec = build_install_spec(&install_input(McpInstallTarget::Claude), &context)
            .expect("spec builds");

        let result =
            install_claude(&context, &spec, |_cwd, _args| Ok(())).expect("install succeeds");

        assert!(result.contains("Warning:"));
        assert!(result.contains("user-scope"));
    }

    #[test]
    fn claude_install_updates_existing_matching_command_entry() {
        let (_temp, context) = temp_context();
        let project_root = context.current_dir.clone();
        fs::create_dir_all(&project_root).expect("project dir");
        fs::create_dir_all(project_root.join(".git")).expect("git dir");
        fs::create_dir_all(&context.home_dir).expect("home dir");
        let resolved_project_root = discover_project_root(&context.current_dir);
        let existing_command = context
            .current_exe
            .parent()
            .expect("bin dir")
            .join("logit-mcp")
            .canonicalize()
            .expect("canonical logit-mcp");
        fs::write(
            context.home_dir.join(".claude.json"),
            format!(
                r#"{{
  "projects": {{
    "{}": {{
      "mcpServers": {{
        "{SERVER_NAME}": {{
          "command": "{}",
          "args": ["--config-dir", "/tmp/config"]
        }}
      }}
    }}
  }}
}}"#,
                resolved_project_root.display(),
                existing_command.display()
            ),
        )
        .expect("claude config");

        let mut input = install_input(McpInstallTarget::Claude);
        input.enable_write_tools = true;
        let spec = build_install_spec(&input, &context).expect("spec builds");
        let mut calls = Vec::new();

        let result = install_claude(&context, &spec, |cwd, args| {
            calls.push((cwd.to_path_buf(), args.to_vec()));
            Ok(())
        })
        .expect("install succeeds");

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, resolved_project_root);
        assert_eq!(calls[0].1, render_claude_remove_args());
        assert_eq!(calls[1].1, render_claude_cli_args(&spec));
        assert!(result.contains("Updated `logit` MCP server for Claude"));
    }

    #[test]
    fn claude_install_rejects_existing_different_command_entry() {
        let (_temp, context) = temp_context();
        let project_root = context.current_dir.clone();
        fs::create_dir_all(&project_root).expect("project dir");
        fs::create_dir_all(project_root.join(".git")).expect("git dir");
        fs::create_dir_all(&context.home_dir).expect("home dir");
        let resolved_project_root = discover_project_root(&context.current_dir);
        fs::write(
            context.home_dir.join(".claude.json"),
            format!(
                r#"{{
  "projects": {{
    "{}": {{
      "mcpServers": {{
        "{SERVER_NAME}": {{
          "command": "/tmp/other-server",
          "args": []
        }}
      }}
    }}
  }}
}}"#,
                resolved_project_root.display()
            ),
        )
        .expect("claude config");

        let mut input = install_input(McpInstallTarget::Claude);
        input.enable_write_tools = true;
        let spec = build_install_spec(&input, &context).expect("spec builds");

        let error = install_claude(&context, &spec, |_cwd, _args| Ok(())).expect_err("rejects");

        assert!(
            error
                .to_string()
                .contains("different local MCP server named `logit`")
        );
    }

    #[test]
    fn codex_install_creates_server_table() {
        let (_temp, mut context) = temp_context();
        let root = context.current_dir.parent().expect("parent").to_path_buf();
        fs::create_dir_all(&context.current_dir).expect("project dir");
        context.codex_home = Some(root.join("codex-home"));
        let spec = build_install_spec(&install_input(McpInstallTarget::Codex), &context)
            .expect("spec builds");

        let result = install_codex(&context, &spec).expect("codex install succeeds");
        let config_path = codex_config_path(&context);
        let saved = fs::read_to_string(config_path).expect("saved config");

        assert!(saved.contains("[mcp_servers.logit]"));
        assert!(saved.contains("command ="));
        assert!(result.contains("Installed `logit` MCP server for Codex"));
    }

    #[test]
    fn codex_install_is_idempotent_when_matching() {
        let (_temp, mut context) = temp_context();
        let root = context.current_dir.parent().expect("parent").to_path_buf();
        fs::create_dir_all(&context.current_dir).expect("project dir");
        context.codex_home = Some(root.join("codex-home"));
        let spec = build_install_spec(&install_input(McpInstallTarget::Codex), &context)
            .expect("spec builds");

        install_codex(&context, &spec).expect("first install");
        let result = install_codex(&context, &spec).expect("second install");

        assert!(result.contains("already has matching MCP config"));
    }

    #[test]
    fn opencode_install_creates_global_config() {
        let (_temp, mut context) = temp_context();
        fs::create_dir_all(&context.current_dir).expect("project dir");
        context.opencode_config = Some(context.home_dir.join("custom-opencode.json"));
        let spec = build_install_spec(&install_input(McpInstallTarget::OpenCode), &context)
            .expect("spec builds");

        let result = install_opencode(&context, &spec).expect("opencode install succeeds");
        let saved = fs::read_to_string(opencode_config_path(&context)).expect("saved config");

        assert!(saved.contains("\"mcp\""));
        assert!(saved.contains("\"logit\""));
        assert!(saved.contains("\"type\": \"local\""));
        assert!(result.contains("Installed `logit` MCP server for OpenCode"));
    }

    #[test]
    fn opencode_install_rejects_non_json_config() {
        let (_temp, context) = temp_context();
        fs::create_dir_all(&context.current_dir).expect("project dir");
        let config_path = context
            .home_dir
            .join(".config")
            .join("opencode")
            .join("opencode.json");
        fs::create_dir_all(config_path.parent().expect("parent")).expect("config dir");
        fs::write(&config_path, b"{\n  // comment\n}\n").expect("jsonc fixture");
        let spec = build_install_spec(&install_input(McpInstallTarget::OpenCode), &context)
            .expect("spec builds");

        let error = install_opencode(&context, &spec).expect_err("jsonc rejected");

        assert!(error.to_string().contains("strict JSON files"));
        assert!(error.to_string().contains("logit-mcp"));
        assert!(error.to_string().contains("--config-dir"));
    }
}
