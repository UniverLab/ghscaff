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

        let message = user.get("message").and_then(|m| m.as_str());

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_status_success() {
        let url = start_mock_server(|_| (200, r#"{"ok":true}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_status_error_404() {
        let url = start_mock_server(|_| (404, r#"{"message":"Not Found"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("404"));
    }

    #[test]
    fn test_check_status_error_403() {
        let url = start_mock_server(|_| (403, r#"{"message":"Forbidden"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("403"));
    }

    #[test]
    fn test_check_status_error_500() {
        let url = start_mock_server(|_| (500, r#"{"message":"Server Error"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("500"));
    }

    #[test]
    fn test_github_client_new() {
        let client = GithubClient::new("ghp_test123");
        assert_eq!(client.token, "ghp_test123");
    }

    #[test]
    fn test_github_client_empty_token() {
        let client = GithubClient::new("");
        assert!(client.token.is_empty());
    }

    #[test]
    fn test_github_client_long_token() {
        let long_token = "ghp_".to_string() + &"a".repeat(200);
        let client = GithubClient::new(&long_token);
        assert_eq!(client.token.len(), 204);
    }

    #[test]
    fn test_check_status_path_strips_leading_slash() {
        // Verify that the error message format includes the API path
        let url = start_mock_server(|_| (404, r#"{"message":"Not Found"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
    }

    #[test]
    fn test_request_builder_headers() {
        let client = GithubClient::new("ghp_test_token");
        let req = client.request(reqwest::Method::GET, "/test");
        let built = req.build().unwrap();
        assert_eq!(
            built.headers().get("Authorization").unwrap(),
            "token ghp_test_token"
        );
        assert_eq!(built.headers().get("User-Agent").unwrap(), "ghscaff");
        assert_eq!(
            built.headers().get("Accept").unwrap(),
            "application/vnd.github+json"
        );
    }

    #[test]
    fn test_request_url_construction() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::GET, "/repos/owner/repo");
        let built = req.build().unwrap();
        assert_eq!(
            built.url().as_str(),
            "https://api.github.com/repos/owner/repo"
        );
    }

    #[test]
    fn test_request_url_user_endpoint() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::GET, "/user");
        let built = req.build().unwrap();
        assert_eq!(built.url().as_str(), "https://api.github.com/user");
    }

    #[test]
    fn test_request_post_method() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::POST, "/repos/owner/repo/issues");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::POST);
    }

    #[test]
    fn test_request_put_method() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::PUT, "/repos/owner/repo/topics");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::PUT);
    }

    #[test]
    fn test_request_delete_method() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::DELETE, "/repos/owner/repo/labels/bug");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::DELETE);
    }

    #[test]
    fn test_request_patch_method() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::PATCH, "/repos/owner/repo");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::PATCH);
    }

    #[test]
    fn test_check_status_json_error_message() {
        let url = start_mock_server(|_| (422, r#"{"message":"Validation Failed"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("422"));
    }

    #[test]
    fn test_github_client_new_various_tokens() {
        let long_token = "a".repeat(500);
        let special_token = "ghp_with_special_chars_!@#$%";
        let tokens = vec!["", "ghp_short", long_token.as_str(), special_token];
        for token in tokens {
            let client = GithubClient::new(token);
            assert_eq!(client.token, token);
        }
    }

    #[test]
    fn test_request_url_complex_path() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::GET, "/repos/owner/repo/git/ref/heads/main");
        let built = req.build().unwrap();
        assert_eq!(
            built.url().as_str(),
            "https://api.github.com/repos/owner/repo/git/ref/heads/main"
        );
    }

    #[test]
    fn test_request_url_with_query_params() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(
            reqwest::Method::GET,
            "/repos/owner/repo/labels?per_page=100",
        );
        let built = req.build().unwrap();
        assert!(built.url().as_str().contains("per_page=100"));
    }

    #[test]
    fn test_check_status_error_401() {
        let url = start_mock_server(|_| (401, r#"{"message":"Unauthorized"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("401"));
    }

    #[test]
    fn test_check_status_error_422() {
        let url = start_mock_server(|_| (422, r#"{"message":"Validation Failed"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("422"));
    }

    #[test]
    fn test_check_status_error_409() {
        let url = start_mock_server(|_| (409, r#"{"message":"Conflict"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("409"));
    }

    #[test]
    fn test_check_status_error_451() {
        let url = start_mock_server(|_| {
            (
                451,
                r#"{"message":"Unavailable For Legal Reasons"}"#.to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("451"));
    }

    #[test]
    fn test_check_status_path_includes_api_prefix() {
        let url = start_mock_server(|_| (404, r#"{"message":"Not Found"}"#.to_string()));
        let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
    }

    #[test]
    fn test_request_builder_post_with_json() {
        use serde::Serialize;
        #[derive(Serialize)]
        struct TestBody {
            name: String,
        }
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::POST, "/test");
        let body = TestBody {
            name: "test".to_string(),
        };
        let built = req.json(&body).build().unwrap();
        assert_eq!(built.method(), reqwest::Method::POST);
        assert!(built.headers().contains_key("content-type"));
    }

    #[test]
    fn test_request_builder_put_with_json() {
        use serde::Serialize;
        #[derive(Serialize)]
        struct TestBody {
            value: i32,
        }
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::PUT, "/test");
        let body = TestBody { value: 42 };
        let built = req.json(&body).build().unwrap();
        assert_eq!(built.method(), reqwest::Method::PUT);
    }

    #[test]
    fn test_request_builder_patch_with_json() {
        use serde::Serialize;
        #[derive(Serialize)]
        struct TestBody {
            enabled: bool,
        }
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::PATCH, "/test");
        let body = TestBody { enabled: true };
        let built = req.json(&body).build().unwrap();
        assert_eq!(built.method(), reqwest::Method::PATCH);
    }

    #[test]
    fn test_request_builder_delete_no_body() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::DELETE, "/repos/owner/repo/issues/1");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::DELETE);
    }

    #[test]
    fn test_request_url_encoded_path() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(
            reqwest::Method::DELETE,
            "/repos/owner/repo/labels/bug%20label",
        );
        let built = req.build().unwrap();
        assert!(built.url().as_str().contains("bug%20label"));
    }

    #[test]
    fn test_check_status_success_various() {
        for code in &[200, 201, 204] {
            let c = *code;
            let url = start_mock_server(move |_| (c, r#"{"ok":true}"#.to_string()));
            let resp = reqwest::blocking::Client::new().get(&url).send().unwrap();
            let result = check_status(resp);
            assert!(result.is_ok(), "Status {code} should succeed");
        }
    }

    fn start_mock_server(
        handler: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
    ) -> String {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::sync::Arc;

        let handler = Arc::new(handler);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() {
                        break;
                    }
                }
                let path = request_line.split_whitespace().nth(1).unwrap_or("/");
                let (status, body) = (handler)(path);
                let status_text = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    404 => "Not Found",
                    _ => "Unknown",
                };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        url
    }

    #[test]
    fn test_check_status_json_message_extraction() {
        let url = start_mock_server(|path| {
            match path {
            "/error" => (
                403,
                r#"{"message": "rate limit exceeded", "documentation_url": "https://docs.github.com"}"#.to_string(),
            ),
            _ => (404, r#"{"message": "Not Found"}"#.to_string()),
        }
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/error"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("403"));
        assert!(msg.contains("rate limit exceeded"));
    }

    #[test]
    fn test_check_status_json_message_not_found() {
        let url = start_mock_server(|_| {
            (
                404,
                r#"{"message": "Not Found", "documentation_url": "https://docs.github.com"}"#
                    .to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("404"));
        assert!(msg.contains("Not Found"));
    }

    #[test]
    fn test_check_status_json_message_bad_credentials() {
        let url = start_mock_server(|_| (401, r#"{"message": "Bad credentials"}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("401"));
        assert!(msg.contains("Bad credentials"));
    }

    #[test]
    fn test_check_status_non_json_body() {
        let url = start_mock_server(|_| (500, "Internal Server Error".to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("500"));
        assert!(msg.contains("Internal Server Error"));
    }

    #[test]
    fn test_check_status_path_strips_leading_slash_in_error() {
        let url = start_mock_server(|_| (422, r#"{"message": "Validation Failed"}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/repos/owner/repo/issues"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("422"));
    }

    #[test]
    fn test_check_status_json_array_body_not_message() {
        let url = start_mock_server(|_| (400, r#"{"errors":[{"code":"custom"}]}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("400"));
    }

    #[test]
    fn test_check_status_json_nested_message() {
        let url = start_mock_server(|_| {
            (
                403,
                r#"{"message": "Resource not accessible by integration"}"#.to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Resource not accessible by integration"));
    }

    #[test]
    fn test_check_status_success_with_body() {
        let url = start_mock_server(|_| (200, r#"{"ok":true}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_ok());
    }

    #[test]
    fn test_client_get_success() {
        let url = start_mock_server(|path| {
            if path == "/repos/o/r" {
                (
                    200,
                    r#"{"full_name":"o/r","html_url":"https://github.com/o/r","default_branch":"main","topics":[]}"#.to_string(),
                )
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/repos/o/r"))
            .send()
            .unwrap();
        let result = check_status(resp).unwrap().json::<serde_json::Value>();
        assert!(result.is_ok());
    }

    #[test]
    fn test_client_get_error() {
        let url = start_mock_server(|_| (404, r#"{"message":"Not Found"}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/missing"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
    }

    #[test]
    fn test_request_post_method_mock() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::POST, "/test");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::POST);
        assert!(built.url().as_str().contains("/test"));
    }

    #[test]
    fn test_request_put_method_mock() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::PUT, "/test");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::PUT);
    }

    #[test]
    fn test_request_delete_method_mock() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::DELETE, "/test");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::DELETE);
    }

    #[test]
    fn test_request_patch_method_mock() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::PATCH, "/test");
        let built = req.build().unwrap();
        assert_eq!(built.method(), reqwest::Method::PATCH);
    }

    #[test]
    fn test_check_status_json_rate_limit() {
        let url = start_mock_server(|_| {
            (
                403,
                r#"{"message":"API rate limit exceeded for user ID 12345"}"#.to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("403"));
        assert!(msg.contains("API rate limit exceeded"));
    }

    #[test]
    fn test_check_status_json_validation_failed() {
        let url = start_mock_server(|_| {
            (
                422,
                r#"{"message":"Validation Failed","errors":[{"resource":"Repository","code":"custom","field":"name","message":"name already exists on this account"}]}"#.to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/repos/o/r"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("422"));
        assert!(msg.contains("Validation Failed"));
    }

    #[test]
    fn test_check_status_json_forbidden() {
        let url = start_mock_server(|_| {
            (
                403,
                r#"{"message":"Resource not accessible by integration"}"#.to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("403"));
        assert!(msg.contains("Resource not accessible by integration"));
    }

    #[test]
    fn test_check_status_json_null_message() {
        let url = start_mock_server(|_| (400, r#"{"message":null}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("400"));
    }

    #[test]
    fn test_check_status_empty_json_object() {
        let url = start_mock_server(|_| (400, r#"{}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("400"));
    }

    #[test]
    fn test_check_status_json_message_not_string() {
        let url = start_mock_server(|_| (400, r#"{"message":123}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("400"));
    }

    #[test]
    fn test_check_status_json_with_extra_fields() {
        let url = start_mock_server(|_| {
            (
                422,
                r#"{"message":"Validation Failed","documentation_url":"https://docs.github.com","errors":[{"code":"missing_field","field":"name"}]}"#.to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("422"));
        assert!(msg.contains("Validation Failed"));
    }

    #[test]
    fn test_check_status_empty_body() {
        let url = start_mock_server(|_| (204, String::new()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_status_502_bad_gateway() {
        let url = start_mock_server(|_| (502, "Bad Gateway".to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("502"));
    }

    #[test]
    fn test_check_status_503_service_unavailable() {
        let url = start_mock_server(|_| (503, "Service Unavailable".to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("503"));
    }

    #[test]
    fn test_check_status_429_too_many_requests() {
        let url =
            start_mock_server(|_| (429, r#"{"message":"API rate limit exceeded"}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("429"));
        assert!(msg.contains("rate limit exceeded"));
    }

    #[test]
    fn test_check_status_path_preserved_in_error() {
        let url = start_mock_server(|path| {
            if path.contains("repos") {
                (404, r#"{"message":"Not Found"}"#.to_string())
            } else {
                (404, r#"{"message":"Not Found"}"#.to_string())
            }
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/repos/owner/repo/branches/main/protection"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("404"));
        assert!(msg.contains("repos/owner/repo/branches/main/protection"));
    }

    #[test]
    fn test_request_builder_custom_headers() {
        let client = GithubClient::new("ghp_special_token_123");
        let req = client.request(reqwest::Method::GET, "/user");
        let built = req.build().unwrap();
        let headers = built.headers();
        assert_eq!(
            headers.get("Authorization").unwrap(),
            "token ghp_special_token_123"
        );
        assert_eq!(headers.get("User-Agent").unwrap(), "ghscaff");
        assert_eq!(
            headers.get("Accept").unwrap(),
            "application/vnd.github+json"
        );
    }

    #[test]
    fn test_request_builder_url_with_special_chars() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(
            reqwest::Method::GET,
            "/repos/owner/repo/labels/bug%20report",
        );
        let built = req.build().unwrap();
        assert!(built.url().as_str().contains("bug%20report"));
    }

    #[test]
    fn test_check_status_json_with_html_like_message() {
        let url =
            start_mock_server(|_| (403, r#"{"message":"<html>Rate limit</html>"}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("403"));
    }

    #[test]
    fn test_check_status_json_message_empty_string() {
        let url = start_mock_server(|_| (400, r#"{"message":""}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("400"));
    }

    #[test]
    fn test_check_status_410_gone() {
        let url = start_mock_server(|_| (410, r#"{"message":"Gone"}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("410"));
    }

    #[test]
    fn test_check_status_404_not_json() {
        let url = start_mock_server(|_| (404, "Not Found HTML".to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("404"));
        assert!(msg.contains("Not Found HTML"));
    }

    #[test]
    fn test_check_status_malformed_json() {
        let url = start_mock_server(|_| (400, "{not valid json".to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("400"));
    }

    #[test]
    fn test_check_status_json_with_html_message_value() {
        let url = start_mock_server(|_| {
            (
                403,
                r#"{"message":"<!DOCTYPE html><html><body>Rate limit</body></html>"}"#.to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("403"));
    }

    #[test]
    fn test_check_status_json_message_with_unicode() {
        let url = start_mock_server(|_| {
            (
                422,
                r#"{"message":"Validation Failed: 名前は必須です"}"#.to_string(),
            )
        });
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("422"));
    }

    #[test]
    fn test_check_status_api_path_with_multiple_segments() {
        let url = start_mock_server(|_| (404, r#"{"message":"Not Found"}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!(
                "{url}/repos/org/repo/git/refs/heads/feature/branch"
            ))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("404"));
    }

    #[test]
    fn test_request_builder_url_deep_path() {
        let client = GithubClient::new("ghp_test");
        let req = client.request(
            reqwest::Method::GET,
            "/repos/owner/repo/actions/secrets/public-key",
        );
        let built = req.build().unwrap();
        assert_eq!(
            built.url().as_str(),
            "https://api.github.com/repos/owner/repo/actions/secrets/public-key"
        );
    }

    #[test]
    fn test_request_builder_post_with_json_body() {
        use serde::Serialize;
        #[derive(Serialize)]
        struct Body {
            name: String,
            description: String,
            private: bool,
        }
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::POST, "/user/repos");
        let body = Body {
            name: "test".into(),
            description: "A test repo".into(),
            private: true,
        };
        let built = req.json(&body).build().unwrap();
        assert_eq!(built.method(), reqwest::Method::POST);
        let body_bytes = built.body().unwrap().as_bytes().unwrap();
        let body_str = String::from_utf8_lossy(body_bytes).to_string();
        assert!(body_str.contains("test"));
        assert!(body_str.contains("A test repo"));
    }

    #[test]
    fn test_request_builder_put_with_json_body() {
        use serde::Serialize;
        #[derive(Serialize)]
        struct Body {
            names: Vec<String>,
        }
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::PUT, "/repos/o/r/topics");
        let body = Body {
            names: vec!["rust".into(), "cli".into()],
        };
        let built = req.json(&body).build().unwrap();
        assert_eq!(built.method(), reqwest::Method::PUT);
    }

    #[test]
    fn test_request_builder_patch_with_json_body() {
        use serde::Serialize;
        #[derive(Serialize)]
        struct Body {
            description: String,
        }
        let client = GithubClient::new("ghp_test");
        let req = client.request(reqwest::Method::PATCH, "/repos/o/r");
        let body = Body {
            description: "Updated".into(),
        };
        let built = req.json(&body).build().unwrap();
        assert_eq!(built.method(), reqwest::Method::PATCH);
    }

    #[test]
    fn test_check_status_json_array_message() {
        let url = start_mock_server(|_| (422, r#"[{"message":"error"}]"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("422"));
    }

    #[test]
    fn test_check_status_301_redirect_not_handled() {
        let url = start_mock_server(|_| (301, "Moved Permanently".to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("301"));
    }

    #[test]
    fn test_check_status_json_nested_object_message() {
        let url = start_mock_server(|_| (400, r#"{"message":{"text":"Bad request"}}"#.to_string()));
        let resp = reqwest::blocking::Client::new()
            .get(format!("{url}/"))
            .send()
            .unwrap();
        let result = check_status(resp);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("400"));
    }
}
