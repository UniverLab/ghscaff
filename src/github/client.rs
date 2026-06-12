use anyhow::{Context, Result};
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;

pub struct GithubClient {
    client: Client,
    token: String,
}

fn check_status(resp: Response) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let path = resp.url().path().to_string();
    let body = resp.text().unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v["message"].as_str().map(String::from))
        .unwrap_or(body);
    let api_path = path.strip_prefix('/').unwrap_or(&path);
    anyhow::bail!("{status} — {message}\n  API: {api_path}")
}

impl GithubClient {
    pub fn new(token: &str) -> Self {
        Self {
            client: Client::new(),
            token: token.to_owned(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        let url = format!("https://api.github.com{path}");
        self.client
            .request(method, &url)
            .header("Authorization", format!("token {}", self.token))
            .header("User-Agent", "ghscaff")
            .header("Accept", "application/vnd.github+json")
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .request(reqwest::Method::GET, path)
            .send()
            .context("HTTP GET failed")?;
        check_status(resp)?
            .json()
            .context("Failed to parse response")
    }

    pub fn post<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .request(reqwest::Method::POST, path)
            .json(body)
            .send()
            .context("HTTP POST failed")?;
        check_status(resp)?
            .json()
            .context("Failed to parse response")
    }

    pub fn put<B: serde::Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let resp = self
            .request(reqwest::Method::PUT, path)
            .json(body)
            .send()
            .context("HTTP PUT failed")?;
        check_status(resp)?
            .json()
            .context("Failed to parse response")
    }

    pub fn put_no_response<B: serde::Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let resp = self
            .request(reqwest::Method::PUT, path)
            .json(body)
            .send()
            .context("HTTP PUT failed")?;
        check_status(resp)?;
        Ok(())
    }

    pub fn patch<B: serde::Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .request(reqwest::Method::PATCH, path)
            .json(body)
            .send()
            .context("HTTP PATCH failed")?;
        check_status(resp)?
            .json()
            .context("Failed to parse response")
    }

    pub fn delete(&self, path: &str) -> Result<()> {
        let resp = self
            .request(reqwest::Method::DELETE, path)
            .send()
            .context("HTTP DELETE failed")?;
        check_status(resp)?;
        Ok(())
    }

    pub fn validate_scopes(&self) -> Result<()> {
        let user = self.get::<serde_json::Value>("/user")?;

        let message = user
            .get("message")
            .and_then(|m| m.as_str());

        if let Some(msg) = message {
            if msg.contains("API rate limit") {
                anyhow::bail!("GitHub API rate limit exceeded");
            } else if msg.contains("Bad credentials") {
                anyhow::bail!("GitHub token is invalid or expired");
            }
        }

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
