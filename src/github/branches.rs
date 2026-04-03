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
    ci_check: &str,
) -> Result<()> {
    #[derive(Serialize)]
    struct Body<'a> {
        required_status_checks: RequiredChecks<'a>,
        enforce_admins: bool,
        required_pull_request_reviews: Reviews,
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
            contexts: vec![ci_check],
        },
        enforce_admins: false,
        required_pull_request_reviews: Reviews {
            dismiss_stale_reviews: true,
            required_approving_review_count: 1,
        },
        restrictions: None,
        allow_force_pushes: false,
    };
    let _: serde_json::Value = client.put(
        &format!("/repos/{owner}/{repo}/branches/{branch}/protection"),
        &body,
    )?;
    Ok(())
}
