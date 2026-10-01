---
title: CLI Reference
description: Every ghscaff command and flag.
order: 8
---

# CLI Reference

```
ghscaff [command] [options]
```

Running `ghscaff` with no command starts the creation wizard (same as
`ghscaff new`).

## Commands

| Command | Description |
|---|---|
| `ghscaff` / `ghscaff new` | Create a new GitHub repository with the wizard |
| `ghscaff apply [owner/repo]` | Configure an existing repository (auto-detected from git remote if omitted) |
| `ghscaff doctor [owner/repo]` | Verify that required status checks can be satisfied (auto-detected from git remote if omitted) |
| `ghscaff config` | Reconfigure credentials — wipes the vault and starts fresh |
| `ghscaff update [--check] [--yes]` | Update to the latest stable release; always asks first. Exit codes: 0 = up to date (or installed / declined / cargo refusal), 1 = update available (`--check`), 2 = update check could not be completed (cause on stderr; also applies to plain `update`). `--dry-run` behaves like `--check`: it reports and never downloads or installs. |

## Global flags

| Flag | Description |
|---|---|
| `--sponsor OWNER/REPO` | Enable GitHub Sponsor button on an existing repository |
| `--dry-run` | Preview changes without making any API calls (`update --dry-run` reports like `--check`) |
| `--help` | Show help for any command |
| `--version` | Show ghscaff version |

## Environment

| Variable | Description |
|---|---|
| `GITHUB_TOKEN` | Token override — takes precedence over the vault (CI/CD friendly) |
| `CARGO_REGISTRY_TOKEN` | Resolved as a template secret for the Rust template |
| `GHSCAFF_NO_UPDATE_CHECK=1` | Silence the startup update notice (does not affect `ghscaff update`) |

## Files

| Path | Description |
|---|---|
| `~/.ghscaff/vault.enc` | Encrypted vault (token + template secrets) |
| `~/.ghscaff/` | Boilerplate cache |
