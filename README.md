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

[![CI](https://github.com/UniverLab/ghscaff/actions/workflows/ci.yml/badge.svg)](https://github.com/UniverLab/ghscaff/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Interactive CLI wizard for creating and configuring GitHub repositories. One binary, zero runtime dependencies.

---

## Features

- **🪄 Interactive wizard** — Create GitHub repos with a conversational guided flow
- **⚡ Zero dependencies** — Single binary, no runtime requirements
- **🔄 Idempotent apply mode** — Configure existing repos without recreation
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

The wizard guides you through **6 interactive steps**:

1. **Repository basics** — Name, description, topics
2. **Visibility & ownership** — Public/Private, personal or org
3. **Language / template** — Choose boilerplate (Rust, Python, etc.)
4. **Branches** — Default branch, develop branch
5. **Features** — LICENSE, standard labels
6. **Review & confirm** — Verify all settings before creation

Then **automatically**:
- Creates the repository
- Commits all boilerplate files in a single atomic commit (`chore: init repository`)
- Applies branch protection to main (and develop if created)
- Syncs labels, topics, and CI/CD workflows
- Configures required GitHub Actions secrets from `secrets.toml`

```
  Create a new GitHub repository

  Validating token... ok  (JheisonMB)

  > Repository name: my-rust-cli
  > Description: A CLI tool built with Rust
  > Topics: rust, cli, tool
  > Visibility: Public
  > Owner: UniverLab
  > Language: rust
  > Default branch: main
  > Create develop branch? Yes
  > Features: LICENSE, Standard labels
  > License: MIT

  Apply these changes? Yes

  Fetching boilerplate template... ok
  [1/7] create repo UniverLab/my-rust-cli... ok  (https://github.com/UniverLab/my-rust-cli)
  [2/7] init repository... ok
  [3/7] create develop branch... ok
  [4/7] apply branch protection (main)... ok
  [5/7] apply branch protection (develop)... ok
  [6/7] sync labels... ok
  [7/7] set topics... ok

  Done  —  https://github.com/UniverLab/my-rust-cli
```

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

If the token is missing or invalid, ghscaff fails immediately with a clear error message before prompting anything else.

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
- **`CONTRIBUTING.md`** — Developer guide (includes secrets config for maintainers)
- **LICENSE** — Placeholder (user selects license type during wizard)

All files are merged into a single atomic `chore: init repository` commit.

### v1 — Rust

CI workflow includes:
- `cargo fmt --check` — code formatting
- `cargo clippy -- -D warnings` — linting
- `cargo test` — test suite

Release workflow:
- Builds and publishes to [crates.io](https://crates.io)
- Requires `CARGO_REGISTRY_TOKEN` secret (configured during wizard)
- Default topics: `rust`, `cli`

### v2+ Planned

- Python (poetry, FastAPI examples)
- Node.js / TypeScript (npm, Next.js examples)
- Java / Spring Boot

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

---

## Development

### Requirements

- Rust 1.70+
- Cargo

### Build

```bash
cargo build --release
```

### Test

```bash
cargo test
```

### Lint

```bash
cargo fmt --check
cargo clippy -- -D warnings
```

### Format

```bash
cargo fmt
```

---

## Roadmap

### v1 — Rust scaffold + apply mode ✓ COMPLETE
- [x] Full wizard flow (`new`)
- [x] `apply` mode for existing repos
- [x] `--dry-run` in both modes
- [x] Rust language template
- [x] Standard labels (6 core labels)
- [x] Branch protection
- [x] Single atomic commit (Git Trees API)
- [x] Template secrets (GitHub Actions API)
- [x] Self-update detection on startup
- [ ] Publish to crates.io

### v2 — Multi-language
- [ ] Python template (poetry/pip)
- [ ] Node.js/TypeScript template (npm/yarn)
- [ ] Java/Spring template
- [ ] `templates.toml` for per-template feature configuration

### v3 — Config & presets
- [ ] `~/.config/ghscaff/presets.toml` — save wizard configs
- [ ] `ghscaff --preset my-rust-lib` — skip wizard
- [ ] `ghscaff clone --preset` — initialize from preset

### v4 — Advanced
- [ ] Issue templates (`ISSUE_TEMPLATE/`)
- [ ] Pull request templates (`PULL_REQUEST_TEMPLATE/`)
- [ ] Organization-level config
- [ ] Monorepo scaffold support

---

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit with conventional commits (`git commit -am 'feat: add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

All code must:
- Pass `cargo fmt --check`
- Pass `cargo clippy -- -D warnings`
- Pass `cargo test`

### Secrets Configuration

If you're extending `ghscaff` with new templates or modifying the release workflow, you may need to set up GitHub Actions secrets for your development fork:

- **`CARGO_REGISTRY_TOKEN`** — Required for publishing Rust crates to crates.io
  - Get your token from [crates.io/me](https://crates.io/me)
  - Add it as a repository secret in GitHub (`Settings > Secrets and variables > Actions`)

See [CONTRIBUTING.md](CONTRIBUTING.md) in each template's boilerplate for maintainer setup instructions.

---

## License

MIT — see [LICENSE](LICENSE) for details.

---

## Support

- 📖 [GitHub Issues](https://github.com/UniverLab/ghscaff/issues) — Report bugs or request features
- 💬 [Discussions](https://github.com/UniverLab/ghscaff/discussions) — Ask questions
- 🐦 Twitter: [@JheisonMB](https://twitter.com/JheisonMB)

---

Made with ❤️ by [JheisonMB](https://github.com/JheisonMB) and [UniverLab](https://github.com/UniverLab)
