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

The wizard guides you through **7 interactive steps**:

1. **Repository basics** — Name, description, topics
2. **Visibility & ownership** — Public/Private, personal or org
3. **Language / template** — Choose boilerplate (Rust, Python, etc.)
4. **Branches** — Default branch, develop branch, branch protection
5. **Features** — LICENSE, README, CI workflow, labels
6. **Review & confirm** — Verify all settings before creation
7. **Execution** — Live progress with step indicators

```
  Create a new GitHub repository

  Validating token... ok  (jheisonmb)

  Summary:
  ◆ jheisonmb/my-rust-cli
  ◆ description: A CLI tool built with Rust
  ◆ visibility: private
  ◆ language: Rust
  ◆ default branch: main
  ◆ develop branch: yes
  ◆ branch protection: yes
  ◆ license: MIT
  ◆ features: README, CI workflow, labels

  Apply these changes? (Y/n) y

  [1/14] create repo jheisonmb/my-rust-cli... ok  (https://github.com/jheisonmb/my-rust-cli)
  [2/14] commit Cargo.toml... ok
  [3/14] commit src/main.rs... ok
  [4/14] commit README.md... ok
  [5/14] commit .gitignore... ok
  [6/14] commit CI workflow... ok
  [7/14] create develop branch... ok
  [8/14] sync labels... ok  (12 created)

  Done  —  https://github.com/jheisonmb/my-rust-cli
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
- ✅ Labels (creates missing, updates existing)
- ✅ Branch protection on `main`
- ✅ Topics (merges with existing)
- ✅ CI workflow (creates if absent)
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
- **Cargo.toml** / **package.json** / etc. — dependency manifest
- **src/main.rs** — entry point boilerplate
- **README.md** — template with name and description
- **.gitignore** — language-specific (via GitHub API)
- **.github/workflows/ci.yml** — CI/CD workflow

### v1 — Rust

Includes:
- `cargo fmt --check` — code formatting
- `cargo clippy -- -D warnings` — linting
- `cargo test` — test suite
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

### v1 — Rust scaffold + apply mode ✓ IN PROGRESS
- [x] Full wizard flow (`new`)
- [ ] `apply` mode for existing repos
- [x] `--dry-run` in both modes
- [x] Rust language template
- [x] Standard labels (6 core labels)
- [x] Branch protection
- [ ] Publish to crates.io

### v2 — Multi-language
- [ ] Python template
- [ ] Node.js/TypeScript template
- [ ] Java/Spring template
- [ ] `CONTRIBUTING.md` template
- [ ] Issue templates

### v3 — Config & presets
- [ ] `~/.config/ghscaff/presets.toml` — save wizard configs
- [ ] `ghscaff --preset my-rust-lib` — skip wizard

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

---

## License

MIT — see [LICENSE](LICENSE) for details.

---

## Support

- 📖 [GitHub Issues](https://github.com/UniverLab/ghscaff/issues) — Report bugs or request features
- 💬 [Discussions](https://github.com/UniverLab/ghscaff/discussions) — Ask questions
- 🐦 Twitter: [@JheisonMB](https://twitter.com/JheisonMB)

---

Made with ❤️ by [JheisonMB](https://github.com/JheisonMB)
