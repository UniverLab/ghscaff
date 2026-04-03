use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;

pub struct GithubClient {
    client: Client,
    token: String,
}

impl GithubClient {
    pub fn new(token: &str) -> Self {
        Self {
            client: Client::new(),
            token: token.to_owned(),
        }
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("https://api.github.com{path}");
        self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "ghscaff")
            .header("Accept", "application/vnd.github+json")
            .send()
            .context("HTTP GET failed")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse response")
    }

    pub fn post<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("https://api.github.com{path}");
        self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "ghscaff")
            .header("Accept", "application/vnd.github+json")
            .json(body)
            .send()
            .context("HTTP POST failed")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse response")
    }

    pub fn put<B: serde::Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let url = format!("https://api.github.com{path}");
        self.client
            .put(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "ghscaff")
            .header("Accept", "application/vnd.github+json")
            .json(body)
            .send()
            .context("HTTP PUT failed")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse response")
    }

    pub fn patch<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("https://api.github.com{path}");
        self.client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "ghscaff")
            .header("Accept", "application/vnd.github+json")
            .json(body)
            .send()
            .context("HTTP PATCH failed")?
            .error_for_status()
            .context("GitHub API error")?
            .json()
            .context("Failed to parse response")
    }
}

/// Read GITHUB_TOKEN from env. Fail fast with a clear message.
pub fn token_from_env() -> Result<String> {
    std::env::var("GITHUB_TOKEN").context(
        "GITHUB_TOKEN not set. Export your token:\n  export GITHUB_TOKEN=ghp_xxxxxxxxxxxx\nRequired scopes: repo, workflow"
    )
}
