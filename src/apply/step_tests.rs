//! Step-function tests for `super` (see `src/apply.rs`).
//!
//! Every `*_step` helper is driven against the recording mock server and
//! asserts the exact `(method, path)` pairs it issues on the success path,
//! plus the error/guard branches. Prompt-touching wrappers are covered
//! through their prompt-free cores (`collect_team_access_after`,
//! `select_team_access`, `apply_changes`, `apply_confirmed`): under a
//! non-TTY stdin every `inquire` prompt returns `Err`, so run the suite as
//! `cargo test </dev/null`.

use super::*;
use crate::github::test_utils::{env_lock, mock_client, start_recording_mock_server};
use std::sync::{Arc, Mutex};

type CallLog = Arc<Mutex<Vec<(String, String)>>>;

fn target_for(url: &str) -> ApplyTarget {
    ApplyTarget {
        client: mock_client(url),
        owner: "owner".into(),
        repo_name: "repo".into(),
        passphrase: String::new(),
    }
}

fn ctx_without_develop() -> ApplyContext {
    ApplyContext {
        owner: "owner".into(),
        repo: "repo".into(),
        current_labels: vec![],
        has_develop: false,
        branch_protection_enabled: false,
        has_ci_workflow: false,
        current_topics: vec![],
    }
}

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

fn has_put(log: &CallLog) -> bool {
    snapshot(log).iter().any(|(m, _)| m == "PUT")
}

fn not_found() -> (u16, String) {
    (404, r#"{"message":"Not Found"}"#.to_string())
}

// ── apply_labels_step ─────────────────────────────────────────────

#[test]
fn apply_labels_step_creates_standard_labels() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path.contains("/labels?per_page=100") {
            (200, "[]".to_string())
        } else if method == "POST" && path.ends_with("/labels") {
            (
                201,
                r#"{"name":"x","color":"000000","description":"x"}"#.to_string(),
            )
        } else {
            not_found()
        }
    });
    let target = target_for(&url);
    apply_labels_step(&target).unwrap();
    assert!(has_call(
        &log,
        "GET",
        "/repos/owner/repo/labels?per_page=100"
    ));
    assert_eq!(count_calls(&log, "POST", "/repos/owner/repo/labels"), 7);
}

#[test]
fn apply_labels_step_propagates_list_error() {
    let (url, log) =
        start_recording_mock_server(|_, _| (500, r#"{"message":"Server Error"}"#.to_string()));
    let target = target_for(&url);
    assert!(apply_labels_step(&target).is_err());
    assert!(has_call(&log, "GET", "/repos/owner/repo/labels"));
}

// ── apply_branch_protection_step ──────────────────────────────────

fn branch_protection_handler(put_status: u16) -> impl Fn(&str, &str) -> (u16, String) {
    move |method, path| {
        if method == "GET" && path.contains("/git/ref/heads/main") {
            (
                200,
                r#"{"ref":"refs/heads/main","object":{"sha":"abc123"}}"#.to_string(),
            )
        } else if method == "PUT" && path.contains("/branches/main/protection") {
            (put_status, "{}".to_string())
        } else {
            not_found()
        }
    }
}

#[test]
fn apply_branch_protection_step_puts_protection() {
    let (url, log) = start_recording_mock_server(branch_protection_handler(200));
    let target = target_for(&url);
    apply_branch_protection_step(&target).unwrap();
    assert!(has_call(
        &log,
        "GET",
        "/repos/owner/repo/git/ref/heads/main"
    ));
    assert!(has_call(
        &log,
        "PUT",
        "/repos/owner/repo/branches/main/protection"
    ));
}

#[test]
fn apply_branch_protection_step_survives_error() {
    let (url, log) = start_recording_mock_server(branch_protection_handler(500));
    let target = target_for(&url);
    apply_branch_protection_step(&target).unwrap();
    assert!(has_call(
        &log,
        "PUT",
        "/repos/owner/repo/branches/main/protection"
    ));
}

// ── apply_develop_branch_step ─────────────────────────────────────

#[test]
fn apply_develop_branch_step_skips_when_present() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected when develop exists, got: {path}")
    });
    let target = target_for(&url);
    let ctx = ApplyContext {
        has_develop: true,
        ..ctx_without_develop()
    };
    apply_develop_branch_step(&target, &ctx).unwrap();
    assert!(snapshot(&log).is_empty());
}

