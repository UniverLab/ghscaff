use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

pub struct TreeFile {
    pub path: String,
    pub content: String,
}

#[derive(Serialize)]
struct BlobBody {
    content: String,
    encoding: String,
}

#[derive(Deserialize)]
struct BlobResponse {
    sha: String,
}

#[derive(Serialize)]
struct TreeItemBody {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    item_type: String,
    sha: String,
}

#[derive(Serialize)]
struct CreateTreeBody {
    tree: Vec<TreeItemBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_tree: Option<String>,
}

#[derive(Deserialize)]
struct TreeResponse {
    sha: String,
}

#[derive(Deserialize)]
struct CommitTreeInfo {
    sha: String,
}

#[derive(Deserialize)]
struct CommitInfo {
    sha: String,
    #[serde(default)]
    tree: Option<CommitTreeInfo>,
}

#[derive(Serialize)]
struct CreateCommitBody {
    message: String,
    tree: String,
    parents: Vec<String>,
}

#[derive(Serialize)]
struct CreateRefBody {
    #[serde(rename = "ref")]
    ref_name: String,
    sha: String,
}

#[derive(Serialize)]
struct UpdateRefBody {
    sha: String,
    force: bool,
}

fn get_branch_sha_opt(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Option<String> {
    #[derive(Deserialize)]
    struct Ref {
        object: RefObject,
    }
    #[derive(Deserialize)]
    struct RefObject {
        sha: String,
    }
    let path = format!("/repos/{owner}/{repo}/git/refs/heads/{branch}");
    client.get::<Ref>(&path).ok().map(|r| r.object.sha)
}

/// Commit all `files` in a single git commit using the Trees API.
/// Works on empty repos (creates the initial ref) and on existing branches.
/// Returns the new commit SHA.
pub fn create_tree_commit(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    files: &[TreeFile],
    message: &str,
    branch: &str,
) -> Result<String> {
    // GitHub initializes the git database asynchronously after repo creation.
    // The ref may exist before the Git Database API is ready, so we verify
    // by attempting a lightweight API call that requires the git DB.
    const READY_DELAYS_MS: &[u64] = &[1000, 2000, 3000, 5000, 8000];
    for &delay_ms in READY_DELAYS_MS {
        if get_branch_sha_opt(client, owner, repo, branch).is_some() {
            // Verify git DB is actually ready by checking the commit is fetchable
            let sha = get_branch_sha_opt(client, owner, repo, branch).unwrap();
            if client
                .get::<serde_json::Value>(&format!("/repos/{owner}/{repo}/git/commits/{sha}"))
                .is_ok()
            {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }

    // 1. Create a blob for each file
    let mut tree_items: Vec<TreeItemBody> = Vec::with_capacity(files.len());
    for file in files {
        let blob: BlobResponse = client
            .post(
                &format!("/repos/{owner}/{repo}/git/blobs"),
                &BlobBody {
                    content: file.content.clone(),
                    encoding: "utf-8".to_string(),
                },
            )
            .with_context(|| format!("Failed to create blob for '{}'", file.path))?;
        tree_items.push(TreeItemBody {
            path: file.path.clone(),
            mode: "100644".to_string(),
            item_type: "blob".to_string(),
            sha: blob.sha,
        });
    }

    // 2. Resolve parent commit + base tree (None for empty repo)
    let (parent_sha, base_tree) =
        if let Some(commit_sha) = get_branch_sha_opt(client, owner, repo, branch) {
            let commit: CommitInfo = client
                .get(&format!("/repos/{owner}/{repo}/git/commits/{commit_sha}"))
                .context("Failed to fetch parent commit")?;
            let tree_sha = commit.tree.map(|t| t.sha);
            (Some(commit_sha), tree_sha)
        } else {
            (None, None)
        };

    // 3. Create tree
    let tree: TreeResponse = client
        .post(
            &format!("/repos/{owner}/{repo}/git/trees"),
            &CreateTreeBody {
                tree: tree_items,
                base_tree,
            },
        )
        .context("Failed to create git tree")?;

    // 4. Create commit
    let parents: Vec<String> = parent_sha.into_iter().collect();
    let commit: CommitInfo = client
        .post(
            &format!("/repos/{owner}/{repo}/git/commits"),
            &CreateCommitBody {
                message: message.to_string(),
                tree: tree.sha,
                parents: parents.clone(),
            },
        )
        .context("Failed to create git commit")?;

    let commit_sha = commit.sha;

    // 5. Create or update the branch ref
    if parents.is_empty() {
        client
            .post::<_, serde_json::Value>(
                &format!("/repos/{owner}/{repo}/git/refs"),
                &CreateRefBody {
                    ref_name: format!("refs/heads/{branch}"),
                    sha: commit_sha.clone(),
                },
            )
            .context("Failed to create branch ref")?;
    } else {
        client
            .patch::<_, serde_json::Value>(
                &format!("/repos/{owner}/{repo}/git/refs/heads/{branch}"),
                &UpdateRefBody {
                    sha: commit_sha.clone(),
                    force: false,
                },
            )
            .context("Failed to update branch ref")?;
    }

    Ok(commit_sha)
}
