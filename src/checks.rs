//! Derive branch-protection required status check contexts from the workflow
//! files ghscaff writes, instead of hardcoding them.
//!
//! GitHub composes a check name as `<calling job name> / <job name inside the
//! reusable workflow>` when a job invokes a local reusable workflow via
//! `uses: ./...`. Renaming either job later leaves any hardcoded required
//! context pointing at a name that no longer exists — the protected branch
//! then waits forever on a check nothing will report. Deriving the names from
//! the workflow content itself keeps them in sync automatically.

use serde::Deserialize;
use std::collections::BTreeMap;

/// A workflow file's repo-relative path and raw YAML text.
pub struct WorkflowSource<'a> {
    pub path: &'a str,
    pub content: &'a str,
}

#[derive(Deserialize)]
struct Workflow {
    #[serde(default)]
    jobs: BTreeMap<String, Job>,
}

#[derive(Deserialize, Default)]
struct Job {
    name: Option<String>,
    uses: Option<String>,
}

/// Derive the composed required status check contexts from a set of workflow
/// files. Only jobs that call another workflow in the same set via a local
/// `uses: ./...` reference contribute contexts — GitHub Actions requires that
/// prefix for same-repo reusable workflow calls, so it's the only case that
/// can be resolved without reaching out to GitHub.
pub fn derive_required_contexts(files: &[WorkflowSource]) -> Vec<String> {
    let workflows: BTreeMap<&str, Workflow> = files
        .iter()
        .filter(|f| is_workflow_path(f.path))
        .filter_map(|f| {
            serde_yaml::from_str::<Workflow>(f.content)
                .ok()
                .map(|w| (f.path, w))
        })
        .collect();

    let mut contexts = Vec::new();
    for workflow in workflows.values() {
        for (job_id, job) in &workflow.jobs {
            let Some(uses) = job.uses.as_deref() else {
                continue;
            };
            let Some(target_path) = local_reusable_workflow_path(uses) else {
                continue;
            };
            let Some(reused) = workflows.get(target_path.as_str()) else {
                continue;
            };
            let calling_name = job.name.clone().unwrap_or_else(|| job_id.clone());
            for (reused_job_id, reused_job) in &reused.jobs {
                let reused_name = reused_job
                    .name
                    .clone()
                    .unwrap_or_else(|| reused_job_id.clone());
                let context = format!("{calling_name} / {reused_name}");
                if !contexts.contains(&context) {
                    contexts.push(context);
                }
            }
        }
    }
    contexts.sort();
    contexts
}

/// The repo-relative paths a single workflow's jobs call as local reusable
/// workflows. Used to fetch just those extra files when the caller only has
/// one workflow's content in hand (e.g. reading a live repo).
pub fn referenced_local_workflows(content: &str) -> Vec<String> {
    let Ok(workflow) = serde_yaml::from_str::<Workflow>(content) else {
        return Vec::new();
    };
    workflow
        .jobs
        .values()
        .filter_map(|j| j.uses.as_deref())
        .filter_map(local_reusable_workflow_path)
        .collect()
}

/// GitHub requires a same-repo reusable workflow call to start with `./`,
/// resolved from the repository root. Anything else — a versioned external
/// workflow, an `owner/repo/...@ref` reference — can't be introspected from
/// files ghscaff has in hand, so it's left alone.
fn local_reusable_workflow_path(uses: &str) -> Option<String> {
    uses.strip_prefix("./").map(str::to_string)
}