#[test]
fn apply_develop_branch_step_creates_when_missing() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path.contains("/git/ref/heads/main") {
            (
                200,
                r#"{"ref":"refs/heads/main","object":{"sha":"abc123"}}"#.to_string(),
            )
        } else if method == "POST" && path.contains("/git/refs") {
            (201, r#"{"ref":"refs/heads/develop"}"#.to_string())
        } else {
            not_found()
        }
    });
    let target = target_for(&url);
    apply_develop_branch_step(&target, &ctx_without_develop()).unwrap();
    assert!(has_call(
        &log,
        "GET",
        "/repos/owner/repo/git/ref/heads/main"
    ));
    assert!(has_call(&log, "POST", "/repos/owner/repo/git/refs"));
}

#[test]
fn apply_develop_branch_step_propagates_error() {
    let (url, _) =
        start_recording_mock_server(|_, _| (500, r#"{"message":"Server Error"}"#.to_string()));
    let target = target_for(&url);
    assert!(apply_develop_branch_step(&target, &ctx_without_develop()).is_err());
}

// ── apply_topics_step ─────────────────────────────────────────────

#[test]
fn apply_topics_step_updates_topics() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path == "/repos/owner/repo" {
            (
                200,
                r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main","topics":[]}"#.to_string(),
            )
        } else if method == "PUT" && path.contains("/topics") {
            (200, r#"{"names":["github","scaffold"]}"#.to_string())
        } else {
            not_found()
        }
    });
    let target = target_for(&url);
    apply_topics_step(&target).unwrap();
    assert!(has_call(&log, "GET", "/repos/owner/repo"));
    assert!(has_call(&log, "PUT", "/repos/owner/repo/topics"));
}

#[test]
fn apply_topics_step_survives_error() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path == "/repos/owner/repo" {
            (
                200,
                r#"{"full_name":"owner/repo","html_url":"https://github.com/owner/repo","default_branch":"main","topics":[]}"#.to_string(),
            )
        } else {
            (500, r#"{"message":"Server Error"}"#.to_string())
        }
    });
    let target = target_for(&url);
    apply_topics_step(&target).unwrap();
    assert!(has_call(&log, "PUT", "/repos/owner/repo/topics"));
}

// ── apply_team_access_step ────────────────────────────────────────

fn team_access(teams: &[(&str, &str)]) -> Vec<teams::TeamAccess> {
    teams
        .iter()
        .map(|(slug, permission)| teams::TeamAccess {
            team_slug: slug.to_string(),
            permission: permission.to_string(),
        })
        .collect()
}

#[test]
fn apply_team_access_step_adds_each_team() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "PUT" && path.contains("/teams/") {
            (204, String::new())
        } else {
            not_found()
        }
    });
    let target = target_for(&url);
    apply_team_access_step(
        &target,
        &team_access(&[("backend", "push"), ("devops", "pull")]),
    )
    .unwrap();
    assert!(has_call(
        &log,
        "PUT",
        "/orgs/owner/teams/backend/repos/owner/repo"
    ));
    assert!(has_call(
        &log,
        "PUT",
        "/orgs/owner/teams/devops/repos/owner/repo"
    ));
}

#[test]
fn apply_team_access_step_survives_error() {
    let (url, log) =
        start_recording_mock_server(|_, _| (500, r#"{"message":"Server Error"}"#.to_string()));
    let target = target_for(&url);
    apply_team_access_step(&target, &team_access(&[("backend", "push")])).unwrap();
    assert!(has_call(
        &log,
        "PUT",
        "/orgs/owner/teams/backend/repos/owner/repo"
    ));
}

#[test]
fn apply_team_access_step_no_teams_no_requests() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected without teams, got: {path}")
    });
    let target = target_for(&url);
    apply_team_access_step(&target, &[]).unwrap();
    assert!(snapshot(&log).is_empty());
}

// ── apply_missing_secrets_step ────────────────────────────────────

const STEP_SECRET: &str = "GHSCAFF_STEP_TEST_SECRET";

fn rust_secrets_toml() -> String {
    format!(
        "[[secrets]]\nname = \"{STEP_SECRET}\"\ndescription = \"step test secret\"\nrequired = false\n"
    )
}

/// Point HOME at a temp dir holding `boilerplate/rust/secrets.toml`.
/// Returns the tempdir (keep alive for the test) and the previous HOME.
fn isolate_home_with_rust_secrets() -> (tempfile::TempDir, Option<std::ffi::OsString>) {
    let home = tempfile::tempdir().unwrap();
    let dir = home
        .path()
        .join(".ghscaff")
        .join("boilerplate")
        .join("rust");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("secrets.toml"), rust_secrets_toml()).unwrap();
    let prev = std::env::var_os("HOME");
    std::env::set_var("HOME", home.path());
    (home, prev)
}

fn restore_home(prev: Option<std::ffi::OsString>) {
    match prev {
        Some(value) => std::env::set_var("HOME", value),
        None => std::env::remove_var("HOME"),
    }
}

