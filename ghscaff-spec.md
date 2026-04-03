# ghscaff — Specification v1.1

> Interactive TUI wizard for creating and configuring GitHub repositories via the GitHub REST API. One binary, zero runtime dependencies.

---

## 1. Overview

`ghscaff` is a Rust CLI with two modes:

- **`new`** — interactive wizard that creates a fully configured GitHub repo from scratch
- **`apply`** — configures an existing repo (labels, branch protection, topics, CI) without recreating it

**Design philosophy:**
- One binary, zero runtime dependencies
- Wizard-first UX: the user answers questions, ghscaff does the rest
- Language-aware: templates are per-language, new languages are additive
- Idempotent: re-running only patches what's missing or outdated

---

## 2. Authentication

`ghscaff` reads the GitHub token exclusively from the `GITHUB_TOKEN` environment variable.

```bash
export GITHUB_TOKEN=ghp_xxxxxxxxxxxx
ghscaff
```

If the variable is absent or the token is invalid, the tool fails immediately with a clear error before prompting anything else.

Required token scopes: `repo`, `workflow`.

---

## 3. Modes

### `ghscaff new` (or `ghscaff` with no args)

Interactive TUI wizard. Creates a new repo and configures it end-to-end.

### `ghscaff apply [owner/repo]`

Configures an existing repo. If `owner/repo` is omitted, detects it automatically from the current directory's git remote (`origin`).

Applies only what's missing or outdated:
- Labels (creates missing, updates color/description of existing)
- Branch protection on `main` (creates or patches)
- Topics (merges with existing, does not remove)
- CI workflow (creates `.github/workflows/ci.yml` if absent)
- `develop` branch (creates if absent)

Idempotent — safe to run multiple times.

### `--dry-run` (both modes)

Prints what would be done without making any API call. Available in both `new` and `apply`.

---

## 4. Wizard Flow (`new` mode)

