# logit

Terminal-first Jira Tempo worklog logger with MCP support.

`logit` ships two interfaces that share the same local config, profiles, secrets, aliases, and cache:

- `logit` for direct CLI worklogging and reporting
- `logit-mcp` for Claude, Codex, OpenCode, and other MCP-compatible editors and agents

Use it to:

- log time from the terminal or through an editor/chat agent
- preview worklogs before writing them
- inspect aliases, config paths, and local health through MCP
- fetch worklog stats for today, a date, week, last week, month, or year
- keep MCP write access opt-in instead of always-on

It also ships the compatibility binary name `cli-tempo`.

## Installation

### Option 1: build from source

Requirements:

- Rust stable toolchain

Build:

```bash
git clone https://github.com/kytmanov/logit.git
cd logit
cargo build --release
```

The binary will be at:

- `target/release/logit`
- `target/release/cli-tempo`
- `target/release/logit-mcp`

Optional install into your cargo bin dir:

```bash
cargo install --path .
```

### Option 2: copy the release binary manually

After `cargo build --release`, copy `target/release/logit` to any directory in your `PATH`.

If you want the old command name too:

```bash
cp target/release/logit ~/.local/bin/logit
cp target/release/cli-tempo ~/.local/bin/cli-tempo
cp target/release/logit-mcp ~/.local/bin/logit-mcp
```

## First run

Run setup:

```bash
logit setup
```

Setup stores:

- config in the config directory
- secrets in the data directory
- cache in the cache directory

Check where those are on your machine:

```bash
logit doctor
logit config path
```

If you plan to use MCP, run `logit setup` first. `logit-mcp` reuses the same config, secrets, profiles, aliases, and cache as the CLI.

## Required credentials

You need:

- Tempo API token
- Jira base URL, for example `https://your-company.atlassian.net`
- Jira account email
- Jira API token

`logit` uses:

- Tempo token for Tempo API calls
- Jira email + Jira token for Jira account and issue lookup calls

## MCP

`logit-mcp` exposes `logit` over stdio for editor and agent workflows. It is designed for cases where you want an MCP client to inspect local setup, read worklog data, preview time entries, and optionally create worklogs.

### MCP features

Read-only by default:

- `doctor`: inspect resolved paths, schema version, active profile, and local config state
- `config_path`: return the resolved `config.toml` path
- `list_aliases`: list aliases for the selected profile
- `get_stats`: get worklog stats for today, a date, week, last week, month, or year
- `preview_log_time`: build a worklog draft without creating a Tempo worklog

Optional write mode:

- `log_time`: create a Tempo worklog using the same core inputs as `preview_log_time`
- start `logit-mcp` with `--enable-write-tools` to expose `log_time`
- default installs stay read-only so agents do not gain write access unless you opt in

### MCP quick start

```bash
logit setup

# install for one client
logit mcp install claude
logit mcp install codex
logit mcp install opencode

# opt in to write access
logit mcp --enable-write-tools install claude
```

### Client install commands

Automatic install commands:

```bash
logit mcp install claude
logit mcp install codex
logit mcp install opencode

# opt in to the mutating log_time MCP tool
logit mcp --enable-write-tools install claude
```

What these do:

- `claude`: runs `claude mcp add` in local scope for the current project
- `codex`: updates `~/.codex/config.toml` or `CODEX_HOME/config.toml`
- `opencode`: updates `~/.config/opencode/opencode.json` or `OPENCODE_CONFIG`

Notes:

- `logit setup` is still required before the installed MCP server can make real API calls
- installs stay read-only by default; add `--enable-write-tools` only if you want MCP clients to create worklogs
- rerun the install command later if you want to switch an existing install to write-enabled mode
- existing Claude local installs that already point at `logit-mcp` are updated in place
- installs are idempotent when the existing config already matches
- installs do not overwrite a different existing `logit` MCP entry
- OpenCode auto-install currently supports strict JSON configs only, not JSONC with comments

### Example MCP workflows

Typical agent requests:

- "Show my aliases for the active profile."
- "Get my stats for last week."
- "Preview logging 30 minutes to standup today."
- "Log 30 minutes to standup with message daily standup." Requires `--enable-write-tools`.

### Manual configuration

Direct server examples:

```bash
logit-mcp
logit-mcp --enable-write-tools
```

Manual OpenCode config:

