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

Ghscaff checks for new releases on startup. When a newer version is available, it prompts you to update. Choose "yes" to replace the running binary with the latest version.

If you installed ghscaff with `cargo install`, the auto-updater will refuse to touch the binary and instead direct you to run:
```bash
cargo install --force ghscaff
```

You can disable update checks by setting the environment variable:
```bash
GHSCAFF_NO_UPDATE_CHECK=1 ghscaff
```

## Uninstall

```bash
rm -f ~/.local/bin/ghscaff   # ghscaff binary
rm -rf ~/.ghscaff/           # boilerplate cache + encrypted vault
```
