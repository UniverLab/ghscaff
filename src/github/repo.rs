use anyhow::Result;
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
    if is_org {
        client.post(&format!("/orgs/{owner}/repos"), &body)
    } else {
        client.post("/user/repos", &body)
    }
}

pub fn get_gitignore_template(client: &GithubClient, name: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Template {
        source: String,
    }
    let t: Template = client.get(&format!("/gitignore/templates/{name}"))?;
    Ok(t.source)
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
