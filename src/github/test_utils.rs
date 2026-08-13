use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;

use super::client::GithubClient;

/// Start a mock HTTP server that calls `handler` for each request.
/// Returns the base URL (e.g. "http://127.0.0.1:12345").
///
/// The handler receives the request path and returns (status_code, response_body).
pub fn start_mock_server(
    handler: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
) -> String {
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
                201 => "Created",
                204 => "No Content",
                400 => "Bad Request",
                403 => "Forbidden",
                404 => "Not Found",
                422 => "Unprocessable Entity",
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

/// Create a `GithubClient` that points at the given mock server URL.
pub fn mock_client(base_url: &str) -> GithubClient {
    GithubClient::new_with_base_url("ghp_test_token", base_url)
}
