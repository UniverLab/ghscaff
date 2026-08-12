---
title: Apply Mode
description: Idempotently bring an existing repository up to your conventions.
order: 6
---

# Apply Mode

Apply mode configures an **existing** repository without recreating it:

```bash
ghscaff apply owner/repo

# Auto-detects owner/repo from the git remote if omitted
cd my-existing-project
ghscaff apply
```

## What gets applied

- ✅ Atomic single commit with all boilerplate files (no individual file
  commits).
- ✅ Labels enforced — creates missing, updates changed, **removes
  non-standard**.
- ✅ Branch protection on `main` and `develop` (if created), with required status checks automatically derived from the CI workflow.
- ✅ Topics — merged with existing ones.
- ✅ GitHub Actions secrets — from vault, env, or interactive prompt.
- ✅ Team access — optionally select teams and assign permissions interactively.
- ✅ CI/CD workflows — included in the boilerplate.
- ✅ `develop` branch — created if absent.

Safe to run multiple times: every operation is idempotent.

To verify that your required status checks can be satisfied, run:
```bash
ghscaff doctor owner/repo
```

## Previewing

```bash
ghscaff apply owner/repo --dry-run
```

Shows every change that would be made without any API calls.

> **Note:** label enforcement removes labels that are not in the
> [standard set](templates-and-conventions.md#standard-labels). If the
> target repo uses custom labels you want to keep, review the dry-run
> output first.
