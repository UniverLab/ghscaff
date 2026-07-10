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
| `ghscaff config` | Reconfigure credentials — wipes the vault and starts fresh |

## Global flags

| Flag | Description |
|---|---|
| `--dry-run` | Preview changes without making any API calls |
| `--help` | Show help for any command |
| `--version` | Show ghscaff version |

## Environment

| Variable | Description |
|---|---|
| `GITHUB_TOKEN` | Token override — takes precedence over the vault (CI/CD friendly) |
| `CARGO_REGISTRY_TOKEN` | Resolved as a template secret for the Rust template |

## Files

| Path | Description |
|---|---|
| `~/.ghscaff/vault.enc` | Encrypted vault (token + template secrets) |
| `~/.ghscaff/` | Boilerplate cache |
