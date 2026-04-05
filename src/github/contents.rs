use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

/// Returns the blob SHA of an existing file, or None if the file does not exist.
fn get_file_sha(client: &GithubClient, owner: &str, repo: &str, path: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct FileInfo {
        sha: String,
    }
    client
        .get::<FileInfo>(&format!("/repos/{owner}/{repo}/contents/{path}"))
        .ok()
        .map(|f| f.sha)
}

pub fn create_file(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    path: &str,
    content: &str,
    message: &str,
) -> Result<String> {
    #[derive(Serialize)]
    struct Body<'a> {
        message: &'a str,
        content: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        sha: Option<String>,
    }
    #[derive(Deserialize)]
    struct Response {
        commit: Commit,
    }
    #[derive(Deserialize)]
    struct Commit {
        sha: String,
    }

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(content);
    let endpoint = format!("/repos/{owner}/{repo}/contents/{path}");

    // GitHub's Contents API can return 404 briefly after repo creation due to
    // eventual consistency (even with auto_init: true). Retry with backoff.
    const DELAYS_MS: &[u64] = &[1500, 3000, 5000];

    let mut last_err = None;
    for (attempt, &delay_ms) in DELAYS_MS.iter().enumerate() {
        // Fetch SHA on every attempt so we always have the latest value.
        let existing_sha = get_file_sha(client, owner, repo, path);

        let body = Body {
            message,
            content: &encoded,
            sha: existing_sha,
        };

        match client.put::<Body, Response>(&endpoint, &body) {
            Ok(resp) => return Ok(resp.commit.sha),
            Err(e) => {
                // {:#} renders the full anyhow chain so we can detect the status code
                let full_msg = format!("{e:#}");
                // Only retry on 404; other errors (403, 422…) fail immediately.
                if !full_msg.contains("404") {
                    bail!(
                        "Failed to create file '{path}'.\n\
                         Hint: if using a fine-grained PAT ensure 'Contents: Read and write' is granted.\n\
                         For org repos the token must also be authorised for the organisation.\n\
                         Cause: {full_msg}"
                    );
                }
                if attempt + 1 < DELAYS_MS.len() {
                    std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                }
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap()).with_context(|| {
        format!(
            "Failed to create file '{path}' after {} attempts.\n\
             Hint: if using a fine-grained PAT ensure 'Contents: Read and write' is granted.\n\
             For org repos the token must also be authorised for the organisation.",
            DELAYS_MS.len()
        )
    })
}
