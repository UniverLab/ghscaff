```
          █████       █████████                        ██████     ██████ 
         ░░███       ███░░░░░███                      ███░░███   ███░░███
  ███████ ░███████  ░███    ░░░   ██████   ██████    ░███ ░░░   ░███ ░░░ 
 ███░░███ ░███░░███ ░░█████████  ███░░███ ░░░░░███  ███████    ███████   
░███ ░███ ░███ ░███  ░░░░░░░░███░███ ░░░   ███████ ░░░███░    ░░░███░    
░███ ░███ ░███ ░███  ███    ░███░███  ███ ███░░███   ░███       ░███     
░░███████ ████ █████░░█████████ ░░██████ ░░████████  █████      █████    
 ░░░░░███░░░░ ░░░░░  ░░░░░░░░░   ░░░░░░   ░░░░░░░░  ░░░░░      ░░░░░     
 ███ ░███                                                                
░░██████                                                                 
 ░░░░░░                                                                   
```

<p align="center">
  <a href="https://github.com/UniverLab/ghscaff/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/UniverLab/ghscaff/ci.yml?branch=main&style=for-the-badge&label=CI" alt="CI"/></a>
  <a href="https://crates.io/crates/ghscaff"><img src="https://img.shields.io/crates/v/ghscaff?style=for-the-badge&logo=rust&logoColor=white" alt="Crates.io"/></a>
  <img src="https://img.shields.io/badge/Status-Active-27AE60?style=for-the-badge" alt="Status"/>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-2E8B57?style=for-the-badge" alt="License"/></a>
</p>

Interactive CLI wizard for creating and configuring GitHub repositories. One binary, zero runtime dependencies.

---

![Demo](assets/demo.gif)

---

## Features

- **🪄 Interactive wizard** — Create GitHub repos with a conversational guided flow
- **⚡ Zero dependencies** — Single binary, no runtime requirements
- **🔄 Idempotent apply mode** — Configure existing repos without recreation
- **👥 Team access control** — Assign repositories to organization teams with custom permissions (read, triage, write, admin)
- **🏷️ Smart labels** — Auto-create 6 core issue labels
- **🛡️ Branch protection** — Enforce reviews, status checks, and workflow validation
- **🚀 Language templates** — Rust (v1), Python/Node.js/Java coming soon
- **📝 Boilerplate files** — README, Cargo.toml, CI/CD workflows, LICENSE
- **🔐 Token validation** — Fail-fast authentication checks
- **🔑 Template secrets** — Automatically configures required GitHub Actions secrets per template
- **⬆️ Self-update** — Detects new releases on startup and offers one-command upgrade

---

## Installation

### Quick install (recommended)

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.ps1 | iex
```

This downloads and installs `ghscaff`. No Rust toolchain required.

You can customize the install:

```bash
# Pin a specific version
VERSION=0.1.0 curl -fsSL https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.sh | sh

# Install to a custom directory
INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.sh | sh
```

### Via cargo

```bash
cargo install ghscaff
```

Available on [crates.io](https://crates.io/crates/ghscaff).

### From source

```bash
git clone https://github.com/UniverLab/ghscaff.git
cd ghscaff
cargo build --release
# Binary at target/release/ghscaff
```

### GitHub Releases

Check the [Releases](https://github.com/UniverLab/ghscaff/releases) page for precompiled binaries (Linux x86_64, macOS x86_64/ARM64, Windows x86_64).

### Uninstall

```bash
rm -f ~/.local/bin/ghscaff         # ghscaff binary
rm -rf ~/.ghscaff/                 # boilerplate cache
```

---

## Quick Start

```bash
# Set your GitHub token
export GITHUB_TOKEN=ghp_xxxxxxxxxxxx

# Interactive wizard — create a new repo
ghscaff

# Or directly with a subcommand
ghscaff new

# Configure an existing repo
ghscaff apply owner/repo

