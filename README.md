# logit - Terminal-first Jira Tempo worklog logger with MCP
<p align=center>
<img width="727" height="465" alt="t-rec-trimmed-3" src="https://github.com/user-attachments/assets/04cc46f7-bda4-4d39-90b1-198bfd97a44d" />
</p>


Terminal-first Jira Tempo worklog logger with MCP support for Claude, Codex, OpenCode, and compatible clients.

`logit` installs three binaries:

- `logit`: CLI for logging time, stats, aliases, and setup
- `logit-mcp`: MCP server for Claude, Codex, OpenCode, and compatible clients
- `cli-tempo`: compatibility name for the main CLI

All three share the same local config, secrets, profiles, aliases, and cache.

## What you need

Before first setup, have these ready:

- Tempo API token
- Jira base URL, for example `https://your-company.atlassian.net`
- Jira account email
- Jira API token

`logit setup` validates those credentials before saving anything.

## Install

### Windows

Install from the latest GitHub Release with PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/kytmanov/logit/releases/latest/download/logit-installer.ps1 | iex"
```

What this installs:

- `logit.exe`
- `logit-mcp.exe`
- `cli-tempo.exe`

Notes:

- open a new terminal window after install so the updated `PATH` is picked up
- if you prefer not to use the installer script, download the Windows `.zip` from GitHub Releases and place the binaries somewhere on your `PATH`

### Homebrew

Recommended:

```bash
brew install kytmanov/tap/logit
```

Upgrade later with:

```bash
brew upgrade logit
```

### Build from source

Requirements:

- Rust stable toolchain

Build:

```bash
git clone https://github.com/kytmanov/logit.git
cd logit
cargo build --release
```

Built binaries:

- `target/release/logit`
- `target/release/logit-mcp`
- `target/release/cli-tempo`

Install all three with Cargo:

```bash
cargo install --path .
```

On Windows, Cargo installs:

- `logit.exe`
- `logit-mcp.exe`
- `cli-tempo.exe`

## End-to-end CLI setup

### 1. Run setup

```bash
logit setup
```

Setup prompts for:

- Jira URL
- Jira email
- Jira API token
- Tempo API token
- timezone
- working hours
- working days
- time format

On success, `logit` stores:

- config in the config directory
- secrets in the data directory
- cache in the cache directory

Check where those paths are on your machine:

```bash
logit doctor
logit config path
```

### 2. Confirm it works

Try a safe read-only command first:

```bash
logit stat
```

If you want to preview a worklog without sending it:

```bash
logit --dry-run 1h TK-1234
```

### 3. Log time

Issue first:

```bash
logit TK-1234 8h
logit TK-1234 1h 30m
```

Duration first:

```bash
logit 8h TK-1234
logit 45m TK-1234
logit 1h 15m standup
logit 1h15m standup
```

Compact mixed-unit durations like `1h15m` work in both issue-first and duration-first forms.

With a message:

```bash
logit 1h TK-1234 -m "fixed flaky test"
```

For a specific date:

```bash
logit 30m TK-1234 --date 2026-04-01
logit 3h TK-1234 2026-05-11
logit 3h TK-1234 yesterday
logit standup yesterday
```

Dated duration logs use the configured `work_hours.end` for that day as the end time, then subtract the duration.

For an explicit time range:

```bash
logit 04/01/2026 0812 - 04/01/2026 1700 TK-1234
```

## Connect MCP

Stats selectors also accept `yesterday`, for example:

```bash
logit stat yesterday
```

If you want to use `logit` from Claude, Codex, OpenCode, or another MCP client, do this after `logit setup`.

### What MCP exposes

Read-only by default:

- `doctor`
- `config_path`
- `list_aliases`
- `get_stats`
- `preview_log_time`

Optional write mode:

- `log_time`

Write access is off by default. Enable it only if you want the MCP client to create real Tempo worklogs.

### Automatic MCP install

`logit` can install client config for:

- Claude
- Codex
- OpenCode

Commands:

```bash
logit mcp install claude
logit mcp install codex
logit mcp install opencode
```

Write-enabled install:

```bash
logit mcp --enable-write-tools install claude
```

What each one does:

- `claude`: runs `claude mcp add` in local scope for the current project
- `codex`: updates `~/.codex/config.toml` or `CODEX_HOME/config.toml`
- `opencode`: updates `~/.config/opencode/opencode.json` or `OPENCODE_CONFIG`

Notes:

- run `logit setup` first, or the installed MCP server will exist but cannot make real API calls yet
- rerun the same install command later if you want to switch to write-enabled mode
- existing matching installs are left alone
- installs do not overwrite a different existing `logit` MCP entry
- OpenCode auto-install only supports strict JSON config files, not JSONC with comments

### Recommended client flows

Claude in the current project:

```bash
logit setup
logit mcp install claude
```

Codex user-level config:

```bash
logit setup
logit mcp install codex
```

OpenCode user-level config:

```bash
logit setup
logit mcp install opencode
```

### Manual MCP configuration

If your client is unsupported, or you want full control, point it at `logit-mcp` directly.

Basic server:

```bash
logit-mcp
```

Write-enabled server:

```bash
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

If you need a specific profile or write access, add the relevant args.

Example:

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

## Common tasks

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

```bash
logit cache clear
```

### Config helpers

```bash
logit config path
logit doctor
logit config edit
```

## Profiles

`logit` supports multiple profiles. Most people only need the default profile.

Use a named profile:

```bash
logit --profile work stat
LOGIT_PROFILE=work logit stat
```

If you install MCP for a non-default profile, include `--profile` when you run the install command so the client points at the right profile.

Example:

```bash
logit --profile work mcp install codex
```

## Directory overrides

Override config, data, and cache directories with flags:

```bash
logit --config-dir /path/to/config --data-dir /path/to/data --cache-dir /path/to/cache stat
```

Or with environment variables:

```bash
export LOGIT_CONFIG_DIR=/path/to/config
export LOGIT_DATA_DIR=/path/to/data
export LOGIT_CACHE_DIR=/path/to/cache
```

Important: keep data and cache outside the config directory. `logit` refuses to run if secrets would end up under the config tree.

## Troubleshooting

### `logit stat` says `run \`logit setup\``

Run:

```bash
logit setup
```

### Check paths and active profile

```bash
logit doctor
logit config path
```

### Check available commands

```bash
logit --help
logit-mcp --help
```

### Config edited into a bad state

If `logit config edit` saves invalid TOML or an unsupported schema, the invalid file is preserved as `config.toml.invalid`.

### Common auth failures

- `Tempo token rejected`: invalid Tempo API token
- `Jira credentials rejected`: invalid Jira email or Jira API token
- `unknown issue key or alias`: typo in issue key or alias name

## Development

Useful local checks:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```
