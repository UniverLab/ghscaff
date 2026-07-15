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
            eprintln!(
                "  [debug] Checking branch {} exists (attempt {}/{})",
                branch,
                attempt + 1,
                5
            );
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
                    eprintln!(
                        "  [debug] Branch {} not found yet: {}, waiting {:?}",
                        branch, e, wait
                    );
                }
                std::thread::sleep(wait);
                wait *= 2;
            }
            Err(e) => {
                if crate::is_debug() {
                    eprintln!(
                        "  [debug] Branch {} check failed after 5 attempts: {}",
                        branch, e
                    );
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
        eprintln!(
            "  [debug] Branch protection applied successfully to {}",
            branch
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ref_deserialize() {
        let json = r#"{"object":{"sha":"abc123def456"}}"#;
        let r: Ref = serde_json::from_str(json).unwrap();
        assert_eq!(r.object.sha, "abc123def456");
    }

    #[test]
    fn test_ref_deserialize_full_github_response() {
        let json = r#"{
            "ref": "refs/heads/main",
            "node_id": "MDM6UmVmMjUyMjMzOTM5OjE=",
            "url": "https://api.github.com/repos/owner/repo/git/refs/heads/main",
            "object": {
                "sha": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
                "type": "commit",
                "url": "https://api.github.com/repos/owner/repo/git/commits/6dcb09b5b57875f334f61aebed695e2e4193db5e"
            }
        }"#;
        let r: Ref = serde_json::from_str(json).unwrap();
        assert_eq!(
            r.object.sha,
            "6dcb09b5b57875f334f61aebed695e2e4193db5e"
        );
    }

    #[test]
    fn test_ref_object_sha_various_lengths() {
        // Full SHA-1 hash
        let json = r#"{"object":{"sha":"da39a3ee5e6b4b0d3255bfef95601890afd80709"}}"#;
        let r: Ref = serde_json::from_str(json).unwrap();
        assert_eq!(r.object.sha.len(), 40);

        // Short SHA
        let json2 = r#"{"object":{"sha":"abc123"}}"#;
        let r2: Ref = serde_json::from_str(json2).unwrap();
        assert_eq!(r2.object.sha, "abc123");
    }

    #[test]
    fn test_body_ref_format() {
        #[derive(Serialize)]
        struct Body<'a> {
            r#ref: &'a str,
            sha: &'a str,
        }
        let body = Body {
            r#ref: "refs/heads/main",
            sha: "abc123",
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ref\":\"refs/heads/main\""));
        assert!(json.contains("\"sha\":\"abc123\""));
    }

    #[test]
    fn test_body_ref_with_slashes() {
        #[derive(Serialize)]
        struct Body<'a> {
            r#ref: &'a str,
            sha: &'a str,
        }
        let body = Body {
            r#ref: "refs/heads/feature/my-feature",
            sha: "deadbeef",
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ref\":\"refs/heads/feature/my-feature\""));
    }

    #[test]
    fn test_protection_body_serialization_with_ci() {
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
                contexts: vec!["ci/build"],
            },
            enforce_admins: false,
            required_pull_request_reviews: Reviews {
                dismiss_stale_reviews: true,
                required_approving_review_count: 1,
            },
            restrictions: None,
            allow_force_pushes: false,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"strict\":true"));
        assert!(json.contains("\"contexts\":[\"ci/build\"]"));
        assert!(json.contains("\"enforce_admins\":false"));
        assert!(json.contains("\"dismiss_stale_reviews\":true"));
        assert!(json.contains("\"required_approving_review_count\":1"));
        assert!(json.contains("\"restrictions\":null"));
        assert!(json.contains("\"allow_force_pushes\":false"));
    }

    #[test]
    fn test_protection_body_serialization_no_ci() {
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
                contexts: vec![],
            },
            enforce_admins: false,
            required_pull_request_reviews: Reviews {
                dismiss_stale_reviews: true,
                required_approving_review_count: 1,
            },
            restrictions: None,
            allow_force_pushes: false,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"contexts\":[]"));
    }

    #[test]
    fn test_protection_body_multiple_ci_checks() {
        #[derive(Serialize)]
        struct RequiredChecks<'a> {
            strict: bool,
            contexts: Vec<&'a str>,
        }

        let checks = RequiredChecks {
            strict: true,
            contexts: vec!["ci/format", "ci/lint", "ci/test"],
        };
        let json = serde_json::to_string(&checks).unwrap();
        assert!(json.contains("\"ci/format\""));
        assert!(json.contains("\"ci/lint\""));
        assert!(json.contains("\"ci/test\""));
    }

    #[test]
    fn test_reviews_serialization_various_counts() {
        #[derive(Serialize)]
        struct Reviews {
            dismiss_stale_reviews: bool,
            required_approving_review_count: u8,
        }

        for count in 0..=6 {
            let reviews = Reviews {
                dismiss_stale_reviews: false,
                required_approving_review_count: count,
            };
            let json = serde_json::to_string(&reviews).unwrap();
            assert!(json.contains(&format!("\"required_approving_review_count\":{count}")));
        }
    }
}
