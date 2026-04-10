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
            .header("Authorization", format!("token {}", self.token))
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
            .header("Authorization", format!("token {}", self.token))
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
            .header("Authorization", format!("token {}", self.token))
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

    pub fn put_no_response<B: serde::Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let url = format!("https://api.github.com{path}");
        self.client
            .put(&url)
            .header("Authorization", format!("token {}", self.token))
            .header("User-Agent", "ghscaff")
            .header("Accept", "application/vnd.github+json")
            .json(body)
            .send()
            .context("HTTP PUT failed")?
            .error_for_status()
            .context("GitHub API error")?;
        Ok(())
    }

    pub fn patch<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("https://api.github.com{path}");
        self.client
            .patch(&url)
            .header("Authorization", format!("token {}", self.token))
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

    pub fn delete(&self, path: &str) -> Result<()> {
        let url = format!("https://api.github.com{path}");
        self.client
            .delete(&url)
            .header("Authorization", format!("token {}", self.token))
            .header("User-Agent", "ghscaff")
            .header("Accept", "application/vnd.github+json")
            .send()
            .context("HTTP DELETE failed")?
            .error_for_status()
            .context("GitHub API error")?;
        Ok(())
    }
}

/// Env var → vault → inline prompt. Returns (token, vault_passphrase).
pub fn resolve_token() -> Result<(String, String)> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        return Ok((token, String::new()));
    }

    if let Some(pair) = crate::vault::resolve_github_token()? {
        return Ok(pair);
    }

    println!("  No GitHub token found.\n");
    crate::vault::prompt_and_save_github_token()
}