fn set_step_secret_env() -> Option<std::ffi::OsString> {
    let prev = std::env::var_os(STEP_SECRET);
    std::env::set_var(STEP_SECRET, "step-test-value");
    prev
}

fn restore_step_secret_env(prev: Option<std::ffi::OsString>) {
    match prev {
        Some(value) => std::env::set_var(STEP_SECRET, value),
        None => std::env::remove_var(STEP_SECRET),
    }
}

fn public_key_json() -> String {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let key = STANDARD.encode([9u8; 32]);
    format!(r#"{{"key_id":"step-test-key","key":"{key}"}}"#)
}

fn secrets_handler(secrets_list: String) -> impl Fn(&str, &str) -> (u16, String) {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let cargo_encoded = STANDARD.encode(b"[package]\nname = \"test\"");
    let public_key = public_key_json();
    move |method, path| {
        if path.contains("/contents/Cargo.toml") {
            (
                200,
                format!(r#"{{"content":"{cargo_encoded}","encoding":"base64"}}"#),
            )
        } else if method == "GET" && path.contains("/actions/secrets?per_page=100") {
            (200, secrets_list.clone())
        } else if method == "GET" && path.contains("/actions/secrets/public-key") {
            (200, public_key.clone())
        } else if method == "PUT" && path.contains("/actions/secrets/") {
            (204, String::new())
        } else {
            not_found()
        }
    }
}

#[test]
fn apply_missing_secrets_step_sets_missing_secret() {
    let _guard = env_lock();
    let (_home, prev_home) = isolate_home_with_rust_secrets();
    let prev_secret = set_step_secret_env();

    let (url, log) = start_recording_mock_server(secrets_handler(r#"{"secrets":[]}"#.to_string()));
    let target = target_for(&url);
    let result = apply_missing_secrets_step(&target);

    restore_step_secret_env(prev_secret);
    restore_home(prev_home);
    result.unwrap();
    assert!(has_call(
        &log,
        "PUT",
        &format!("/repos/owner/repo/actions/secrets/{STEP_SECRET}")
    ));
}

#[test]
fn apply_missing_secrets_step_skips_secret_already_present() {
    let _guard = env_lock();
    let (_home, prev_home) = isolate_home_with_rust_secrets();
    let prev_secret = set_step_secret_env();

    let list = format!(r#"{{"secrets":[{{"name":"{STEP_SECRET}"}}]}}"#);
    let (url, log) = start_recording_mock_server(secrets_handler(list));
    let target = target_for(&url);
    let result = apply_missing_secrets_step(&target);

    restore_step_secret_env(prev_secret);
    restore_home(prev_home);
    result.unwrap();
    assert!(
        !has_put(&log),
        "a present secret must not be written again: {:?}",
        snapshot(&log)
    );
}

#[test]
fn apply_missing_secrets_step_no_markers_no_writes() {
    let _guard = env_lock();
    let (_home, prev_home) = isolate_home_with_rust_secrets();
    let prev_secret = set_step_secret_env();

    let (url, log) = start_recording_mock_server(|_, _| not_found());
    let target = target_for(&url);
    let result = apply_missing_secrets_step(&target);

    restore_step_secret_env(prev_secret);
    restore_home(prev_home);
    result.unwrap();
    assert!(
        !has_put(&log),
        "without marker files nothing may be written: {:?}",
        snapshot(&log)
    );
}

// ── collect_apply_team_access / cores ─────────────────────────────

#[test]
fn collect_apply_team_access_errors_without_tty() {
    let url = start_mock_server_for_path();
    let client = mock_client(&url);
    assert!(collect_apply_team_access(&client, "owner").is_err());
}

fn start_mock_server_for_path() -> String {
    crate::github::test_utils::start_mock_server(|_| not_found())
}

#[test]
fn collect_team_access_after_false_wants_nothing() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected when teams declined, got: {path}")
    });
    let client = mock_client(&url);
    let result = collect_team_access_after(&client, "owner", false, |_| {
        panic!("must not prompt when teams declined")
    })
    .unwrap();
    assert!(result.is_empty());
    assert!(snapshot(&log).is_empty());
}

fn one_backend_team() -> String {
    r#"[{"name":"Backend","slug":"backend","description":null}]"#.to_string()
}

#[test]
fn collect_team_access_after_true_empty_org_wants_nothing() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path == "/user/teams" {
            (200, "[]".to_string())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let result = collect_team_access_after(&client, "owner", true, |_| {
        panic!("must not prompt without org teams")
    })
    .unwrap();
    assert!(result.is_empty());
    assert!(has_call(&log, "GET", "/user/teams"));
}

#[test]
fn collect_team_access_after_true_skipped_selection_wants_nothing() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path == "/user/teams" {
            (200, one_backend_team())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let result = collect_team_access_after(&client, "owner", true, |names| {
        assert_eq!(names, vec!["Backend".to_string()]);
        Ok(None)
    })
    .unwrap();
    assert!(result.is_empty());
    assert!(has_call(&log, "GET", "/user/teams"));
}

#[test]
fn collect_team_access_after_true_selection_propagates_permission_error() {
    // The selection is injected; the per-team permission prompt has no TTY
    // here, so resolving it fails — which is exactly the observable
    // difference from the `Ok(vec![])` whole-function mutant.
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path == "/user/teams" {
            (200, one_backend_team())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let result = collect_team_access_after(&client, "owner", true, |_| {
        Ok(Some(vec!["Backend".to_string()]))
    });
    assert!(result.is_err());
    assert!(has_call(&log, "GET", "/user/teams"));
}

#[test]
fn collect_team_access_after_true_propagates_prompt_error() {
    let (url, log) = start_recording_mock_server(|method, path| {
        if method == "GET" && path == "/user/teams" {
            (200, one_backend_team())
        } else {
            not_found()
        }
    });
    let client = mock_client(&url);
    let result = collect_team_access_after(&client, "owner", true, |_| {
        Err::<Option<Vec<String>>, _>(anyhow::anyhow!("prompt unavailable"))
    });
    assert!(result.is_err());
    assert!(has_call(&log, "GET", "/user/teams"));
}

#[test]
fn select_team_access_matches_by_exact_name() {
    let org_teams = vec![teams::Team {
        name: "backend".to_string(),
        slug: "backend".to_string(),
        description: None,
    }];
    let selections = vec!["backend".to_string()];
    let result = select_team_access(&org_teams, &selections, |team| {
        Ok(teams::TeamAccess {
            team_slug: team.slug.clone(),
            permission: "push".to_string(),
        })
    })
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].team_slug, "backend");
    assert_eq!(result[0].permission, "push");
}

#[test]
fn select_team_access_skips_unknown_name() {
    let org_teams = vec![teams::Team {
        name: "backend".to_string(),
        slug: "backend".to_string(),
        description: None,
    }];
    let selections = vec!["no-such-team".to_string()];
    let result = select_team_access(&org_teams, &selections, |_| {
        panic!("permission must not be asked for unknown teams")
    })
    .unwrap();
    assert!(result.is_empty());
}

#[test]
fn select_team_access_propagates_permission_error() {
    let org_teams = vec![teams::Team {
        name: "backend".to_string(),
        slug: "backend".to_string(),
        description: None,
    }];
    let selections = vec!["backend".to_string()];
    let result = select_team_access(&org_teams, &selections, |_| {
        Err(anyhow::anyhow!("permission prompt unavailable"))
    });
    assert!(result.is_err());
}

// ── run_apply / cores ─────────────────────────────────────────────

#[test]
fn run_apply_errors_without_token() {
    let _guard = env_lock();
    let prev_token = std::env::var_os("GITHUB_TOKEN");
    std::env::remove_var("GITHUB_TOKEN");
    let home = tempfile::tempdir().unwrap();
    let prev_home = std::env::var_os("HOME");
    std::env::set_var("HOME", home.path());

    let result = run_apply(Some("owner/repo"), false);

    match prev_token {
        Some(value) => std::env::set_var("GITHUB_TOKEN", value),
        None => std::env::remove_var("GITHUB_TOKEN"),
    }
    restore_home(prev_home);
    assert!(result.is_err());
}

#[test]
fn apply_changes_errors_when_prompt_unavailable() {
    let (url, _) = start_recording_mock_server(|_, _| not_found());
    let target = target_for(&url);
    assert!(apply_changes(&target, &ctx_without_develop(), false).is_err());
}

#[test]
fn apply_changes_dry_run_makes_no_requests() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected for a dry run, got: {path}")
    });
    let target = target_for(&url);
    apply_changes(&target, &ctx_without_develop(), true).unwrap();
    assert!(snapshot(&log).is_empty());
}

#[test]
fn apply_confirmed_aborts_when_declined() {
    let (url, log) = start_recording_mock_server(|_, path| {
        panic!("no requests expected after declining, got: {path}")
    });
    let target = target_for(&url);
    apply_confirmed(&target, &ctx_without_develop(), &[], false).unwrap();
    assert!(snapshot(&log).is_empty());
}

#[test]
fn apply_confirmed_propagates_step_error() {
    let (url, log) =
        start_recording_mock_server(|_, _| (500, r#"{"message":"Server Error"}"#.to_string()));
    let target = target_for(&url);
    assert!(apply_confirmed(&target, &ctx_without_develop(), &[], true).is_err());
    assert!(has_call(&log, "GET", "/repos/owner/repo/labels"));
}
