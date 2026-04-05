use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

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
    let resp: Response = client.put(
        &format!("/repos/{owner}/{repo}/contents/{path}"),
        &Body {
            message,
            content: &encoded,
        },
    ).with_context(|| format!("Failed to create file '{path}'. If using a fine-grained PAT, ensure 'Contents: Read and write' permission is granted."))?;
    Ok(resp.commit.sha)
}
