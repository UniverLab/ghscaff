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
- **.gitignore** — GitHub's official template for the language, followed by an
  agentic block (see below).
- **.github/workflows/ci.yml** — CI workflow with basic checks.
- **.github/workflows/release.yml** — release workflow (runs on git tags; skipped when no license is chosen).
- **LICENSE** — fetched from the API based on the license selected during the wizard.

All files land in a single atomic `chore: init repository` commit.

When the wizard's license choice is **None**, the scaffold is marked
unpublishable: `Cargo.toml` gets `publish = false` instead of `license-file`,
the `rust-ci` job runs with `publish-check: false`, no `release.yml` is
committed, and `pyproject.toml` keeps no `license = …` line.

### The agentic .gitignore block

GitHub's gitignore templates predate AI coding agents, so none of them ignore
the directories those agents write. Every scaffolded repository therefore gets
a second block appended after the official template, separated by a blank
line. The block is derived from the canopy agent registry
(<https://github.com/UniverLab/canopy-registry>), so a harness the registry
lists — opencode, codebuddy, devin, kilo… — reaches new repositories as soon
as it appears there, with no ghscaff release in between:

```gitignore
# AI coding agents (paths derived from the canopy registry)
.agents/*
.claude/*
.opencode/*
...

# instruction files stay committable
!.claude/CLAUDE.md
...
```

Three properties worth knowing:

- **The official template is never modified** — it is preserved verbatim as the
  prefix, and the agentic block only ever follows it.
- **A failed fetch does not lose the block.** If GitHub's template cannot be
  retrieved the wizard warns and continues with an empty prefix, so the
  repository is still born ignoring agent state rather than failing outright.
- **The registry is best-effort.** ghscaff fetches it on every scaffold and
  keeps the merged result in its own cache directory; when the fetch fails it
  falls back to that cache (if younger than 24 h) and then to a bundled
  snapshot, printing one warning line that names the source used.
  Scaffolding never fails because of the registry.

The block is built with gitkit's own rules: directory paths become `<dir>/*`
so the directory stays trackable, `.github` and `.config` are never ignored,
and instruction files (`AGENTS.md`, `CLAUDE.md`, each platform's own) are
negated under `# instruction files stay committable`. The header is the exact
line `gitkit ignore add agentic` writes, so running gitkit later in a
scaffolded repository recognises the block and replaces it instead of
appending a second one. ghscaff carries its own copy of the registry snapshot
rather than depending on gitkit, because a machine that scaffolds a repository
may not have gitkit installed.

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
