use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

#[derive(Deserialize)]
pub struct User {
    pub login: String,
}

#[derive(Deserialize)]
pub struct Org {
    pub login: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub struct Repo {
    pub full_name: String,
    pub html_url: String,
    pub default_branch: String,
    pub topics: Option<Vec<String>>,
}

#[derive(Serialize)]
struct CreateRepoBody<'a> {
    name: &'a str,
    description: &'a str,
    private: bool,
    auto_init: bool,
}

pub fn get_user(client: &GithubClient) -> Result<User> {
    client.get("/user")
}

pub fn list_orgs(client: &GithubClient) -> Result<Vec<Org>> {
    client.get("/user/orgs")
}

pub fn create_repo(
    client: &GithubClient,
    owner: &str,
    name: &str,
    description: &str,
    private: bool,
    is_org: bool,
) -> Result<Repo> {
    let body = CreateRepoBody {
        name,
        description,
        private,
        auto_init: true,
    };
    let path = if is_org {
        format!("/orgs/{owner}/repos")
    } else {
        "/user/repos".to_string()
    };
    client
        .post(&path, &body)
        .with_context(|| format!("Failed to create repo '{owner}/{name}' — does it already exist?"))
}

pub fn get_gitignore_template(client: &GithubClient, name: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Template {
        source: String,
    }
    let t: Template = client.get(&format!("/gitignore/templates/{name}"))?;
    Ok(t.source)
}

/// Fetch the full license text from GitHub's license API (e.g. key "mit",
/// "apache-2.0", "gpl-3.0"). The body carries placeholders like `[year]` and
/// `[fullname]` that the caller fills in.
pub fn get_license_template(client: &GithubClient, key: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct License {
        body: String,
    }
    let l: License = client.get(&format!("/licenses/{key}"))?;
    Ok(l.body)
}

pub fn get_repo(client: &GithubClient, owner: &str, name: &str) -> Result<Repo> {
    client.get(&format!("/repos/{owner}/{name}"))
}

