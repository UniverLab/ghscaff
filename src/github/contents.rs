use anyhow::{Context, Result};
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

    // If a file already exists (e.g. README.md from auto_init), its SHA is required
    // for the update. Fetch it before writing.
    let existing_sha = get_file_sha(client, owner, repo, path);

    let body = Body {
        message,
        content: &encoded,
        sha: existing_sha,
    };

    let resp: Response = client
        .put(&endpoint, &body)
        .with_context(|| {
            format!(
                "Failed to create file '{path}'.\n\
                 Hint: if using a fine-grained PAT ensure 'Contents: Read and write' is granted.\n\
                 For org repos the token must also be authorised for the organisation."
            )
        })?;

    Ok(resp.commit.sha)
}
