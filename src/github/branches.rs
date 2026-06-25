use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

#[derive(Deserialize)]
struct Ref {
    object: RefObject,
}

#[derive(Deserialize)]
struct RefObject {
    sha: String,
}

pub fn get_branch_sha(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<String> {
    let r: Ref = client.get(&format!("/repos/{owner}/{repo}/git/ref/heads/{branch}"))?;
    Ok(r.object.sha)
}

pub fn create_branch(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
    sha: &str,
) -> Result<()> {
    #[derive(Serialize)]
    struct Body<'a> {
        r#ref: &'a str,
        sha: &'a str,
    }
    let _: serde_json::Value = client.post(
        &format!("/repos/{owner}/{repo}/git/refs"),
        &Body {
            r#ref: &format!("refs/heads/{branch}"),
            sha,
        },
    )?;
    Ok(())
}

pub fn apply_branch_protection(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
    ci_check: Option<&str>,
) -> Result<()> {
    #[derive(Serialize)]
    struct Body<'a> {
        required_status_checks: RequiredChecks<'a>,
        enforce_admins: bool,
        required_pull_request_reviews: Reviews,
        // `restrictions` (push restrictions by users/teams/apps) is ONLY valid for
        // organization-owned repos. Sending an object — even with empty arrays — on a
        // user-owned repo returns a permanent 422 "Validation Failed". The field is
        // required but nullable, so we always send `null`: ghscaff never configures any
        // push restrictions, and `null` is accepted by both user- and org-owned repos.
        restrictions: Option<()>,
        allow_force_pushes: bool,
    }
    #[derive(Serialize)]
    struct RequiredChecks<'a> {
        strict: bool,
        contexts: Vec<&'a str>,
    }
    #[derive(Serialize)]
    struct Reviews {
        dismiss_stale_reviews: bool,
        required_approving_review_count: u8,
    }

    let body = Body {
        required_status_checks: RequiredChecks {
            strict: true,
            contexts: ci_check.into_iter().collect(),
        },
        enforce_admins: false,
        required_pull_request_reviews: Reviews {
            dismiss_stale_reviews: true,
            required_approving_review_count: 1,
        },
        restrictions: None,
        allow_force_pushes: false,
    };
    
    // Wait for branch to be indexed by GitHub (up to 10 seconds)
    let mut wait = std::time::Duration::from_millis(500);
    for attempt in 0..5 {
        if crate::is_debug() {
            eprintln!("  [debug] Checking branch {} exists (attempt {}/{})", branch, attempt + 1, 5);
        }
        match get_branch_sha(client, owner, repo, branch) {
            Ok(sha) => {
                if crate::is_debug() {
                    eprintln!("  [debug] Branch {} found: {}", branch, sha);
                }
                break;
            }
            Err(e) if attempt < 4 => {
                if crate::is_debug() {
                    eprintln!("  [debug] Branch {} not found yet: {}, waiting {:?}", branch, e, wait);
                }
                std::thread::sleep(wait);
                wait *= 2;
            }
            Err(e) => {
                if crate::is_debug() {
                    eprintln!("  [debug] Branch {} check failed after 5 attempts: {}", branch, e);
                }
                return Err(e);
            }
        }
    }
    
    if crate::is_debug() {
        eprintln!("  [debug] Applying branch protection to {}", branch);
    }
    client.put::<_, serde_json::Value>(
        &format!("/repos/{owner}/{repo}/branches/{branch}/protection"),
        &body,
    )?;
    if crate::is_debug() {
        eprintln!("  [debug] Branch protection applied successfully to {}", branch);
    }
    Ok(())
}
