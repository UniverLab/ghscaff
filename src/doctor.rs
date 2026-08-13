//! `ghscaff doctor` — verify that a repo's required status checks can ever be
//! satisfied.
//!
//! Branch protection and CI are configured independently: protection stores a
//! list of required context strings, CI reports whatever check names its jobs
//! happen to have today. Renaming a job (or the workflow that calls it) breaks
//! that link silently — the required check just never arrives, and because
//! every other check goes green, nothing on screen says the problem is
//! configuration rather than CI. This command detects that mismatch by
//! reading both sides of the live repo and comparing them, without changing
//! anything.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::apply::{auto_detect_repo, parse_owner_repo};
use crate::github::client::GithubClient;

/// A required context with nothing on the latest PR to satisfy it.
pub struct Unsatisfiable {
    pub context: String,
}

/// Required contexts that no reported check name satisfies. Pure and
/// fixture-testable: the CLI-facing [`run_doctor`] does the API calls and
/// hands the two lists here.
pub fn find_unsatisfiable(required: &[String], reported: &[String]) -> Vec<Unsatisfiable> {
    required
        .iter()
        .filter(|r| !reported.contains(r))
        .map(|context| Unsatisfiable {
            context: context.clone(),
        })
        .collect()
}

#[derive(Deserialize, Default)]
struct RequiredStatusChecks {
    #[serde(default)]
    contexts: Vec<String>,
}

#[derive(Deserialize, Default)]
struct ProtectionResponse {
    #[serde(default)]
    required_status_checks: Option<RequiredStatusChecks>,
}

fn get_required_contexts(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<Vec<String>> {
    let path = format!("/repos/{owner}/{repo}/branches/{branch}/protection");
    match client.get::<ProtectionResponse>(&path) {
        // No `required_status_checks` block means status checks aren't required
        // at all — nothing to verify.
        Ok(resp) => Ok(resp.required_status_checks.unwrap_or_default().contexts),
        Err(e) => {
            // No protection configured at all — nothing required, nothing to verify.
            if format!("{e:#}").contains("404") {
                Ok(Vec::new())
            } else {
                Err(e)
            }
        }
    }
}

const LATEST_PR_CHECKS_QUERY: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { pullRequests(first: 1, orderBy: {field: CREATED_AT, direction: DESC}) { nodes { number commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes { __typename ... on CheckRun { name } ... on StatusContext { context } } } } } } } } } } }";

#[derive(Serialize)]
struct GraphqlRequest {
    query: &'static str,
    variables: serde_json::Value,
}

#[derive(Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
}

#[derive(Deserialize)]
struct LatestPrData {
    repository: Option<RepositoryPrs>,
}

#[derive(Deserialize)]
struct RepositoryPrs {
    #[serde(rename = "pullRequests")]
    pull_requests: PullRequestConnection,
}

#[derive(Deserialize)]
struct PullRequestConnection {
    nodes: Vec<PullRequestNode>,
}

#[derive(Deserialize)]
struct PullRequestNode {
    number: u64,
    commits: CommitConnection,
}

#[derive(Deserialize)]
struct CommitConnection {
    nodes: Vec<CommitNode>,
}

#[derive(Deserialize)]
struct CommitNode {
    commit: Commit,
}

#[derive(Deserialize)]
struct Commit {
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<StatusCheckRollup>,
}

#[derive(Deserialize)]
struct StatusCheckRollup {
    contexts: RollupContexts,
}

#[derive(Deserialize)]
struct RollupContexts {
    nodes: Vec<RollupContextNode>,
}

#[derive(Deserialize)]
struct RollupContextNode {
    name: Option<String>,
    context: Option<String>,
}

/// The most recent pull request's number and the check names its latest
/// commit reported. `None` when the repo has no pull requests at all.
fn latest_pr_checks(data: Option<LatestPrData>) -> Option<(u64, Vec<String>)> {
    let pr = data?.repository?.pull_requests.nodes.into_iter().next()?;
    let checks = pr
        .commits
        .nodes
        .into_iter()
        .next()
        .and_then(|c| c.commit.status_check_rollup)
        .map(|r| {
            r.contexts
                .nodes
                .into_iter()
                .filter_map(|n| n.name.or(n.context))
                .collect()
        })
        .unwrap_or_default();
    Some((pr.number, checks))
}