```json
{
  "mcp": {
    "logit": {
      "type": "local",
      "command": ["/absolute/path/to/logit-mcp"]
    }
  }
}
```

Manual Claude Code config:

```json
{
  "projects": {
    "/absolute/path/to/project": {
      "mcpServers": {
        "logit": {
          "command": "/absolute/path/to/logit-mcp"
        }
      }
    }
  }
}
```

Manual Codex config:

```toml
[mcp_servers.logit]
command = "/absolute/path/to/logit-mcp"
args = ["--config-dir", "/path/to/config", "--data-dir", "/path/to/data", "--cache-dir", "/path/to/cache"]
```

If you need a specific profile, write access, or custom directories, include them in the client-specific args or command array.

Claude example:

```json
{
  "mcpServers": {
    "logit": {
      "command": "/absolute/path/to/logit-mcp",
      "args": ["--profile", "work", "--enable-write-tools"]
    }
  }
}
```

OpenCode example:

```json
{
  "mcp": {
    "logit": {
      "type": "local",
      "command": ["/absolute/path/to/logit-mcp", "--profile", "work", "--enable-write-tools"]
    }
  }
}
```

## CLI usage

### Log by duration

Issue first:

```bash
logit TK-1234 8h
logit TK-1234 1h 30m
```

Duration first:

```bash
logit 8h TK-1234
logit 45m TK-1234
```

With a message:

```bash
logit 1h TK-1234 -m "fixed flaky test"
```

### Log by explicit period

```bash
logit 04/01/2026 8 12 am - 04/01/2026 5 00 pm TK-1234
logit 04/01/2026 0812 - 04/01/2026 1700 TK-1234
```

### Dry run

Preview what would be logged without sending it:

```bash
logit --dry-run 1h TK-1234
```

You can also enable dry-run through the environment:

```bash
LOGIT_DRY_RUN=1 logit 1h TK-1234
```

### Log for a past date

```bash
logit 30m TK-1234 --date 2026-04-01
```

### Stats

```bash
logit stat
logit stat --details
logit stat today
logit stat 2026-04-01
logit stat week
logit stat week --details
logit stat last week
logit stat April
logit stat May 2026
logit stat 2026
```

`logit stat` shows a summary for the selected range.

Use `--details` when you want the individual worklog rows below the summary.

Examples:

- `logit stat` shows today
- `logit stat 2026-04-01` shows one day
- `logit stat week` shows a weekly summary
- `logit stat week --details` shows the weekly summary plus each worklog row

### Aliases

Create an alias:

```bash
logit alias standup TK-1234 --default-duration 30m -m "daily standup"
```

Use it:

```bash
logit standup
logit 1h standup -m "longer today"
```

List aliases:

```bash
logit alias list
```

Delete an alias:

```bash
logit alias delete standup
```

Skip Jira validation when creating an alias:

```bash
logit alias standup TK-1234 --no-validate
```

### Cache

Clear the active profile cache:

```bash
logit cache clear
```

### Config helpers

Show config file location:

```bash
logit config path
```

Show resolved config, data, and cache paths:

```bash
logit doctor
```

Edit config in your editor:

```bash
logit config edit
```

### Profiles

`logit` supports multiple profiles. Most people can stay on the default profile.

You can still select a profile explicitly:

```bash
logit --profile default stat
LOGIT_PROFILE=default logit stat
```

## Directory overrides

You can override all three directories.

Flags:

```bash
logit --config-dir /path/to/config --data-dir /path/to/data --cache-dir /path/to/cache stat
```

Environment variables:

```bash
export LOGIT_CONFIG_DIR=/path/to/config
export LOGIT_DATA_DIR=/path/to/data
export LOGIT_CACHE_DIR=/path/to/cache
```

Important: keep data and cache outside the config directory. `logit` will refuse to run if secrets would end up under the config tree.

## Troubleshooting

### Check resolved paths

```bash
logit doctor
```

### Check the current command list

```bash
logit --help
```

### Config edited into a bad state

If `logit config edit` saves invalid TOML or an unsupported schema, the invalid file is preserved as `config.toml.invalid`.

### Common auth failures

- `Tempo token rejected`: invalid Tempo API token
- `Jira credentials rejected`: invalid Jira email/token pair
- `unknown issue key or alias`: typo in issue key or alias name

## Development

Useful local checks:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```
