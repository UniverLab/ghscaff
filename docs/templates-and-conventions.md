---
title: Templates & Conventions
description: Boilerplate templates, the standard label set, branch protection rules and template secrets.
order: 7
---

# Templates & Conventions

## Boilerplate templates

Each language template includes:

- **Dependency manifest** — e.g. `Cargo.toml`
- **Entry point** — boilerplate source file.
- **README.md** — with placeholders for name and description.
- **.gitignore** — language-specific (fetched from the GitHub API).
- **.github/workflows/ci.yml** — CI workflow with basic checks.
- **.github/workflows/release.yml** — release workflow (runs on git tags).
- **LICENSE** — fetched from the API based on the license selected during the wizard.

All files land in a single atomic `chore: init repository` commit.

Available today: **Rust**. Python, Node.js and Java are planned.

## Standard labels

7 labels are enforced on every repo. Non-standard labels are removed.

| Label | Color | Description |
|-------|-------|-------------|
| `bug` | `#d73a4a` | Something isn't working |
| `feature` | `#a2eeef` | New feature or request |
| `documentation` | `#0075ca` | Improvements to docs |
| `breaking-change` | `#e4e669` | Introduces breaking changes |
| `target:main` | `#1d76db` | Targets the main branch |
| `target:develop` | `#0e8a16` | Targets the develop branch |
| `help wanted` | `#008672` | Extra attention needed |

## Branch protection

When enabled, ghscaff applies to the default branch (and `develop` when
present):

- ✅ Require 1 approval before merging.
- ✅ Require status checks to pass (automatically derived from the CI workflow).
- ✅ Dismiss stale reviews.
- ✅ Disallow force-push.

### Status Check Derivation

GitHub normally requires you to manually type the exact names of CI checks
that must pass during branch protection setup. A typo or job name mismatch
creates a rule that silently guards nothing.

Ghscaff reads the CI workflow it commits and derives the required check
names from it automatically. If you later rename a job in your CI workflow,
the protection rule automatically refers to the new name on your next
`ghscaff apply` run — no manual synchronization needed.

To verify that your required checks can be satisfied by the latest CI run,
use the `doctor` command:

```bash
ghscaff doctor owner/repo
```

## Secrets

Templates declare required GitHub Actions secrets in `secrets.toml`.
Ghscaff resolves each one in order:

1. **Encrypted vault** — previously saved secrets.
2. **Environment variable** — e.g. `CARGO_REGISTRY_TOKEN`.
3. **Interactive prompt** — with the option to save to the vault.

For the Rust template:

- **`CARGO_REGISTRY_TOKEN`** — required for publishing to crates.io
  ([get one here](https://crates.io/me)).
