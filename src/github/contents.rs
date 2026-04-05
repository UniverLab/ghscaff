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
    let body = Body { message, content: &encoded };
    let endpoint = format!("/repos/{owner}/{repo}/contents/{path}");

    // Retry up to 3 times with increasing delays.
    // GitHub occasionally returns 404 right after repo creation (especially org repos)
    // while the repository is still being initialised on their end.
    let mut last_err = anyhow::anyhow!("unreachable");
    for attempt in 0u64..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(1500 * attempt));
        }
        match client.put::<_, Response>(&endpoint, &body) {
            Ok(resp) => return Ok(resp.commit.sha),
            Err(e) => {
                let msg = e.to_string();
                // Only retry on 404 (timing) — any other status is a real error
                if !msg.contains("404") {
                    return Err(e).with_context(|| hint(path));
                }
                last_err = e;
            }
        }
    }
    Err(last_err).with_context(|| hint(path))
}

fn hint(path: &str) -> String {
    format!(
        "Failed to create file '{path}'.\n\
         Hint: if using a fine-grained PAT ensure 'Contents: Read and write' is granted.\n\
         For org repos the token must also be authorised for the organisation."
    )
}
