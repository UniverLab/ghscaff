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
        restrictions: Restrictions,
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
    #[derive(Serialize)]
    struct Restrictions {
        users: Vec<String>,
        teams: Vec<String>,
        apps: Vec<String>,
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
        restrictions: Restrictions {
            users: vec![],
            teams: vec![],
            apps: vec![],
        },
        allow_force_pushes: false,
    };
    
    // Retry with backoff for 422 errors (GitHub indexing delay)
    let mut delay = std::time::Duration::from_millis(500);
    for attempt in 0..3 {
        match client.put::<_, serde_json::Value>(
            &format!("/repos/{owner}/{repo}/branches/{branch}/protection"),
            &body,
        ) {
            Ok(_) => return Ok(()),
            Err(e) if attempt < 2 && e.to_string().contains("422") => {
                if crate::is_debug() {
                    eprintln!("  [debug] Branch protection attempt {} failed: {}", attempt + 1, e);
                }
                std::thread::sleep(delay);
                delay *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
