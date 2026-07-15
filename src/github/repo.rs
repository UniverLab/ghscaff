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
}
