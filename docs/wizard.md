---
title: The Wizard
description: The 7 interactive steps and everything ghscaff automates after you confirm.
order: 5
---

# The Wizard

Running `ghscaff` (or `ghscaff new`) starts a conversational wizard with
**7 steps**:

1. **Repository basics** — name, description, topics.
2. **Visibility & ownership** — public/private, personal or organization.
3. **Team access** (org only) — select teams and assign permissions
   (pull, triage, push, admin).
4. **Language / template** — choose a boilerplate (Rust today; more
   languages coming).
5. **Branches** — default branch, optional `develop` branch.
6. **Features** — LICENSE, standard labels.
7. **Review & confirm** — verify all settings before creation.

## What happens after confirm

Ghscaff then performs the full setup automatically:

- Creates the repository.
- Commits all boilerplate files in a **single atomic commit**
  (`chore: init repository`) — no noisy per-file commits.
- Applies [branch protection](templates-and-conventions.md#branch-protection)
  to `main` (and `develop` if created), with required status checks
  automatically derived from the CI workflow.
- Adds the selected teams with their assigned permissions.
- Enforces the [standard label set](templates-and-conventions.md#standard-labels)
  — creates missing, updates changed, removes non-standard.
- Configures required GitHub Actions
  [secrets](templates-and-conventions.md#secrets) from the vault,
  environment, or an interactive prompt.
- Offers to enable the GitHub Sponsor button.

Every one of these operations is idempotent — if something already
matches, it is skipped, which is what makes
[Apply Mode](apply-mode.md) safe.