pub fn set_topics(client: &GithubClient, owner: &str, name: &str, topics: &[String]) -> Result<()> {
    #[derive(Serialize)]
    struct TopicsBody {
        names: Vec<String>,
    }
    let body = TopicsBody {
        names: topics.to_vec(),
    };
    let _: serde_json::Value = client.put(&format!("/repos/{owner}/{name}/topics"), &body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_deserialize() {
        let json = r#"{"login":"octocat"}"#;
        let user: User = serde_json::from_str(json).unwrap();
        assert_eq!(user.login, "octocat");
    }

    #[test]
    fn test_user_deserialize_extra_fields() {
        let json = r#"{"login":"octocat","id":1,"node_id":"MDQ6VXNlcjE=","avatar_url":"https://example.com/avatar.png"}"#;
        let user: User = serde_json::from_str(json).unwrap();
        assert_eq!(user.login, "octocat");
    }

    #[test]
    fn test_org_deserialize() {
        let json = r#"{"login":"my-org"}"#;
        let org: Org = serde_json::from_str(json).unwrap();
        assert_eq!(org.login, "my-org");
    }

    #[test]
    fn test_org_deserialize_extra_fields() {
        let json = r#"{"login":"my-org","id":42,"description":"Test org"}"#;
        let org: Org = serde_json::from_str(json).unwrap();
        assert_eq!(org.login, "my-org");
    }

    #[test]
    fn test_repo_deserialize() {
        let json = r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main","topics":["rust","cli"]}"#;
        let repo: Repo = serde_json::from_str(json).unwrap();
        assert_eq!(repo.full_name, "owner/repo");
        assert_eq!(repo.html_url, "https://github.com/owner/repo");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.topics.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_repo_deserialize_no_topics() {
        let json = r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main","topics":null}"#;
        let repo: Repo = serde_json::from_str(json).unwrap();
        assert!(repo.topics.is_none());
    }

    #[test]
    fn test_repo_deserialize_missing_topics_field() {
        let json = r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main"}"#;
        let repo: Repo = serde_json::from_str(json).unwrap();
        assert!(repo.topics.is_none());
    }

    #[test]
    fn test_repo_deserialize_empty_topics() {
        let json = r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main","topics":[]}"#;
        let repo: Repo = serde_json::from_str(json).unwrap();
        assert_eq!(repo.topics.unwrap().len(), 0);
    }

    #[test]
    fn test_create_repo_body_user_serialization() {
        #[derive(Serialize)]
        struct CreateRepoBody<'a> {
            name: &'a str,
            description: &'a str,
            private: bool,
            auto_init: bool,
        }
        let body = CreateRepoBody {
            name: "my-repo",
            description: "A test repo",
            private: true,
            auto_init: true,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"name\":\"my-repo\""));
        assert!(json.contains("\"description\":\"A test repo\""));
        assert!(json.contains("\"private\":true"));
        assert!(json.contains("\"auto_init\":true"));
    }

    #[test]
    fn test_create_repo_body_org_serialization() {
        #[derive(Serialize)]
        struct CreateRepoBody<'a> {
            name: &'a str,
            description: &'a str,
            private: bool,
            auto_init: bool,
        }
        let body = CreateRepoBody {
            name: "org-repo",
            description: "Org repo",
            private: false,
            auto_init: true,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"name\":\"org-repo\""));
        assert!(json.contains("\"private\":false"));
    }

    #[test]
    fn test_topics_body_serialization() {
        #[derive(Serialize)]
        struct TopicsBody {
            names: Vec<String>,
        }
        let body = TopicsBody {
            names: vec!["rust".to_string(), "cli".to_string()],
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"names\":[\"rust\",\"cli\"]"));
    }

    #[test]
    fn test_topics_body_empty() {
        #[derive(Serialize)]
        struct TopicsBody {
            names: Vec<String>,
        }
        let body = TopicsBody { names: vec![] };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"names\":[]"));
    }

    #[test]
    fn test_gitignore_template_deserialize() {
        let json = r#"{"source":"Dependency directories node_modules/"}"#;
        #[derive(Deserialize)]
        struct Template {
            source: String,
        }
        let t: Template = serde_json::from_str(json).unwrap();
        assert!(t.source.contains("node_modules/"));
    }

    #[test]
    fn test_license_template_deserialize() {
        let json = r#"{"body":"MIT License - Copyright (c) [year] [fullname]"}"#;
        #[derive(Deserialize)]
        struct License {
            body: String,
        }
        let l: License = serde_json::from_str(json).unwrap();
        assert!(l.body.contains("[year]"));
        assert!(l.body.contains("[fullname]"));
    }

    #[test]
    fn test_repo_deserialize_real_github_response() {
        let json = r#"{
            "full_name": "UniverLab/ghscaff",
            "html_url": "https://github.com/UniverLab/ghscaff",
            "default_branch": "main",
            "topics": ["rust", "cli", "github", "scaffold"]
        }"#;
        let repo: Repo = serde_json::from_str(json).unwrap();
        assert_eq!(repo.full_name, "UniverLab/ghscaff");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.topics.unwrap().len(), 4);
    }

    #[test]
    fn test_user_deserialize_minimal() {
        let json = r#"{"login":"minimal-user"}"#;
        let user: User = serde_json::from_str(json).unwrap();
        assert_eq!(user.login, "minimal-user");
    }

    #[test]
    fn test_org_deserialize_minimal() {
        let json = r#"{"login":"minimal-org"}"#;
        let org: Org = serde_json::from_str(json).unwrap();
        assert_eq!(org.login, "minimal-org");
    }

    #[test]
    fn test_user_deserialize_unicode_login() {
        let json = r#"{"login":"user-日本語"}"#;
        let user: User = serde_json::from_str(json).unwrap();
        assert_eq!(user.login, "user-日本語");
    }

    #[test]
    fn test_org_deserialize_unicode_login() {
        let json = r#"{"login":"org-日本語"}"#;
        let org: Org = serde_json::from_str(json).unwrap();
        assert_eq!(org.login, "org-日本語");
    }

    #[test]
    fn test_repo_deserialize_with_long_description() {
        let json = r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main","topics":["a","b","c","d","e","f","g","h","i","j"]}"#;
        let repo: Repo = serde_json::from_str(json).unwrap();
        assert_eq!(repo.topics.unwrap().len(), 10);
    }

    #[test]
    fn test_create_repo_body_private_false() {
        #[derive(Serialize)]
        struct CreateRepoBody<'a> {
            name: &'a str,
            description: &'a str,
            private: bool,
            auto_init: bool,
        }
        let body = CreateRepoBody {
            name: "public-repo",
            description: "Public",
            private: false,
            auto_init: true,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"private\":false"));
        assert!(json.contains("\"auto_init\":true"));
    }

    #[test]
    fn test_create_repo_body_empty_description() {
        #[derive(Serialize)]
        struct CreateRepoBody<'a> {
            name: &'a str,
            description: &'a str,
            private: bool,
            auto_init: bool,
        }
        let body = CreateRepoBody {
            name: "repo",
            description: "",
            private: false,
            auto_init: true,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"description\":\"\""));
    }

    #[test]
    fn test_topics_body_many_topics() {
        #[derive(Serialize)]
        struct TopicsBody {
            names: Vec<String>,
        }
        let body = TopicsBody {
            names: (0..20).map(|i| format!("topic{i}")).collect(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("topic0"));
        assert!(json.contains("topic19"));
    }

    #[test]
    fn test_repo_deserialize_default_branch_variations() {
        for branch in &["main", "master", "develop", "dev", "production"] {
            let json = format!(
                r#"{{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"{}"}}"#,
                branch
            );
            let repo: Repo = serde_json::from_str(&json).unwrap();
            assert_eq!(repo.default_branch, *branch);
        }
    }

    #[test]
    fn test_gitignore_template_empty_source() {
        let json = r#"{"source":""}"#;
        #[derive(Deserialize)]
        struct Template {
            source: String,
        }
        let t: Template = serde_json::from_str(json).unwrap();
        assert!(t.source.is_empty());
    }

    #[test]
    fn test_license_template_empty_body() {
        let json = r#"{"body":""}"#;
        #[derive(Deserialize)]
        struct License {
            body: String,
        }
        let l: License = serde_json::from_str(json).unwrap();
        assert!(l.body.is_empty());
    }

    #[test]
    fn test_user_deserialize_special_characters() {
        let json = r#"{"login":"user-with-dots.and-dashes"}"#;
        let user: User = serde_json::from_str(json).unwrap();
        assert_eq!(user.login, "user-with-dots.and-dashes");
    }

    #[test]
    fn test_repo_deserialize_long_full_name() {
        let full_name = format!("{}/{}", "a".repeat(39), "b".repeat(100));
        let json = format!(
            r#"{{"full_name":"{}","html_url":"https://example.com","default_branch":"main"}}"#,
            full_name
        );
        let repo: Repo = serde_json::from_str(&json).unwrap();
        assert_eq!(repo.full_name, full_name);
    }

    #[test]
    fn test_create_repo_body_special_chars_in_name() {
        #[derive(Serialize)]
        struct CreateRepoBody<'a> {
            name: &'a str,
            description: &'a str,
            private: bool,
            auto_init: bool,
        }
        let body = CreateRepoBody {
            name: "my-special-repo_v2.0",
            description: "Repo with special chars: !@#$%",
            private: true,
            auto_init: false,
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("my-special-repo_v2.0"));
        assert!(json.contains("\"auto_init\":false"));
    }

    #[test]
    fn test_repo_deserialize_many_topics() {
        let topics: Vec<String> = (0..50).map(|i| format!("topic{i}")).collect();
        let topics_json = topics
            .iter()
            .map(|t| format!("\"{}\"", t))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{"full_name":"o/r","html_url":"https://example.com","default_branch":"main","topics":[{}]}}"#,
            topics_json
        );
        let repo: Repo = serde_json::from_str(&json).unwrap();
        assert_eq!(repo.topics.unwrap().len(), 50);
    }

    // ── Mock-based integration tests ──────────────────────────────

    use super::super::test_utils::{mock_client, start_mock_server};

    #[test]
    fn get_user_returns_user() {
        let url = start_mock_server(|path| {
            if path == "/user" {
                (200, r#"{"login":"octocat"}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let user = get_user(&client).unwrap();
        assert_eq!(user.login, "octocat");
    }

    #[test]
    fn list_orgs_returns_orgs() {
        let url = start_mock_server(|path| {
            if path == "/user/orgs" {
                (200, r#"[{"login":"org1"},{"login":"org2"}]"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let orgs = list_orgs(&client).unwrap();
        assert_eq!(orgs.len(), 2);
        assert_eq!(orgs[0].login, "org1");
        assert_eq!(orgs[1].login, "org2");
    }

    #[test]
    fn get_repo_returns_repo() {
        let url = start_mock_server(|path| {
            if path == "/repos/owner/repo" {
                (200, r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main","topics":["rust"]}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let repo = get_repo(&client, "owner", "repo").unwrap();
        assert_eq!(repo.full_name, "owner/repo");
        assert_eq!(repo.default_branch, "main");
    }

    #[test]
    fn create_user_repo_succeeds() {
        let url = start_mock_server(|path| {
            if path == "/user/repos" {
                (201, r#"{"full_name":"user/repo","html_url":"https://github.com/user/repo","default_branch":"main","topics":[]}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let repo = create_repo(&client, "user", "repo", "Test", false, false).unwrap();
        assert_eq!(repo.full_name, "user/repo");
    }

    #[test]
    fn create_org_repo_succeeds() {
        let url = start_mock_server(|path| {
            if path == "/orgs/myorg/repos" {
                (201, r#"{"full_name":"myorg/repo","html_url":"https://github.com/myorg/repo","default_branch":"main","topics":[]}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let repo = create_repo(&client, "myorg", "repo", "Org repo", true, true).unwrap();
        assert_eq!(repo.full_name, "myorg/repo");
    }

    #[test]
    fn set_topics_succeeds() {
        let url = start_mock_server(|path| {
            if path.contains("/topics") {
                (200, r#"{"names":["rust","cli"]}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let result = set_topics(&client, "owner", "repo", &["rust".to_string(), "cli".to_string()]);
        assert!(result.is_ok());
    }

    #[test]
    fn get_gitignore_template_succeeds() {
        let url = start_mock_server(|path| {
            if path.contains("/gitignore/templates/") {
                (200, r#"{"source":"Dependency directories\nnode_modules/"}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let template = get_gitignore_template(&client, "Node").unwrap();
        assert!(template.contains("node_modules/"));
    }

    #[test]
    fn get_license_template_succeeds() {
        let url = start_mock_server(|path| {
            if path.contains("/licenses/") {
                (200, r#"{"body":"MIT License\nCopyright (c) [year] [fullname]"}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let client = mock_client(&url);
        let license = get_license_template(&client, "mit").unwrap();
        assert!(license.contains("[year]"));
    }
}
