use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use super::client::GithubClient;

/// The one crate-wide lock for tests that read or write process-global state
/// (`GITHUB_TOKEN`, `HOME`, secret-name env vars, the `~/.ghscaff` cache and
/// vault). Every such test — in `vault`, `updater`, `apply` and `wizard` —
/// must take *this* guard, never a module-local one, otherwise two suites
/// swap `HOME` at the same time and each observes the other's temp
/// directory. Tests run in parallel by default; hold the guard for the whole
/// test.
pub fn env_lock() -> MutexGuard<'static, ()> {
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub type RequestLog = Arc<Mutex<Vec<(String, String)>>>;

/// Start a mock HTTP server that calls `handler` for each request and
/// records every `(method, path)` pair it serves.
///
/// Returns the base URL plus a shared log the test can read. `GithubClient`
/// uses keep-alive connections, so each connection is served in a loop until
/// the client closes it.
pub fn start_recording_mock_server(
    handler: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
) -> (String, RequestLog) {
    let handler = Arc::new(handler);
    let log: RequestLog = Arc::new(Mutex::new(Vec::new()));
    let logged = Arc::clone(&log);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://127.0.0.1:{}", addr.port());

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                    break;
                }
                if request_line.trim().is_empty() {
                    break;
                }
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let trimmed = line.trim().to_string();
                    if trimmed.is_empty() {
                        break;
                    }
                }
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or("GET");
                let path = parts.next().unwrap_or("/");
                logged
                    .lock()
                    .unwrap()
                    .push((method.to_string(), path.to_string()));
                let (status, body) = (handler)(method, path);
                let status_text = match status {
                    200 => "OK",
                    201 => "Created",
                    204 => "No Content",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    422 => "Unprocessable Entity",
                    500 => "Internal Server Error",
                    _ => "Unknown",
                };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
        }
    });

    (url, log)
}

/// Start a mock HTTP server that calls `handler` for each request.
/// Returns the base URL (e.g. "http://127.0.0.1:12345").
///
/// The handler receives the request path and returns (status_code, response_body).
pub fn start_mock_server(
    handler: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
) -> String {
    let (url, _log) = start_recording_mock_server(move |_method, path| handler(path));
    url
}

/// Create a `GithubClient` that points at the given mock server URL.
pub fn mock_client(base_url: &str) -> GithubClient {
    GithubClient::new_with_base_url("ghp_test_token", base_url)
}
