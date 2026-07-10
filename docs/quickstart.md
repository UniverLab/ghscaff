---
title: Quick Start
description: Create a repo with the wizard, configure an existing one, or preview with dry-run.
order: 3
---

# Quick Start

## Create a new repository

```bash
ghscaff
# or explicitly:
ghscaff new
```

On first run ghscaff asks for a GitHub token and stores it in the
[encrypted vault](authentication.md). The wizard then walks you through
[7 steps](wizard.md) — basics, visibility, teams, template, branches,
features, review — and creates the fully configured repository.

## Configure an existing repository

```bash
ghscaff apply owner/repo

# Auto-detects owner/repo from the git remote if omitted
cd my-existing-project
ghscaff apply
```

See [Apply Mode](apply-mode.md) for exactly what gets applied.

## Preview without touching anything

```bash
ghscaff --dry-run
ghscaff apply owner/repo --dry-run
```

Dry-run shows every change that would be made, without any API calls.

## Reconfigure credentials

```bash
ghscaff config
```

Wipes the vault (with confirmation) and starts fresh — new token,
optional passphrase.
