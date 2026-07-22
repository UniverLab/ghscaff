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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_file_struct() {
        let file = TreeFile {
            path: "src/main.rs".to_string(),
            content: "fn main() {}".to_string(),
        };
        assert_eq!(file.path, "src/main.rs");
        assert_eq!(file.content, "fn main() {}");
    }

    #[test]
    fn test_blob_body_serialization() {
        let body = BlobBody {
            content: "hello world".to_string(),
            encoding: "utf-8".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"content\":\"hello world\""));
        assert!(json.contains("\"encoding\":\"utf-8\""));
    }

    #[test]
    fn test_blob_response_deserialize() {
        let json = r#"{"sha":"abc123","node_id":"MDQ6QmxvYjEyMw=="}"#;
        let resp: BlobResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.sha, "abc123");
    }

    #[test]
    fn test_tree_item_body_serialization() {
        let item = TreeItemBody {
            path: "README.md".to_string(),
            mode: "100644".to_string(),
            item_type: "blob".to_string(),
            sha: "abc123".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"path\":\"README.md\""));
        assert!(json.contains("\"mode\":\"100644\""));
        assert!(json.contains("\"type\":\"blob\""));
        assert!(json.contains("\"sha\":\"abc123\""));
    }

    #[test]
    fn test_tree_item_body_executable_mode() {
        let item = TreeItemBody {
            path: "run.sh".to_string(),
            mode: "100755".to_string(),
            item_type: "blob".to_string(),
            sha: "def456".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"mode\":\"100755\""));
    }

    #[test]
    fn test_tree_item_body_directory_type() {
        let item = TreeItemBody {
            path: "src".to_string(),
            mode: "040000".to_string(),
            item_type: "tree".to_string(),
            sha: "tree_sha_123".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"type\":\"tree\""));
    }

    #[test]
    fn test_create_tree_body_with_base() {
        let body = CreateTreeBody {
            tree: vec![
                TreeItemBody {
                    path: "a.txt".to_string(),
                    mode: "100644".to_string(),
                    item_type: "blob".to_string(),
                    sha: "sha_a".to_string(),
                },
                TreeItemBody {
                    path: "b.txt".to_string(),
                    mode: "100644".to_string(),
                    item_type: "blob".to_string(),
                    sha: "sha_b".to_string(),
                },
            ],
            base_tree: Some("base_sha_789".to_string()),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"base_tree\":\"base_sha_789\""));
        assert!(json.contains("\"a.txt\""));
        assert!(json.contains("\"b.txt\""));
    }

    #[test]
    fn test_create_tree_body_no_base() {
        let body = CreateTreeBody {
            tree: vec![],
            base_tree: None,
        };
        let json = serde_json::to_string(&body).unwrap();
        // base_tree should be skipped when None
        assert!(!json.contains("base_tree"));
        assert!(json.contains("\"tree\":[]"));
    }

    #[test]
    fn test_tree_response_deserialize() {
        let json = r#"{"sha":"tree_sha_abc","truncated":false}"#;
        let resp: TreeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.sha, "tree_sha_abc");
    }

    #[test]
    fn test_commit_info_deserialize_with_tree() {
        let json = r#"{"sha":"commit_sha_123","tree":{"sha":"tree_sha_456"}}"#;
        let info: CommitInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.sha, "commit_sha_123");
        assert_eq!(info.tree.unwrap().sha, "tree_sha_456");
    }

    #[test]
    fn test_commit_info_deserialize_without_tree() {
        let json = r#"{"sha":"commit_sha_789"}"#;
        let info: CommitInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.sha, "commit_sha_789");
        assert!(info.tree.is_none());
    }

    #[test]
    fn test_commit_tree_info_deserialize() {
        let json = r#"{"sha":"tree_abc"}"#;
        let info: CommitTreeInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.sha, "tree_abc");
    }

    #[test]
    fn test_create_commit_body_serialization() {
        let body = CreateCommitBody {
            message: "chore: init repository".to_string(),
            tree: "tree_sha_123".to_string(),
            parents: vec!["parent_sha_1".to_string(), "parent_sha_2".to_string()],
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"message\":\"chore: init repository\""));
        assert!(json.contains("\"tree\":\"tree_sha_123\""));
        assert!(json.contains("\"parents\":[\"parent_sha_1\",\"parent_sha_2\"]"));
    }

    #[test]
    fn test_create_commit_body_no_parents() {
        let body = CreateCommitBody {
            message: "initial commit".to_string(),
            tree: "first_tree".to_string(),
            parents: vec![],
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"parents\":[]"));
    }

    #[test]
    fn test_create_ref_body_serialization() {
        let body = CreateRefBody {
            ref_name: "refs/heads/main".to_string(),
            sha: "abc123".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ref\":\"refs/heads/main\""));
        assert!(json.contains("\"sha\":\"abc123\""));
    }

    #[test]
    fn test_create_ref_body_nested_branch() {
        let body = CreateRefBody {
            ref_name: "refs/heads/feature/my-feature".to_string(),
            sha: "def456".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ref\":\"refs/heads/feature/my-feature\""));
    }

    #[test]
    fn test_update_ref_body_serialization() {
        let body = UpdateRefBody {
            sha: "new_sha_abc".to_string(),
            force: false,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"sha\":\"new_sha_abc\""));
        assert!(json.contains("\"force\":false"));
    }

    #[test]
    fn test_update_ref_body_force() {
        let body = UpdateRefBody {
            sha: "force_sha".to_string(),
            force: true,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"force\":true"));
    }

    #[test]
    fn test_blob_response_full_github_json() {
        let json = r#"{
            "sha": "20b2aa843a850c8e09a4f9d5e8f7c0e8a5e3d2f1",
            "size": 42,
            "encoding": "base64",
            "content": "aGVsbG8gd29ybGQ=",
            "node_id": "MDQ6QmxvYjEyMw=="
        }"#;
        let resp: BlobResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.sha, "20b2aa843a850c8e09a4f9d5e8f7c0e8a5e3d2f1");
    }

    #[test]
    fn test_tree_item_body_real_github_mode() {
        let item = TreeItemBody {
            path: ".github/workflows/ci.yml".to_string(),
            mode: "100644".to_string(),
            item_type: "blob".to_string(),
            sha: "cafe".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["path"], ".github/workflows/ci.yml");
        assert_eq!(parsed["mode"], "100644");
        assert_eq!(parsed["type"], "blob");
    }

    #[test]
    fn test_tree_file_empty_content() {
        let file = TreeFile {
            path: ".gitkeep".to_string(),
            content: String::new(),
        };
        assert!(file.content.is_empty());
    }

    #[test]
    fn test_tree_file_path_with_spaces() {
        let file = TreeFile {
            path: "docs/my file.md".to_string(),
            content: "# Hello".to_string(),
        };
        assert_eq!(file.path, "docs/my file.md");
    }

    #[test]
    fn test_blob_body_empty_content() {
        let body = BlobBody {
            content: String::new(),
            encoding: "utf-8".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"content\":\"\""));
    }

    #[test]
    fn test_create_tree_body_single_file() {
        let body = CreateTreeBody {
            tree: vec![TreeItemBody {
                path: "single.txt".to_string(),
                mode: "100644".to_string(),
                item_type: "blob".to_string(),
                sha: "sha1".to_string(),
            }],
            base_tree: None,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"single.txt\""));
        assert!(!json.contains("base_tree"));
    }

    #[test]
    fn test_create_tree_body_many_files() {
        let items: Vec<TreeItemBody> = (0..50)
            .map(|i| TreeItemBody {
                path: format!("file_{}.txt", i),
                mode: "100644".to_string(),
                item_type: "blob".to_string(),
                sha: format!("sha_{}", i),
            })
            .collect();
        let body = CreateTreeBody {
            tree: items,
            base_tree: None,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"file_0.txt\""));
        assert!(json.contains("\"file_49.txt\""));
    }

    #[test]
    fn test_commit_info_deserialize_full_github() {
        let json = r#"{
            "sha": "abc123",
            "url": "https://api.github.com/repos/owner/repo/git/commits/abc123",
            "author": {"name": "test", "email": "test@test.com", "date": "2024-01-01"},
            "committer": {"name": "test", "email": "test@test.com", "date": "2024-01-01"},
            "tree": {"sha": "tree456", "url": "https://api.github.com/repos/owner/repo/git/trees/tree456"},
            "message": "init"
        }"#;
        let info: CommitInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.sha, "abc123");
        assert_eq!(info.tree.unwrap().sha, "tree456");
    }

    #[test]
    fn test_tree_response_full_github() {
        let json = r#"{
            "sha": "tree_sha_abc",
            "url": "https://api.github.com/repos/owner/repo/git/trees/tree_sha_abc",
            "truncated": false,
            "tree": []
        }"#;
        let resp: TreeResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.sha, "tree_sha_abc");
    }

    #[test]
    fn test_create_ref_body_tag() {
        let body = CreateRefBody {
            ref_name: "refs/tags/v1.0.0".to_string(),
            sha: "tag_sha".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"ref\":\"refs/tags/v1.0.0\""));
    }

    #[test]
    fn test_tree_item_body_symlink() {
        let item = TreeItemBody {
            path: "link".to_string(),
            mode: "120000".to_string(),
            item_type: "blob".to_string(),
            sha: "symlink_sha".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"mode\":\"120000\""));
    }

    #[test]
    fn test_tree_item_body_submodule() {
        let item = TreeItemBody {
            path: "vendor/lib".to_string(),
            mode: "160000".to_string(),
            item_type: "commit".to_string(),
            sha: "commit_sha".to_string(),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"type\":\"commit\""));
        assert!(json.contains("\"mode\":\"160000\""));
    }

    #[test]
    fn test_blob_response_deserialize_minimal() {
        let json = r#"{"sha":"simple_sha"}"#;
        let resp: BlobResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.sha, "simple_sha");
    }

    #[test]
    fn test_commit_info_deserialize_empty_parents() {
        let json = r#"{"sha":"first_commit","tree":{"sha":"t1"}}"#;
        let info: CommitInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.sha, "first_commit");
    }

    #[test]
    fn test_update_ref_body_various_shas() {
        for sha in &["abc", "da39a3ee5e6b4b0d3255bfef95601890afd80709", "x"] {
            let body = UpdateRefBody {
                sha: sha.to_string(),
                force: false,
            };
            let json = serde_json::to_string(&body).unwrap();
            assert!(json.contains(&format!("\"sha\":\"{}\"", sha)));
        }
    }

    #[test]
    fn test_tree_file_unicode_content() {
        let file = TreeFile {
            path: "unicode.txt".to_string(),
            content: "你好世界 🌍".to_string(),
        };
        assert!(file.content.contains("你好"));
    }

    #[test]
    fn test_tree_file_long_path() {
        let path = format!("src/{}/file.rs", "a/".repeat(10));
        let file = TreeFile {
            path,
            content: "code".to_string(),
        };
        assert!(file.path.contains("a/"));
    }
}