fn is_workflow_path(path: &str) -> bool {
    path.starts_with(".github/workflows/") && (path.ends_with(".yml") || path.ends_with(".yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CI_YML: &str = r#"
name: CI
on: [pull_request]
jobs:
  ci:
    uses: ./.github/workflows/rust-ci.yml
"#;

    const RUST_CI_YML: &str = r#"
on: [workflow_call]
jobs:
  fmt-lint-test:
    name: "Format, Lint & Test"
    runs-on: ubuntu-latest
    steps: []
  publish-check:
    name: "Publish Check"
    runs-on: ubuntu-latest
    steps: []
  coverage:
    name: Coverage
    runs-on: ubuntu-latest
    steps: []
"#;

    #[test]
    fn derives_composed_contexts_from_reusable_workflow_call() {
        let files = vec![
            WorkflowSource {
                path: ".github/workflows/ci.yml",
                content: CI_YML,
            },
            WorkflowSource {
                path: ".github/workflows/rust-ci.yml",
                content: RUST_CI_YML,
            },
        ];
        let contexts = derive_required_contexts(&files);
        assert_eq!(
            contexts,
            vec![
                "ci / Coverage".to_string(),
                "ci / Format, Lint & Test".to_string(),
                "ci / Publish Check".to_string(),
            ]
        );
    }

    #[test]
    fn matches_the_production_evidence_exactly() {
        // Regression check against the harness-canopy 2026-08-07 incident:
        // required context was "rust-ci / Format, Lint & Test" while the PR
        // reported "ci / Format, Lint & Test" because the calling job's name
        // had been renamed from `rust-ci` to `ci`.
        let files = vec![
            WorkflowSource {
                path: ".github/workflows/ci.yml",
                content: CI_YML,
            },
            WorkflowSource {
                path: ".github/workflows/rust-ci.yml",
                content: RUST_CI_YML,
            },
        ];
        let contexts = derive_required_contexts(&files);
        assert!(contexts.contains(&"ci / Format, Lint & Test".to_string()));
        assert!(!contexts.contains(&"rust-ci / Format, Lint & Test".to_string()));
    }

    #[test]
    fn uses_job_id_when_job_has_no_name() {
        let ci = r#"
jobs:
  build:
    uses: ./.github/workflows/reusable.yml
"#;
        let reusable = r#"
jobs:
  test:
    runs-on: ubuntu-latest
    steps: []
"#;
        let files = vec![
            WorkflowSource {
                path: ".github/workflows/ci.yml",
                content: ci,
            },
            WorkflowSource {
                path: ".github/workflows/reusable.yml",
                content: reusable,
            },
        ];
        assert_eq!(derive_required_contexts(&files), vec!["build / test"]);
    }

    #[test]
    fn ignores_external_reusable_workflow_calls() {
        let ci = r#"
jobs:
  ci:
    uses: octo-org/octo-repo/.github/workflows/build.yml@v1
"#;
        let files = vec![WorkflowSource {
            path: ".github/workflows/ci.yml",
            content: ci,
        }];
        assert!(derive_required_contexts(&files).is_empty());
    }

    #[test]
    fn ignores_jobs_that_do_not_call_a_reusable_workflow() {
        let ci = r#"
jobs:
  lint:
    runs-on: ubuntu-latest
    steps: []
"#;
        let files = vec![WorkflowSource {
            path: ".github/workflows/ci.yml",
            content: ci,
        }];
        assert!(derive_required_contexts(&files).is_empty());
    }

    #[test]
    fn ignores_non_workflow_files() {
        let files = vec![
            WorkflowSource {
                path: "README.md",
                content: "jobs:\n  ci:\n    uses: ./.github/workflows/rust-ci.yml\n",
            },
            WorkflowSource {
                path: ".github/workflows/rust-ci.yml",
                content: RUST_CI_YML,
            },
        ];
        assert!(derive_required_contexts(&files).is_empty());
    }

    #[test]
    fn ignores_reference_to_a_workflow_not_in_the_set() {
        let ci = r#"
jobs:
  ci:
    uses: ./.github/workflows/missing.yml
"#;
        let files = vec![WorkflowSource {
            path: ".github/workflows/ci.yml",
            content: ci,
        }];
        assert!(derive_required_contexts(&files).is_empty());
    }

    #[test]
    fn deduplicates_identical_contexts_across_workflows() {
        let ci_a = r#"
jobs:
  ci:
    uses: ./.github/workflows/rust-ci.yml
"#;
        let ci_b = r#"
jobs:
  ci:
    uses: ./.github/workflows/rust-ci.yml
"#;
        let reusable = r#"
jobs:
  test:
    name: Test
    steps: []
"#;
        let files = vec![
            WorkflowSource {
                path: ".github/workflows/a.yml",
                content: ci_a,
            },
            WorkflowSource {
                path: ".github/workflows/b.yml",
                content: ci_b,
            },
            WorkflowSource {
                path: ".github/workflows/rust-ci.yml",
                content: reusable,
            },
        ];
        assert_eq!(derive_required_contexts(&files), vec!["ci / Test"]);
    }

    #[test]
    fn handles_unparseable_yaml_gracefully() {
        let files = vec![WorkflowSource {
            path: ".github/workflows/ci.yml",
            content: "not: valid: yaml: [",
        }];
        assert!(derive_required_contexts(&files).is_empty());
    }

    #[test]
    fn handles_empty_file_list() {
        assert!(derive_required_contexts(&[]).is_empty());
    }

    #[test]
    fn recognizes_yaml_extension_too() {
        let ci = r#"
jobs:
  ci:
    uses: ./.github/workflows/rust-ci.yaml
"#;
        let files = vec![
            WorkflowSource {
                path: ".github/workflows/ci.yaml",
                content: ci,
            },
            WorkflowSource {
                path: ".github/workflows/rust-ci.yaml",
                content: RUST_CI_YML,
            },
        ];
        assert!(!derive_required_contexts(&files).is_empty());
    }

    #[test]
    fn referenced_local_workflows_extracts_local_paths_only() {
        let content = r#"
jobs:
  ci:
    uses: ./.github/workflows/rust-ci.yml
  external:
    uses: octo-org/octo-repo/.github/workflows/build.yml@v1
  plain:
    runs-on: ubuntu-latest
    steps: []
"#;
        assert_eq!(
            referenced_local_workflows(content),
            vec![".github/workflows/rust-ci.yml".to_string()]
        );
    }

    #[test]
    fn referenced_local_workflows_empty_for_unparseable_content() {
        assert!(referenced_local_workflows("not valid yaml: [").is_empty());
    }

    #[test]
    fn referenced_local_workflows_empty_when_no_jobs_use_uses() {
        let content = "jobs:\n  a:\n    runs-on: ubuntu-latest\n";
        assert!(referenced_local_workflows(content).is_empty());
    }
}
