//! Step-function tests for `super` (see `src/wizard/execution.rs`).
//!
//! Every `*_step` helper is driven against the recording mock server and
//! asserts the exact `(method, path)` pairs it issues on the success path,
//! plus the error/guard branches. `inquire` prompts fail without a TTY, so
//! run the suite as `cargo test </dev/null`.

use super::*;
use crate::github::test_utils::{env_lock, mock_client, start_recording_mock_server};
use std::sync::{Arc, Mutex};

type CallLog = Arc<Mutex<Vec<(String, String)>>>;

fn snapshot(log: &CallLog) -> Vec<(String, String)> {
    log.lock().unwrap().clone()
}

fn has_call(log: &CallLog, method: &str, path_part: &str) -> bool {
    snapshot(log)
        .iter()
        .any(|(m, p)| m == method && p.contains(path_part))
}

fn count_calls(log: &CallLog, method: &str, path_part: &str) -> usize {
    snapshot(log)
        .iter()
        .filter(|(m, p)| m == method && p.contains(path_part))
        .count()
}

fn not_found() -> (u16, String) {
    (404, r#"{"message":"Not Found"}"#.to_string())
}

fn repo_json() -> String {
    r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main","topics":[]}"#.to_string()
}

fn ref_json() -> String {
    r#"{"ref":"refs/heads/main","object":{"sha":"abc123"}}"#.to_string()
}

fn minimal_config() -> WizardConfig {
    WizardConfig {
        name: "repo".to_string(),
        description: "test repo".to_string(),
        topics: vec![],
        private: false,
        owner: "owner".to_string(),
        is_org: false,
        language: None,
        default_branch: "main".to_string(),
        create_develop: false,
        license: None,
        create_labels: false,
        team_access: vec![],
    }
}

// ── StepCounter ───────────────────────────────────────────────────

#[test]
fn step_counter_advance_counts_up() {
    let mut counter = StepCounter::new(3, false);
    assert_eq!(counter.advance(), (1, 3));
    assert_eq!(counter.advance(), (2, 3));
}

#[test]
fn step_counter_run_propagates_error() {
    let mut counter = StepCounter::new(1, false);
    let result = counter.run("failing op", || Err(anyhow::anyhow!("boom")));
    assert!(result.is_err());
}

#[test]
fn step_counter_run_dry_run_skips_op() {
    let mut counter = StepCounter::new(1, true);
    counter
        .run("skipped op", || panic!("must not run in dry-run"))
        .unwrap();
}

#[test]
fn step_counter_skip_counts_without_output() {
    let mut counter = StepCounter::new(5, false);
    counter.skip();
    assert_eq!(counter.advance(), (2, 5));
}

// ── build_wizard_init_files ───────────────────────────────────────

#[test]
fn build_wizard_init_files_includes_boilerplate() {
    let (url, _) = start_recording_mock_server(|method, path| {
        if method == "GET" && path.contains("/gitignore/templates/") {
            (200, r#"{"source":"*.log"}"#.to_string())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let template = crate::templates::rust::RustTemplate;
    let files = build_wizard_init_files(&client, &config, Some(&template)).unwrap();
    assert!(!files.is_empty());
    assert!(files.iter().any(|f| f.path == "Cargo.toml"));
    assert!(files.iter().any(|f| f.path == ".gitignore"));
}

#[test]
fn build_wizard_init_files_empty_without_template_or_license() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected without template or license, got: {path}")
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let files = build_wizard_init_files(&client, &config, None).unwrap();
    assert!(files.is_empty());
    assert!(snapshot(&log).is_empty());
}

// ── create_wizard_repo_step ───────────────────────────────────────

#[test]
fn create_wizard_repo_step_creates_repo() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "POST" && path == "/user/repos" {
            (201, repo_json())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(2, false);
    let repo = create_wizard_repo_step(&mut progress, &client, &config).unwrap();
    assert!(repo.is_some());
    assert!(has_call(&log, "POST", "/user/repos"));
}

#[test]
fn create_wizard_repo_step_dry_run_returns_none() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected for a dry run, got: {path}")
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(2, true);
    let repo = create_wizard_repo_step(&mut progress, &client, &config).unwrap();
    assert!(repo.is_none());
    assert!(snapshot(&log).is_empty());
}

#[test]
fn create_wizard_repo_step_propagates_error() {
    let (url, _) =
        start_recording_mock_server(|_, _| (422, r#"{"message":"Validation Failed"}"#.to_string()));
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(2, false);
    assert!(create_wizard_repo_step(&mut progress, &client, &config).is_err());
}

// ── commit_init_files_step ────────────────────────────────────────

fn tree_commit_handler() -> impl Fn(&str, &str) -> (u16, String) {
    move |method, path| {
        if method == "GET" && path.contains("/git/refs/heads/") {
            (200, r#"{"object":{"sha":"basesha"}}"#.to_string())
        } else if method == "GET" && path.contains("/git/commits/") {
            (
                200,
                r#"{"sha":"basesha","tree":{"sha":"basetree"}}"#.to_string(),
            )
        } else if method == "POST" && path.contains("/git/blobs") {
            (201, r#"{"sha":"blobsha"}"#.to_string())
        } else if method == "POST" && path.contains("/git/trees") {
            (201, r#"{"sha":"treesha"}"#.to_string())
        } else if method == "POST" && path.contains("/git/commits") {
            (
                201,
                r#"{"sha":"newsha","tree":{"sha":"treesha"}}"#.to_string(),
            )
        } else if method == "PATCH" && path.contains("/git/refs/heads/") {
            (200, r#"{"sha":"newsha"}"#.to_string())
        } else {
            not_found()
        }
    }
}

fn one_file() -> Vec<contents::TreeFile> {
    vec![contents::TreeFile {
        path: "README.md".to_string(),
        content: "# hi".to_string(),
    }]
}

#[test]
fn commit_init_files_step_commits_and_sets_sha() {
    let (url, log) = start_recording_mock_server(tree_commit_handler());
    let client = mock_client(&url);
    let config = minimal_config();
    let files = one_file();
    let mut progress = StepCounter::new(2, false);
    let mut sha = String::new();
    commit_init_files_step(&mut progress, &client, &config, &files, &mut sha).unwrap();
    assert_eq!(sha, "newsha");
    assert!(has_call(&log, "POST", "/repos/owner/repo/git/blobs"));
    assert!(has_call(&log, "POST", "/repos/owner/repo/git/trees"));
    assert!(has_call(&log, "POST", "/repos/owner/repo/git/commits"));
}

#[test]
fn commit_init_files_step_fails_on_server_error() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path.contains("/git/refs/heads/") {
            (200, r#"{"object":{"sha":"basesha"}}"#.to_string())
        } else if method == "GET" && path.contains("/git/commits/") {
            (
                200,
                r#"{"sha":"basesha","tree":{"sha":"basetree"}}"#.to_string(),
            )
        } else {
            (500, r#"{"message":"Server Error"}"#.to_string())
        }
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let files = one_file();
    let mut progress = StepCounter::new(2, false);
    let mut sha = String::new();
    let result = commit_init_files_step(&mut progress, &client, &config, &files, &mut sha);
    assert!(result.is_err(), "a 5xx on blob creation must surface");
    assert!(sha.is_empty(), "init sha must stay unset on failure");
    assert!(has_call(&log, "POST", "/repos/owner/repo/git/blobs"));
}

#[test]
fn commit_init_files_step_skips_without_files() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected without files, got: {path}")
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(2, false);
    let mut sha = String::new();
    commit_init_files_step(&mut progress, &client, &config, &[], &mut sha).unwrap();
    assert!(sha.is_empty());
    assert!(snapshot(&log).is_empty());
}

// ── create_develop_step ───────────────────────────────────────────

#[test]
fn create_develop_step_creates_branch() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "POST" && path.contains("/git/refs") {
            (201, r#"{"ref":"refs/heads/develop"}"#.to_string())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.create_develop = true;
    let mut progress = StepCounter::new(4, false);
    let mut sha = "presha".to_string();
    create_develop_step(&mut progress, &client, &config, &mut sha).unwrap();
    assert_eq!(sha, "presha");
    assert!(has_call(&log, "POST", "/repos/owner/repo/git/refs"));
}

#[test]
fn create_develop_step_skips_when_disabled() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected when develop is disabled, got: {path}")
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(2, false);
    let mut sha = String::new();
    create_develop_step(&mut progress, &client, &config, &mut sha).unwrap();
    assert!(sha.is_empty());
    assert!(snapshot(&log).is_empty());
}

#[test]
fn create_develop_step_propagates_error() {
    let (url, _) =
        start_recording_mock_server(|_, _| (500, r#"{"message":"Server Error"}"#.to_string()));
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.create_develop = true;
    let mut progress = StepCounter::new(4, false);
    let mut sha = "presha".to_string();
    assert!(create_develop_step(&mut progress, &client, &config, &mut sha).is_err());
}

#[test]
fn create_develop_step_dry_run_fetches_no_branch_sha() {
    // A dry run never created the repo, so reading `main` back would 404 and
    // abort the whole dry run; the SHA is only consumed by the skipped op.
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected for a dry run, got: {path}")
    });
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.create_develop = true;
    let mut progress = StepCounter::new(4, true);
    let mut sha = String::new();
    create_develop_step(&mut progress, &client, &config, &mut sha).unwrap();
    assert!(sha.is_empty());
    assert!(snapshot(&log).is_empty());
}

// ── apply_wizard_protections_step ─────────────────────────────────

fn protections_handler(put_status: u16) -> impl Fn(&str, &str) -> (u16, String) {
    move |method, path| {
        if method == "GET" && path.contains("/git/ref/heads/") {
            (200, ref_json())
        } else if method == "PUT" && path.contains("/protection") {
            (put_status, "{}".to_string())
        } else {
            not_found()
        }
    }
}

fn workflow_files() -> Vec<contents::TreeFile> {
    vec![contents::TreeFile {
        path: ".github/workflows/ci.yml".to_string(),
        content: "name: CI\non: [push]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n"
            .to_string(),
    }]
}

#[test]
fn apply_wizard_protections_step_puts_main_and_develop() {
    let (url, log) = start_recording_mock_server(protections_handler(200));
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.create_develop = true;
    let files = workflow_files();
    let mut progress = StepCounter::new(4, false);
    apply_wizard_protections_step(&mut progress, &client, &config, &files).unwrap();
    assert!(has_call(
        &log,
        "PUT",
        "/repos/owner/repo/branches/main/protection"
    ));
    assert!(has_call(
        &log,
        "PUT",
        "/repos/owner/repo/branches/develop/protection"
    ));
}

#[test]
fn apply_wizard_protections_step_propagates_error() {
    let (url, log) = start_recording_mock_server(protections_handler(500));
    let client = mock_client(&url);
    let config = minimal_config();
    let files = workflow_files();
    let mut progress = StepCounter::new(4, false);
    assert!(apply_wizard_protections_step(&mut progress, &client, &config, &files).is_err());
    assert!(has_call(
        &log,
        "PUT",
        "/repos/owner/repo/branches/main/protection"
    ));
}

// ── sync_wizard_labels_step ───────────────────────────────────────

fn wizard_labels_handler() -> impl Fn(&str, &str) -> (u16, String) {
    move |method, path| {
        if method == "GET" && path.contains("/labels?per_page=100") {
            (
                200,
                r#"[{"name":"bug","color":"ff0000","description":"Old"},{"name":"stale","color":"000000","description":"Stale"}]"#.to_string(),
            )
        } else if method == "PATCH" && path.contains("/labels/") {
            (
                200,
                r#"{"name":"bug","color":"d73a4a","description":"Something isn't working"}"#
                    .to_string(),
            )
        } else if method == "POST" && path.ends_with("/labels") {
            (
                201,
                r#"{"name":"x","color":"000000","description":"x"}"#.to_string(),
            )
        } else if method == "DELETE" && path.contains("/labels/") {
            (204, String::new())
        } else {
            not_found()
        }
    }
}

#[test]
fn sync_wizard_labels_step_reconciles() {
    let (url, log) = start_recording_mock_server(wizard_labels_handler());
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.create_labels = true;
    let mut progress = StepCounter::new(3, false);
    sync_wizard_labels_step(&mut progress, &client, &config).unwrap();
    assert!(has_call(
        &log,
        "GET",
        "/repos/owner/repo/labels?per_page=100"
    ));
    assert_eq!(
        count_calls(&log, "PATCH", "/repos/owner/repo/labels/bug"),
        1
    );
    assert_eq!(count_calls(&log, "POST", "/repos/owner/repo/labels"), 6);
    assert!(has_call(&log, "DELETE", "/repos/owner/repo/labels/stale"));
    assert!(!has_call(&log, "DELETE", "/repos/owner/repo/labels/bug"));
}

#[test]
fn sync_wizard_labels_step_skips_when_disabled() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected when labels are disabled, got: {path}")
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(2, false);
    sync_wizard_labels_step(&mut progress, &client, &config).unwrap();
    assert!(snapshot(&log).is_empty());
}

#[test]
fn sync_wizard_labels_step_propagates_error() {
    let (url, _) =
        start_recording_mock_server(|_, _| (500, r#"{"message":"Server Error"}"#.to_string()));
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.create_labels = true;
    let mut progress = StepCounter::new(3, false);
    assert!(sync_wizard_labels_step(&mut progress, &client, &config).is_err());
}

// ── set_wizard_topics_step ────────────────────────────────────────

#[test]
fn set_wizard_topics_step_sets_topics() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "PUT" && path.contains("/topics") {
            (200, r#"{"names":["rust"]}"#.to_string())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.topics = vec!["rust".to_string()];
    let mut progress = StepCounter::new(3, false);
    set_wizard_topics_step(&mut progress, &client, &config).unwrap();
    assert!(has_call(&log, "PUT", "/repos/owner/repo/topics"));
}

#[test]
fn set_wizard_topics_step_skips_when_empty() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected without topics, got: {path}")
    });
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(2, false);
    set_wizard_topics_step(&mut progress, &client, &config).unwrap();
    assert!(snapshot(&log).is_empty());
}

#[test]
fn set_wizard_topics_step_propagates_error() {
    let (url, _) =
        start_recording_mock_server(|_, _| (500, r#"{"message":"Server Error"}"#.to_string()));
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.topics = vec!["rust".to_string()];
    let mut progress = StepCounter::new(3, false);
    assert!(set_wizard_topics_step(&mut progress, &client, &config).is_err());
}

// ── grant_wizard_team_access_step ─────────────────────────────────

#[test]
fn grant_wizard_team_access_step_adds_teams() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "PUT" && path.contains("/teams/") {
            (204, String::new())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.team_access = vec![crate::github::teams::TeamAccess {
        team_slug: "backend".to_string(),
        permission: "push".to_string(),
    }];
    let mut progress = StepCounter::new(3, false);
    grant_wizard_team_access_step(&mut progress, &client, &config).unwrap();
    assert!(has_call(
        &log,
        "PUT",
        "/orgs/owner/teams/backend/repos/owner/repo"
    ));
}

#[test]
fn grant_wizard_team_access_step_propagates_error() {
    let (url, _) =
        start_recording_mock_server(|_, _| (500, r#"{"message":"Server Error"}"#.to_string()));
    let client = mock_client(&url);
    let mut config = minimal_config();
    config.team_access = vec![crate::github::teams::TeamAccess {
        team_slug: "backend".to_string(),
        permission: "push".to_string(),
    }];
    let mut progress = StepCounter::new(3, false);
    assert!(grant_wizard_team_access_step(&mut progress, &client, &config).is_err());
}

// ── configure_wizard_secrets_step ─────────────────────────────────

const WIZARD_SECRET: &str = "GHSCAFF_WIZARD_STEP_SECRET";

fn wizard_secret_spec() -> crate::templates::SecretSpec {
    crate::templates::SecretSpec {
        name: WIZARD_SECRET.to_string(),
        description: "wizard step test secret".to_string(),
        required: false,
    }
}

fn wizard_secrets_handler(put_status: u16) -> impl Fn(&str, &str) -> (u16, String) {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let key = STANDARD.encode([4u8; 32]);
    let public_key = format!(r#"{{"key_id":"wizard-key","key":"{key}"}}"#);
    move |method, path| {
        if method == "GET" && path.contains("/actions/secrets/public-key") {
            (200, public_key.clone())
        } else if method == "PUT" && path.contains("/actions/secrets/") {
            (put_status, String::new())
        } else {
            not_found()
        }
    }
}

#[test]
fn configure_wizard_secrets_step_sets_secret_from_env() {
    let _guard = env_lock();
    let prev = std::env::var_os(WIZARD_SECRET);
    std::env::set_var(WIZARD_SECRET, "wizard-value");

    let (url, log) = start_recording_mock_server(wizard_secrets_handler(204));
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(3, false);
    let result =
        configure_wizard_secrets_step(&mut progress, &client, &config, "", &[wizard_secret_spec()]);

    match prev {
        Some(value) => std::env::set_var(WIZARD_SECRET, value),
        None => std::env::remove_var(WIZARD_SECRET),
    }
    result.unwrap();
    assert!(has_call(
        &log,
        "GET",
        "/repos/owner/repo/actions/secrets/public-key"
    ));
    assert!(has_call(
        &log,
        "PUT",
        &format!("/repos/owner/repo/actions/secrets/{WIZARD_SECRET}")
    ));
}

#[test]
fn configure_wizard_secrets_step_propagates_error() {
    let _guard = env_lock();
    let prev = std::env::var_os(WIZARD_SECRET);
    std::env::set_var(WIZARD_SECRET, "wizard-value");

    let (url, _) = start_recording_mock_server(wizard_secrets_handler(500));
    let client = mock_client(&url);
    let config = minimal_config();
    let mut progress = StepCounter::new(3, false);
    let result =
        configure_wizard_secrets_step(&mut progress, &client, &config, "", &[wizard_secret_spec()]);

    match prev {
        Some(value) => std::env::set_var(WIZARD_SECRET, value),
        None => std::env::remove_var(WIZARD_SECRET),
    }
    assert!(result.is_err());
}

// ── execute ───────────────────────────────────────────────────────

#[test]
fn execute_runs_end_to_end_against_mock() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "POST" && path == "/user/repos" {
            (201, repo_json())
        } else if method == "GET" && path.contains("/git/ref/heads/") {
            (200, ref_json())
        } else if method == "PUT" && path.contains("/protection") {
            (200, "{}".to_string())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let config = minimal_config();
    execute(&client, &config, false, "token", "").unwrap();
    assert!(has_call(&log, "POST", "/user/repos"));
    assert!(has_call(
        &log,
        "PUT",
        "/repos/owner/repo/branches/main/protection"
    ));
}

// ── warn_if_rust_without_license ───────────────────────────────────

fn config_with(language: Option<&str>, license: Option<&str>) -> WizardConfig {
    WizardConfig {
        language: language.map(str::to_string),
        license: license.map(str::to_string),
        ..minimal_config()
    }
}

#[test]
fn warn_if_rust_without_license_only_for_rust_without_license() {
    assert!(warn_if_rust_without_license(&config_with(
        Some("rust"),
        None
    )));
    assert!(!warn_if_rust_without_license(&config_with(
        Some("rust"),
        Some("MIT")
    )));
    assert!(!warn_if_rust_without_license(&config_with(
        Some("python-module"),
        None
    )));
    assert!(!warn_if_rust_without_license(&config_with(None, None)));
}

// ── followups_enabled ─────────────────────────────────────────────

#[test]
fn followups_enabled_truth_table() {
    assert!(followups_enabled(false, true));
    assert!(!followups_enabled(true, true));
    assert!(!followups_enabled(false, false));
    assert!(!followups_enabled(true, false));
}