The TUI is built with [`ratatui`](https://github.com/ratatui-org/ratatui) + [`crossterm`](https://github.com/crossterm-rs/crossterm). Each step is a screen; the user navigates with arrow keys, Tab, and Enter.

```
Step 1 — Repository basics
Step 2 — Visibility & ownership
Step 3 — Language / template
Step 4 — Branches
Step 5 — Features (checklist)
Step 6 — Review & confirm
Step 7 — Execution (live progress)
```

### Step 1 — Repository basics
| Field | Type | Default |
|---|---|---|
| Repository name | Text input | — |
| Description | Text input | "" |
| Topics (comma-separated) | Text input | "" |

### Step 2 — Visibility & ownership
| Field | Type | Default |
|---|---|---|
| Visibility | Select: Public / Private | Private |
| Owner | Select: personal / org (lists orgs via API) | personal |

### Step 3 — Language / template
Select one language. Drives `.gitignore`, Actions workflow, and README badge defaults.

**v1 languages:** Rust

**Planned (v2+):** Python, Java/Spring, Node.js/TypeScript

### Step 4 — Branches
| Field | Type | Default |
|---|---|---|
| Default branch name | Text input | `main` |
| Create `develop` branch | Toggle | Yes |
| Enable branch protection on `main` | Toggle | Yes |

Branch protection rules (when enabled):
- Require PR before merging
- Require at least 1 approval
- Dismiss stale reviews
- Require status checks to pass (wired to the CI workflow name)
- Disallow force-push

### Step 5 — Features checklist
- [x] `.gitignore` (language-specific via GitHub API)
- [x] `LICENSE` → sub-select: MIT / Apache-2.0 / GPL-3.0 / None
- [x] `README.md` (template with name, description, CI badge)
- [x] GitHub Actions CI workflow
- [x] Standard label set
- [ ] `CONTRIBUTING.md` (v2)
- [ ] Issue templates (v2)

### Step 6 — Review & confirm
Full summary of all choices before any API call. User confirms with Enter or goes back with Esc.

### Step 7 — Execution
Live progress screen. Each operation is a numbered step with spinner and ✓/✗ outcome.

Files are committed before applying branch protection — guarantees at least one commit exists and the workflow name is known to GitHub before being required as a status check.

```
[1/9]  ✓ Token validated
[2/9]  ✓ Repository created — github.com/jheisonmb/my-project
[3/9]  ✓ Initial files committed (Cargo.toml, src/main.rs, .gitignore, LICENSE, README.md, .github/workflows/ci.yml)
[4/9]  ✓ develop branch created (from main @ abc1234)
[5/9]  ✓ Branch protection applied to main
[6/9]  ✓ Labels created (12)

Done in 3.2s
```

On any failure the step shows ✗ with the API error message and execution stops.

---

## 5. `apply` mode execution

Auto-detects `owner/repo` from `git remote get-url origin` if not provided.

```
[1/5]  ✓ Token validated
[2/5]  ✓ Repository found — github.com/jheisonmb/texforge
[3/5]  ✓ Labels synced (3 created, 2 updated, 7 already up to date)
[4/5]  ✓ Branch protection applied to main
[5/5]  ✓ develop branch already exists — skipped

Done in 1.1s
```

---

## 6. Language Templates

Each language implements a `LanguageTemplate` trait:

```rust
pub trait LanguageTemplate {
    fn name(&self) -> &'static str;
    fn gitignore_name(&self) -> &'static str;
    fn ci_workflow(&self) -> &'static str;
    fn boilerplate_files(&self, ctx: &ScaffContext) -> Vec<RepoFile>;
    fn readme_badges(&self) -> Vec<Badge>;
    fn default_topics(&self) -> Vec<&'static str>;
}
```

### Rust template (v1)

**CI workflow** (`.github/workflows/ci.yml`):
```yaml
name: CI
on:
  pull_request:
    branches: [main, develop]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo fmt --check
      - run: cargo clippy -- -D warnings
      - run: cargo test
```

**Default topics:** `rust`, `cli`

---

## 7. Standard Label Set

| Name | Color | Description |
|---|---|---|
| `bug` | `#d73a4a` | Something isn't working |
| `enhancement` | `#a2eeef` | New feature or request |
| `documentation` | `#0075ca` | Improvements to docs |
| `breaking-change` | `#e4e669` | Introduces breaking changes |
| `good first issue` | `#7057ff` | Good for newcomers |
| `help wanted` | `#008672` | Extra attention needed |
| `wontfix` | `#ffffff` | This will not be worked on |
| `duplicate` | `#cfd3d7` | This issue already exists |
| `question` | `#d876e3` | Further information requested |
| `dependencies` | `#0366d6` | Dependency updates |
| `ci/cd` | `#f9d0c4` | CI/CD related changes |
| `refactor` | `#e99695` | Code refactor, no behavior change |

---

## 8. Crate Structure

```
ghscaff/
├── Cargo.toml
├── src/
│   ├── main.rs               # Entry point, token validation, mode dispatch
│   ├── wizard/
│   │   ├── mod.rs            # Wizard orchestrator
│   │   ├── steps.rs          # Step definitions and state machine
│   │   └── ui.rs             # ratatui rendering
│   ├── github/
│   │   ├── mod.rs
│   │   ├── client.rs         # Thin reqwest wrapper, auth header
│   │   ├── repo.rs           # create repo, get gitignore
│   │   ├── branches.rs       # create branch, branch protection
│   │   ├── contents.rs       # create file (base64 commit)
│   │   └── labels.rs         # create/update labels
│   ├── templates/
│   │   ├── mod.rs            # LanguageTemplate trait
│   │   └── rust.rs           # Rust implementation
│   ├── apply.rs              # apply mode: detect remote, sync labels/protection/branches
│   └── executor.rs           # Ordered API operations with live progress
```

---

## 9. Key Dependencies

| Crate | Purpose |
|---|---|
| `ratatui` | TUI framework |
| `crossterm` | Cross-platform terminal backend |
| `reqwest` (blocking) | GitHub API HTTP client |
| `serde` / `serde_json` | JSON serialization |
| `base64` | File content encoding for GitHub contents API |
| `anyhow` | Error handling |

---

## 10. GitHub API Endpoints Used

| Operation | Endpoint |
|---|---|
| Validate token / get user | `GET /user` |
| List orgs | `GET /user/orgs` |
| Create repo | `POST /user/repos` or `POST /orgs/{org}/repos` |
| Get repo | `GET /repos/{owner}/{repo}` |
| Get gitignore template | `GET /gitignore/templates/{name}` |
| Create branch | `POST /repos/{owner}/{repo}/git/refs` |
| Get branch SHA | `GET /repos/{owner}/{repo}/git/ref/heads/{branch}` |
| Branch protection | `PUT /repos/{owner}/{repo}/branches/{branch}/protection` |
| Create file (commit) | `PUT /repos/{owner}/{repo}/contents/{path}` |
| List labels | `GET /repos/{owner}/{repo}/labels` |
| Create label | `POST /repos/{owner}/{repo}/labels` |
| Update label | `PATCH /repos/{owner}/{repo}/labels/{name}` |
| Replace topics | `PUT /repos/{owner}/{repo}/topics` |

---

## 11. Roadmap

### v1 — Rust scaffold + apply mode
- [ ] Full wizard flow (`new`)
- [ ] `apply` mode for existing repos
- [ ] `--dry-run` in both modes
- [ ] Rust language template
- [ ] All features in Step 5
- [ ] Published to crates.io as `ghscaff`

### v2 — Multi-language
- [ ] Python template
- [ ] Node.js/TypeScript template
- [ ] Java/Spring template
- [ ] `CONTRIBUTING.md` and issue templates

### v3 — Config & presets
- [ ] `~/.config/ghscaff/presets.toml` — save and replay wizard configs
- [ ] `ghscaff --preset my-rust-lib` — skip the wizard entirely

---

## 12. Non-Goals (v1)

- No local `git clone` after creation
- No GitHub Enterprise support
- No interactive token setup flow
- No deletion or teardown commands
- No `tokio` / async (blocking reqwest is sufficient)