# Preview changes without API calls
ghscaff --dry-run
```

---

## Wizard Flow

The wizard guides you through **7 interactive steps**:

1. **Repository basics** — Name, description, topics
2. **Visibility & ownership** — Public/Private, personal or org
3. **Team access** (org only) — Select teams and assign permissions (pull, triage, push, admin)
4. **Language / template** — Choose boilerplate (Rust, Python, etc.)
5. **Branches** — Default branch, develop branch
6. **Features** — LICENSE, standard labels
7. **Review & confirm** — Verify all settings before creation

Then **automatically**:
- Creates the repository
- Commits all boilerplate files in a single atomic commit (`chore: init repository`)
- Applies branch protection to main (and develop if created)
- Adds selected teams with their assigned permissions
- Syncs labels, topics, and CI/CD workflows
- Configures required GitHub Actions secrets from `secrets.toml`

---

## Apply Mode

Idempotently configure an existing repository:

```bash
ghscaff apply owner/repo

# Auto-detects from git remote if omitted
cd my-existing-project
ghscaff apply
```

Applies:
- ✅ Atomic single commit with all boilerplate files (no individual file commits)
- ✅ Labels (creates missing, updates existing)
- ✅ Branch protection on `main` and `develop` (if created)
- ✅ Topics (merges with existing)
- ✅ GitHub Actions secrets (from template's `secrets.toml`)
- ✅ CI/CD workflows (included in boilerplate)
- ✅ `develop` branch (creates if absent)

Safe to run multiple times — idempotent operations only.

---

## Dry-Run Mode

Preview changes without making any API calls:

```bash
ghscaff --dry-run

# Or with apply mode
ghscaff apply owner/repo --dry-run
```

---

## Authentication

`ghscaff` reads the GitHub token exclusively from the `GITHUB_TOKEN` environment variable:

```bash
export GITHUB_TOKEN=ghp_xxxxxxxxxxxx
ghscaff
```

**Required token scopes:**
- `repo` — Repository access
- `workflow` — GitHub Actions access
- `read:org` — (Optional, for team access feature) Organization and team access

If the token is missing or invalid, ghscaff fails immediately with a clear error message before prompting anything else.

**Note on team access:** If your token lacks the `read:org` scope, the wizard will skip the team selection step with a warning, but the rest of the repository setup will continue normally.

**Security note:** Never hardcode tokens. Use environment variables or secret managers.

---

## Boilerplate Templates

Each language template includes:
- **Dependency manifest** — Cargo.toml, package.json, etc.
- **Entry point** — src/main.rs boilerplate
- **README.md** — Template with placeholders for name and description
- **.gitignore** — Language-specific (fetched from GitHub API)
- **.github/workflows/ci.yml** — CI/CD workflow with basic checks
- **.github/workflows/release.yml** — Release workflow (published on Git tags)
- **LICENSE** — Placeholder (user selects license type during wizard)

All files are merged into a single atomic `chore: init repository` commit.

---

## Standard Label Set

6 core labels are auto-created with every new repo:

| Label | Color | Description |
|-------|-------|-------------|
| `bug` | `#d73a4a` | Something isn't working |
| `feature` | `#a2eeef` | New feature or request |
| `documentation` | `#0075ca` | Improvements to docs |
| `breaking-change` | `#e4e669` | Introduces breaking changes |
| `good first issue` | `#7057ff` | Good for newcomers |
| `help wanted` | `#008672` | Extra attention needed |

---

## Branch Protection

When enabled, applies to the default branch:
- ✅ Require 1 approval before merging
- ✅ Require status checks to pass (wired to CI workflow)
- ✅ Dismiss stale reviews
- ✅ Disallow force-push

--

### Secrets Configuration

If you're extending `ghscaff` with new templates or modifying the release workflow, you may need to set up GitHub Actions secrets for your development fork:

- **`CARGO_REGISTRY_TOKEN`** — Required for publishing Rust crates to crates.io
  - Get your token from [crates.io/me](https://crates.io/me)
  - Add it as a repository secret in GitHub (`Settings > Secrets and variables > Actions`)

---

## License

MIT — see [LICENSE](LICENSE) for details.

---

Made with ❤️ by [JheisonMB](https://github.com/JheisonMB) and [UniverLab](https://github.com/UniverLab)
