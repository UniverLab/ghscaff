---
title: Authentication
description: Token resolution order, the encrypted vault, and required scopes.
order: 4
---

# Authentication

## Token resolution order

Ghscaff resolves the GitHub token in this order:

1. **`GITHUB_TOKEN` env var** — for CI/CD and backward compatibility.
2. **Encrypted vault** (`~/.ghscaff/vault.enc`) — for secure local usage.
3. **Interactive prompt** — on first run, asks for the token and saves it
   to the vault.

## The encrypted vault

Tokens are encrypted with **XSalsa20-Poly1305**. The key is derived from
several machine-bound factors:

| Factor | Purpose |
|--------|---------|
| Username | Only your OS user can decrypt |
| Hostname | Copying the vault to another machine won't work |
| Binary path | Other programs can't derive the same key |
| Passphrase (optional) | Extra protection if desired |

The vault file has `0600` permissions and its directory `0700`. Writes
are atomic (temp file + rename) to prevent corruption.

The vault also stores [template secrets](templates-and-conventions.md#secrets)
such as `CARGO_REGISTRY_TOKEN`, so you only type them once.

## Reconfiguring

```bash
ghscaff config
```

This wipes the vault (with confirmation) and starts fresh — new token,
optional passphrase. Template secrets are requested again on the next run.

## Required token scopes

| Scope | Needed for |
|---|---|
| `repo` | Repository access |
| `workflow` | GitHub Actions access |
| `read:org` | (Optional) Organization and team access |

If the token lacks `read:org`, the wizard skips the team-selection step
with a warning; everything else proceeds normally.
