# ghscaff

[![CI](https://github.com/JheisonMB/ghscaff/actions/workflows/ci.yml/badge.svg)](https://github.com/JheisonMB/ghscaff/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Interactive CLI wizard for creating and configuring GitHub repositories. One binary, zero runtime dependencies.

---

## Installation

```bash
cargo install ghscaff
```

## Usage

```bash
export GITHUB_TOKEN=ghp_xxxxxxxxxxxx

# Interactive wizard — create a new repo
ghscaff

# Configure an existing repo
ghscaff apply [owner/repo]

# Preview without making API calls
ghscaff --dry-run
```

## License

MIT
