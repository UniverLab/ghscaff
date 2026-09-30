---
title: Installation
description: Install ghscaff with the quick installer, cargo, or from source.
order: 2
---

# Installation

## Quick install (recommended)

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.sh | sh
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.ps1 | iex
```

No Rust toolchain required. The installer accepts environment variables:

```bash
# Pin a specific version
VERSION=0.1.0 curl -fsSL https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.sh | sh

# Install to a custom directory
INSTALL_DIR=/usr/local/bin curl -fsSL https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.sh | sh
```

## Via cargo

```bash
cargo install ghscaff
```

Available on [crates.io](https://crates.io/crates/ghscaff).

## From source

```bash
git clone https://github.com/UniverLab/ghscaff.git
cd ghscaff
cargo build --release
# Binary at target/release/ghscaff
```

## GitHub Releases

Precompiled binaries for Linux x86_64, macOS x86_64/ARM64 and Windows
x86_64 are published on the
[Releases](https://github.com/UniverLab/ghscaff/releases) page.

## Self-update

Updating is always explicit — ghscaff never installs anything on its own:

```bash
ghscaff update            # asks "Update to 0.7.0? [y/N]" — default is NO
ghscaff update --yes      # skip the prompt
ghscaff update --check    # report only: exit 0 = up to date, 1 = update available, 2 = the check could not be completed
```

On "yes", ghscaff downloads the release asset for your platform, verifies it
against the release's `SHA256SUMS.txt` (skipped only when the release ships
no checksum file) and atomically replaces the running binary. On success it
prints `✓ updated to <version>` — no restart needed, the running binary is
replaced in place and the next invocation already runs the new one. Your
`~/.ghscaff` state — encrypted vault and boilerplate cache — is never touched.

Exit codes: `ghscaff update` (with or without `--check`) exits **2** when the
release check cannot be completed — no network, DNS or TLS failure, HTTP ≥ 400,
or an unparsable release-list response — and prints the cause on stderr; other
failures (download, checksum, permissions) are ordinary errors (exit 1).
`--check` exits 1 when an update is available, 0 when the binary is current.

If you installed ghscaff with `cargo install`, `ghscaff update` refuses to
touch the binary and instead prints:
`installed with cargo — run: cargo install --force ghscaff`

At startup ghscaff only prints a one-line notice when a newer release exists.
Disable that notice with:
```bash
GHSCAFF_NO_UPDATE_CHECK=1 ghscaff
```

## Uninstall

```bash
rm -f ~/.local/bin/ghscaff   # ghscaff binary
rm -rf ~/.ghscaff/           # boilerplate cache + encrypted vault
```
