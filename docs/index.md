---
title: Ghscaff
description: Interactive CLI wizard for creating and configuring GitHub repositories — one binary, zero runtime dependencies.
order: 1
---

# Ghscaff

Ghscaff is an interactive CLI wizard for creating and configuring GitHub
repositories. One Rust binary, zero runtime dependencies — no `gh`, no
Node.js, no Python.

## Why ghscaff

Creating a repository "properly" is a dozen manual steps: boilerplate
files, CI workflows, labels, branch protection, team permissions, Actions
secrets. Ghscaff turns that into a 7-step conversational wizard, and makes
every operation **idempotent** so you can re-apply your conventions to
existing repositories at any time.

- **Interactive wizard** — create repos with a guided flow.
- **Encrypted vault** — tokens stored locally with XSalsa20-Poly1305,
  never in env vars or plain text.
- **Idempotent apply mode** — configure existing repos without recreation.
- **Team access control** — assign org teams with custom permissions.
- **Enforced labels** — a standard label set synced on every run.
- **Branch protection** — reviews, status checks, no force-push.
- **Language templates** — Rust today; Python, Node.js and Java coming.
- **Template secrets** — required Actions secrets configured automatically.
- **Self-update** — detects new releases on startup.

## How the documentation is organized

- [Installation](installation.md) — install, update and uninstall.
- [Quick Start](quickstart.md) — wizard, apply mode and dry-run.
- [Authentication](authentication.md) — token resolution and the encrypted vault.
- [The Wizard](wizard.md) — the 7 steps and what happens after confirm.
- [Apply Mode](apply-mode.md) — idempotent configuration of existing repos.
- [Templates & Conventions](templates-and-conventions.md) — boilerplates, labels, branch protection, secrets.
- [CLI Reference](cli-reference.md) — every command and flag.

## Part of UniverLab

Ghscaff is an experiment of [UniverLab](https://github.com/UniverLab),
an open computational laboratory. It follows the lab's engineering
principles: one tool one job, reproducibility first, offline-friendly
design.