fn get_latest_pr_checks(
    client: &GithubClient,
    owner: &str,
    repo: &str,
) -> Result<Option<(u64, Vec<String>)>> {
    let request = GraphqlRequest {
        query: LATEST_PR_CHECKS_QUERY,
        variables: serde_json::json!({ "owner": owner, "name": repo }),
    };
    let resp: GraphqlResponse<LatestPrData> = client.post("/graphql", &request)?;
    Ok(latest_pr_checks(resp.data))
}

/// Verify that `owner/repo`'s required status checks can actually be
/// satisfied by what its most recent pull request reports. Read-only: never
/// modifies branch protection. Exits non-zero (via an `Err`) when at least one
/// required context is unsatisfiable.
pub fn run_doctor(repo_arg: Option<&str>) -> Result<()> {
    let (token, _) = crate::github::client::resolve_token()?;
    let client = GithubClient::new(&token);

    let (owner, repo_name) = match repo_arg {
        Some(repo) => parse_owner_repo(repo)?,
        None => auto_detect_repo()?,
    };

    let gh_repo = crate::github::repo::get_repo(&client, &owner, &repo_name)?;
    let branch = gh_repo.default_branch;

    println!("  Checking {owner}/{repo_name} ({branch})...");
    println!();

    let required = get_required_contexts(&client, &owner, &repo_name, &branch)?;
    if required.is_empty() {
        println!("  ✓ No required status checks configured — nothing to verify.");
        return Ok(());
    }

    let Some((pr_number, reported)) = get_latest_pr_checks(&client, &owner, &repo_name)? else {
        println!("  ℹ No pull requests yet — nothing to compare against.");
        return Ok(());
    };

    let unsatisfiable = find_unsatisfiable(&required, &reported);
    if unsatisfiable.is_empty() {
        println!(
            "  ✓ All {} required check(s) are satisfiable (checked against PR #{pr_number}).",
            required.len()
        );
        return Ok(());
    }

    println!(
        "  ✗ {} of {} required check(s) can never be satisfied:",
        unsatisfiable.len(),
        required.len()
    );
    println!();
    for finding in &unsatisfiable {
        println!("    \"{}\"", finding.context);
        println!(
            "      required by branch protection, but no check on PR #{pr_number} reports it."
        );
        println!("      That pull request can never merge without an admin bypass.");
    }
    println!();
    anyhow::bail!(
        "{} required status check(s) will never be satisfied — see above",
        unsatisfiable.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_context_with_nothing_reported() {
        let required = vec!["rust-ci / Format, Lint & Test".to_string()];
        let reported = vec!["ci / Format, Lint & Test".to_string()];
        let found = find_unsatisfiable(&required, &reported);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].context, "rust-ci / Format, Lint & Test");
    }

    #[test]
    fn matches_the_production_evidence_exactly() {
        // Regression check against the harness-canopy 2026-08-07 incident.
        let required = vec!["rust-ci / Format, Lint & Test".to_string()];
        let reported = vec![
            "ci / Format, Lint & Test".to_string(),
            "ci / Publish Check".to_string(),
            "ci / Coverage".to_string(),
            "ci / Main PR Version Bump".to_string(),
        ];
        let found = find_unsatisfiable(&required, &reported);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].context, "rust-ci / Format, Lint & Test");
    }

    #[test]
    fn empty_when_every_required_context_is_reported() {
        let required = vec!["ci / Format, Lint & Test".to_string()];
        let reported = vec![
            "ci / Format, Lint & Test".to_string(),
            "ci / Coverage".to_string(),
        ];
        assert!(find_unsatisfiable(&required, &reported).is_empty());
    }

    #[test]
    fn empty_when_nothing_required() {
        assert!(find_unsatisfiable(&[], &["ci / Coverage".to_string()]).is_empty());
    }

    #[test]
    fn reports_every_unsatisfiable_context_not_just_the_first() {
        let required = vec![
            "rust-ci / Format, Lint & Test".to_string(),
            "rust-ci / Coverage".to_string(),
        ];
        let reported = vec!["ci / Format, Lint & Test".to_string()];
        let found = find_unsatisfiable(&required, &reported);
        assert_eq!(found.len(), 2);
    }

    fn parse_pr_checks(body: &str) -> Option<(u64, Vec<String>)> {
        let parsed: GraphqlResponse<LatestPrData> = serde_json::from_str(body).unwrap();
        latest_pr_checks(parsed.data)
    }

    #[test]
    fn extracts_contexts_from_protection_response() {
        let body = r#"{"required_status_checks":{"strict":true,"contexts":["ci / Format, Lint & Test","ci / Coverage"]}}"#;
        let resp: ProtectionResponse = serde_json::from_str(body).unwrap();
        assert_eq!(
            resp.required_status_checks.unwrap().contexts,
            vec![
                "ci / Format, Lint & Test".to_string(),
                "ci / Coverage".to_string()
            ]
        );
    }

    #[test]
    fn protection_response_with_no_required_status_checks_is_empty() {
        let body = r#"{"enforce_admins":{"enabled":false}}"#;
        let resp: ProtectionResponse = serde_json::from_str(body).unwrap();
        assert!(resp.required_status_checks.is_none());
    }

    #[test]
    fn protection_response_with_empty_contexts_is_empty() {
        let body = r#"{"required_status_checks":{"strict":true,"contexts":[]}}"#;
        let resp: ProtectionResponse = serde_json::from_str(body).unwrap();
        assert!(resp.required_status_checks.unwrap().contexts.is_empty());
    }

    #[test]
    fn extracts_check_run_names_from_pr_graphql_response() {
        let body = r#"{
            "data": {
                "repository": {
                    "pullRequests": {
                        "nodes": [{
                            "number": 42,
                            "commits": {
                                "nodes": [{
                                    "commit": {
                                        "statusCheckRollup": {
                                            "contexts": {
                                                "nodes": [
                                                    {"__typename":"CheckRun","name":"ci / Format, Lint & Test"},
                                                    {"__typename":"CheckRun","name":"ci / Coverage"}
                                                ]
                                            }
                                        }
                                    }
                                }]
                            }
                        }]
                    }
                }
            }
        }"#;
        let (number, checks) = parse_pr_checks(body).unwrap();
        assert_eq!(number, 42);
        assert_eq!(
            checks,
            vec![
                "ci / Format, Lint & Test".to_string(),
                "ci / Coverage".to_string()
            ]
        );
    }

    #[test]
    fn extracts_legacy_status_context_names_from_pr_graphql_response() {
        let body = r#"{
            "data": {
                "repository": {
                    "pullRequests": {
                        "nodes": [{
                            "number": 7,
                            "commits": {
                                "nodes": [{
                                    "commit": {
                                        "statusCheckRollup": {
                                            "contexts": {
                                                "nodes": [
                                                    {"__typename":"StatusContext","context":"legacy/status"}
                                                ]
                                            }
                                        }
                                    }
                                }]
                            }
                        }]
                    }
                }
            }
        }"#;
        let (number, checks) = parse_pr_checks(body).unwrap();
        assert_eq!(number, 7);
        assert_eq!(checks, vec!["legacy/status".to_string()]);
    }

    #[test]
    fn no_pull_requests_yields_none() {
        let body = r#"{
            "data": {
                "repository": {
                    "pullRequests": { "nodes": [] }
                }
            }
        }"#;
        assert!(parse_pr_checks(body).is_none());
    }

    #[test]
    fn pull_request_with_no_status_check_rollup_yields_empty_checks() {
        let body = r#"{
            "data": {
                "repository": {
                    "pullRequests": {
                        "nodes": [{
                            "number": 3,
                            "commits": {
                                "nodes": [{
                                    "commit": { "statusCheckRollup": null }
                                }]
                            }
                        }]
                    }
                }
            }
        }"#;
        let (number, checks) = parse_pr_checks(body).unwrap();
        assert_eq!(number, 3);
        assert!(checks.is_empty());
    }

    #[test]
    fn missing_repository_yields_none() {
        let body = r#"{"data": {"repository": null}}"#;
        assert!(parse_pr_checks(body).is_none());
    }

    #[test]
    fn graphql_errors_with_null_data_yield_none() {
        let body = r#"{"data": null, "errors": [{"message": "not found"}]}"#;
        assert!(parse_pr_checks(body).is_none());
    }

    // ── Mock-based integration tests ──────────────────────────────

    use super::super::github::test_utils::{mock_client, start_mock_server};

    #[test]
    fn get_required_contexts_returns_contexts_when_protected() {
        let url = start_mock_server(|path| {
            if path.contains("/branches/") && path.contains("/protection") {
                (200, r#"{"required_status_checks":{"strict":true,"contexts":["ci / Test","ci / Lint"]}}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let result = get_required_contexts(&client, "owner", "repo", "main").unwrap();
        assert_eq!(
            result,
            vec!["ci / Test".to_string(), "ci / Lint".to_string()]
        );
    }

    #[test]
    fn get_required_contexts_returns_empty_when_no_protection() {
        let url = start_mock_server(|_| (404, r#"{"message":"Not Found"}"#.to_string()));
        let client = mock_client(&url);
        let result = get_required_contexts(&client, "owner", "repo", "main").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn get_required_contexts_returns_empty_when_no_required_checks() {
        let url = start_mock_server(|path| {
            if path.contains("/branches/") && path.contains("/protection") {
                (200, r#"{"enforce_admins":{"enabled":true}}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let result = get_required_contexts(&client, "owner", "repo", "main").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn get_required_contexts_returns_error_on_server_error() {
        let url = start_mock_server(|path| {
            if path.contains("/branches/") && path.contains("/protection") {
                (500, r#"{"message":"Internal Server Error"}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let result = get_required_contexts(&client, "owner", "repo", "main");
        assert!(result.is_err());
    }

    #[test]
    fn get_latest_pr_checks_returns_checks_from_graphql() {
        let graphql_response = r#"{
            "data": {
                "repository": {
                    "pullRequests": {
                        "nodes": [{
                            "number": 5,
                            "commits": {
                                "nodes": [{
                                    "commit": {
                                        "statusCheckRollup": {
                                            "contexts": {
                                                "nodes": [
                                                    {"__typename":"CheckRun","name":"ci / Test"},
                                                    {"__typename":"StatusContext","context":"legacy/check"}
                                                ]
                                            }
                                        }
                                    }
                                }]
                            }
                        }]
                    }
                }
            }
        }"#;
        let url = start_mock_server(move |path| {
            if path == "/graphql" {
                (200, graphql_response.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let result = get_latest_pr_checks(&client, "owner", "repo").unwrap();
        assert!(result.is_some());
        let (number, checks) = result.unwrap();
        assert_eq!(number, 5);
        assert_eq!(
            checks,
            vec!["ci / Test".to_string(), "legacy/check".to_string()]
        );
    }

    #[test]
    fn get_latest_pr_checks_returns_none_when_no_prs() {
        let graphql_response = r#"{
            "data": {
                "repository": {
                    "pullRequests": { "nodes": [] }
                }
            }
        }"#;
        let url = start_mock_server(move |path| {
            if path == "/graphql" {
                (200, graphql_response.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let result = get_latest_pr_checks(&client, "owner", "repo").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn get_latest_pr_checks_returns_none_when_null_data() {
        let graphql_response = r#"{"data": null, "errors": [{"message": "not found"}]}"#;
        let url = start_mock_server(move |path| {
            if path == "/graphql" {
                (200, graphql_response.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let result = get_latest_pr_checks(&client, "owner", "repo").unwrap();
        assert!(result.is_none());
    }
}
