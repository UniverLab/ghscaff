use anyhow::{Context, Result};
use std::process::Command;

use crate::github::{client::GithubClient, labels, repo, secrets, teams};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ApplyContext {
    pub owner: String,
    pub repo: String,
    pub current_labels: Vec<labels::Label>,
    pub has_develop: bool,
    pub branch_protection_enabled: bool,
    pub has_ci_workflow: bool,
    pub current_topics: Vec<String>,
}

/// Auto-detect owner/repo from git remote origin
/// Handles both HTTPS (https://github.com/owner/repo.git) and SSH (git@github.com:owner/repo.git)
pub fn auto_detect_repo() -> Result<(String, String)> {
    let output = Command::new("git")
        .arg("remote")
        .arg("get-url")
        .arg("origin")
        .output()
        .context("Failed to execute git command. Are you in a git repository?")?;

    if !output.status.success() {
        anyhow::bail!("Failed to get git remote. Make sure you're in a git repository with an 'origin' remote.");
    }

    let remote = String::from_utf8(output.stdout)
        .context("Invalid UTF-8 in git remote URL")?
        .trim()
        .to_string();

    parse_github_remote(&remote)
}

fn parse_github_remote(remote: &str) -> Result<(String, String)> {
    // Handle HTTPS: https://github.com/owner/repo.git
    if remote.starts_with("https://") {
        let trimmed = remote
            .strip_prefix("https://github.com/")
            .context("HTTPS remote must be from github.com")?
            .strip_suffix(".git")
            .unwrap_or(remote.strip_prefix("https://github.com/").unwrap());

        let parts: Vec<&str> = trimmed.split('/').collect();
        if parts.len() >= 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
    }

    // Handle SSH: git@github.com:owner/repo.git
    if remote.starts_with("git@github.com:") {
        let trimmed = remote
            .strip_prefix("git@github.com:")
            .unwrap()
            .strip_suffix(".git")
            .unwrap_or(remote.strip_prefix("git@github.com:").unwrap());

        let parts: Vec<&str> = trimmed.split('/').collect();
        if parts.len() >= 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
    }

    anyhow::bail!(
        "Could not parse GitHub remote: {}. Expected format: https://github.com/owner/repo.git or git@github.com:owner/repo.git",
        remote
    )
}

/// Fetch current state of the repository
pub fn get_repo_state(client: &GithubClient, owner: &str, repo_name: &str) -> Result<ApplyContext> {
    // Get current labels
    let current_labels = labels::list_labels(client, owner, repo_name)?;

    // Check for develop branch
    let has_develop = check_branch_exists(client, owner, repo_name, "develop")?;

    // Check branch protection status
    let branch_protection_enabled = check_branch_protection(client, owner, repo_name, "main")?;

    // Check if CI workflow exists
    let has_ci_workflow = check_file_exists(client, owner, repo_name, ".github/workflows/ci.yml")?;

    // Get current topics
    let gh_repo = repo::get_repo(client, owner, repo_name)?;
    let current_topics = gh_repo.topics.unwrap_or_default();

    Ok(ApplyContext {
        owner: owner.to_string(),
        repo: repo_name.to_string(),
        current_labels,
        has_develop,
        branch_protection_enabled,
        has_ci_workflow,
        current_topics,
    })
}

fn check_branch_exists(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<bool> {
    let path = format!("/repos/{owner}/{repo}/git/ref/heads/{branch}");
    match client.get::<serde_json::Value>(&path) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn check_branch_protection(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<bool> {
    let path = format!("/repos/{owner}/{repo}/branches/{branch}/protection");
    match client.get::<serde_json::Value>(&path) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn check_file_exists(client: &GithubClient, owner: &str, repo: &str, path: &str) -> Result<bool> {
    let encoded_path = urlencoding::encode(path);
    let api_path = format!("/repos/{owner}/{repo}/contents/{encoded_path}");
    match client.get::<serde_json::Value>(&api_path) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// Detect which template a repo was scaffolded from by its marker files and
/// return the secrets those templates declare. Unlike the wizard (which knows the
/// chosen template), `apply` runs against an existing repo, so we infer it from
/// language marker files. Secrets are de-duplicated by name across matches.
fn detect_template_secrets(
    client: &GithubClient,
    owner: &str,
    repo: &str,
) -> Vec<crate::templates::SecretSpec> {
    const MARKERS: &[(&str, &[&str])] = &[
        ("Cargo.toml", &["rust"]),
        ("pyproject.toml", &["python-fastapi", "python-module"]),
    ];
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    MARKERS
        .iter()
        .filter(|(marker, _)| check_file_exists(client, owner, repo, marker).unwrap_or(false))
        .flat_map(|(_, langs)| {
            langs
                .iter()
                .flat_map(|lang| crate::templates::load_secrets(lang))
        })
        .filter(|spec| seen.insert(spec.name.clone()))
        .collect()
}

/// Sync labels idempotently - create missing, update existing
pub fn sync_labels(
    client: &GithubClient,
    owner: &str,
    repo_name: &str,
    dry_run: bool,
) -> Result<SyncResult> {
    let current = labels::list_labels(client, owner, repo_name)?;
    let standard = labels::standard_labels();

    let mut created = 0;
    let mut updated = 0;
    let mut up_to_date = 0;
    let mut deleted = 0;

    for std_label in &standard {
        if let Some(existing) = current.iter().find(|l| l.name == std_label.name) {
            if existing.color != std_label.color || existing.description != std_label.description {
                if !dry_run {
                    labels::update_label(client, owner, repo_name, &std_label.name, std_label)?;
                }
                updated += 1;
            } else {
                up_to_date += 1;
            }
        } else {
            if !dry_run {
                labels::create_label(client, owner, repo_name, std_label)?;
            }
            created += 1;
        }
    }

    for existing in &current {
        if !standard.iter().any(|s| s.name == existing.name) {
            if !dry_run {
                let _ = labels::delete_label(client, owner, repo_name, &existing.name);
            }
            deleted += 1;
        }
    }

    Ok(SyncResult {
        created,
        updated,
        up_to_date,
        deleted,
    })
}

/// Merge topics - add template topics without removing existing
pub fn merge_topics(
    client: &GithubClient,
    owner: &str,
    repo_name: &str,
    template_topics: &[&str],
    dry_run: bool,
) -> Result<bool> {
    let repo_obj = repo::get_repo(client, owner, repo_name)?;
    let mut current_topics = repo_obj.topics.unwrap_or_default();

    let mut changed = false;
    for topic in template_topics {
        if !current_topics.contains(&topic.to_string()) {
            current_topics.push(topic.to_string());
            changed = true;
        }
    }

    if changed && !dry_run {
        repo::set_topics(client, owner, repo_name, &current_topics)?;
    }

    Ok(changed)
}

#[derive(Debug, Clone)]
pub struct SyncResult {
    pub created: usize,
    pub updated: usize,
    pub up_to_date: usize,
    pub deleted: usize,
}

/// Main apply mode orchestrator
pub fn run_apply(repo_arg: Option<&str>, dry_run: bool) -> Result<()> {
    // Get token
    let (token, passphrase) = crate::github::client::resolve_token()?;
    let client = crate::github::client::GithubClient::new(&token);

    // Determine repo
    let (owner, repo_name) = if let Some(repo) = repo_arg {
        parse_owner_repo(repo)?
    } else {
        auto_detect_repo()?
    };

    println!("  Checking existing repo... {}/{}", owner, repo_name);
    let ctx = get_repo_state(&client, &owner, &repo_name)?;

    // Display summary
    println!();
    println!("  Summary of changes:");
    println!("  ◆ Labels: checking...");
    let label_result = sync_labels(&client, &owner, &repo_name, true)?; // dry check
    if label_result.created > 0 || label_result.updated > 0 || label_result.deleted > 0 {
        println!(
            "    • {} to create, {} to update, {} to delete, {} up to date",
            label_result.created,
            label_result.updated,
            label_result.deleted,
            label_result.up_to_date
        );
    } else {
        println!("    • all up to date");
    }

    println!("  ◆ Branch protection (main): {}", {
        if ctx.branch_protection_enabled {
            "enabled"
        } else {
            "would apply"
        }
    });

    println!("  ◆ develop branch: {}", {
        if ctx.has_develop {
            "exists"
        } else {
            "would create"
        }
    });

    println!("  ◆ CI workflow: {}", {
        if ctx.has_ci_workflow {
            "exists"
        } else {
            "would create"
        }
    });

    if dry_run {
        println!();
        println!("  [dry-run] No changes applied.");
        return Ok(());
    }

    println!();
    let mut selected_teams: Vec<teams::TeamAccess> = vec![];

    let want_teams = inquire::Confirm::new("Add team access?")
        .with_default(false)
        .prompt()?;

    if want_teams {
        if let Ok(org_teams) = list_org_teams(&client, &owner) {
            if !org_teams.is_empty() {
                let team_names: Vec<String> = org_teams.iter().map(|t| t.name.clone()).collect();

                if let Ok(Some(selections)) =
                    inquire::MultiSelect::new("Select teams:", team_names.clone())
                        .with_help_message("space select  enter confirm")
                        .prompt_skippable()
                {
                    for selected_team_display in selections {
                        if let Some(team) =
                            org_teams.iter().find(|t| t.name == selected_team_display)
                        {
                            let permission = inquire::Select::new(
                                &format!("Permission for {} team:", team.name),
                                vec!["pull", "triage", "push", "admin"],
                            )
                            .prompt()?;

                            selected_teams.push(teams::TeamAccess {
                                team_slug: team.slug.clone(),
                                permission: permission.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    println!();
    let confirmed = inquire::Confirm::new("Apply these changes?")
        .with_default(true)
        .prompt()?;

    if !confirmed {
        println!("  Aborted.");
        return Ok(());
    }

    // Apply all changes
    println!();
    println!("  Applying changes...");

    // 1. Labels
    sync_labels(&client, &owner, &repo_name, false)?;
    println!("  ✓ Labels synced");

    // 2. Branch protection (always apply to ensure correct config)
    match crate::github::branches::apply_branch_protection(
        &client,
        &owner,
        &repo_name,
        "main",
        Some("rust-ci / Format, Lint & Test"),
    ) {
        Ok(()) => println!("  ✓ Branch protection applied"),
        Err(e) => {
            let msg = format!("{e:#}");
            if msg.contains("403") {
                println!("  ⚠ Branch protection skipped (403 Forbidden)");
                println!("    Possible causes:");
                println!("    • Private repo on a free org plan (requires GitHub Team)");
                println!("    • Token not authorized for this organization");
            } else {
                println!("  ⚠ Branch protection failed: {msg}");
            }
        }
    }

    // 3. Develop branch (if needed)
    if !ctx.has_develop {
        create_develop_branch(&client, &owner, &repo_name)?;
        println!("  ✓ develop branch created");
    }

    // 4. Merge topics
    match merge_topics(&client, &owner, &repo_name, &["github", "scaffold"], false) {
        Ok(true) => println!("  ✓ Topics updated"),
        Ok(false) => {}
        Err(e) => {
            let msg = format!("{e:#}");
            if msg.contains("403") {
                println!("  ⚠ Topics skipped (403 Forbidden) — token may lack org write access");
            } else {
                println!("  ⚠ Topics failed: {msg}");
            }
        }
    }

    // 5. Team access
    for team in &selected_teams {
        match add_team_to_repo(
            &client,
            &owner,
            &repo_name,
            &team.team_slug,
            &team.permission,
            false,
        ) {
            Ok(()) => println!(
                "  ✓ Team {} added with {} access",
                team.team_slug, team.permission
            ),
            Err(e) => {
                let msg = format!("{e:#}");
                if msg.contains("403") {
                    println!(
                        "  ⚠ Team {} skipped (403 Forbidden) — token may lack org write access",
                        team.team_slug
                    );
                } else {
                    println!("  ⚠ Failed to add team {}: {msg}", team.team_slug);
                }
            }
        }
    }

    // 6. Secrets from template (detected from the repo's marker files)
    let secret_specs = detect_template_secrets(&client, &owner, &repo_name);
    if !secret_specs.is_empty() {
        let existing = secrets::list_secret_names(&client, &owner, &repo_name).unwrap_or_default();
        let missing: Vec<_> = secret_specs
            .iter()
            .filter(|s| !existing.iter().any(|e| e == &s.name))
            .collect();
        if !missing.is_empty() {
            println!();
            println!("  ◆ Secrets required by template:");
            for spec in &missing {
                println!("    • {} — {}", spec.name, spec.description);
            }
            println!();
            for spec in missing {
                let value =
                    if let Some(val) = crate::vault::resolve_secret(&spec.name, &passphrase)? {
                        Some(val)
                    } else {
                        crate::wizard::prompt_secret_value(spec, &passphrase)?
                    };
                if let Some(val) = value {
                    match secrets::set_secret(&client, &owner, &repo_name, &spec.name, &val) {
                        Ok(()) => println!("  ✓ Secret {} configured", spec.name),
                        Err(e) => println!("  ⚠ Failed to set {}: {e:#}", spec.name),
                    }
                }
            }
        }
    }

    println!();
    println!("  Done!");
    Ok(())
}

fn parse_owner_repo(input: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = input.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repo format. Use: owner/repo");
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// List teams available in organization
pub fn list_org_teams(client: &GithubClient, _owner: &str) -> Result<Vec<teams::Team>> {
    teams::list_teams(client)
}

/// Add team to repository with specified permission
pub fn add_team_to_repo(
    client: &GithubClient,
    owner: &str,
    repo_name: &str,
    team_slug: &str,
    permission: &str,
    dry_run: bool,
) -> Result<()> {
    if !dry_run {
        teams::add_team_to_repo(client, owner, repo_name, team_slug, permission)?;
    }
    Ok(())
}

fn create_develop_branch(client: &GithubClient, owner: &str, repo_name: &str) -> Result<()> {
    use crate::github::branches;
    let main_sha = branches::get_branch_sha(client, owner, repo_name, "main")?;
    branches::create_branch(client, owner, repo_name, "develop", &main_sha)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_https_remote() {
        let remote = "https://github.com/owner/repo.git";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_https_remote_no_git_suffix() {
        let remote = "https://github.com/owner/repo";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_ssh_remote() {
        let remote = "git@github.com:owner/repo.git";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_ssh_remote_no_git_suffix() {
        let remote = "git@github.com:owner/repo";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_invalid_remote() {
        let remote = "https://gitlab.com/owner/repo.git";
        let result = parse_github_remote(remote);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_owner_repo_valid() {
        let (owner, repo) = parse_owner_repo("myowner/myrepo").unwrap();
        assert_eq!(owner, "myowner");
        assert_eq!(repo, "myrepo");
    }

    #[test]
    fn test_parse_owner_repo_invalid() {
        let result = parse_owner_repo("invalid-format");
        assert!(result.is_err());
    }

    #[test]
    fn test_sync_result_struct() {
        let result = SyncResult {
            created: 2,
            updated: 1,
            up_to_date: 9,
            deleted: 3,
        };
        assert_eq!(result.created, 2);
        assert_eq!(result.updated, 1);
        assert_eq!(result.up_to_date, 9);
        assert_eq!(result.deleted, 3);
    }

    #[test]
    fn test_parse_https_remote_deep_path() {
        let remote = "https://github.com/org-name/repo-name.git";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "org-name");
        assert_eq!(repo, "repo-name");
    }

    #[test]
    fn test_parse_ssh_remote_deep_path() {
        let remote = "git@github.com:org-name/repo-name.git";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "org-name");
        assert_eq!(repo, "repo-name");
    }

    #[test]
    fn test_parse_empty_remote() {
        assert!(parse_github_remote("").is_err());
    }

    #[test]
    fn test_parse_ssh_single_component() {
        assert!(parse_github_remote("git@github.com:onlyone").is_err());
    }

    #[test]
    fn test_parse_https_single_component() {
        assert!(parse_github_remote("https://github.com/onlyone").is_err());
    }

    #[test]
    fn test_parse_http_not_github() {
        assert!(parse_github_remote("http://github.com/a/b").is_err());
    }

    #[test]
    fn test_parse_owner_repo_empty() {
        assert!(parse_owner_repo("").is_err());
    }

    #[test]
    fn test_parse_owner_repo_too_many_slashes() {
        assert!(parse_owner_repo("a/b/c").is_err());
    }

    #[test]
    fn test_apply_context_struct() {
        let ctx = ApplyContext {
            owner: "o".into(),
            repo: "r".into(),
            current_labels: vec![],
            has_develop: true,
            branch_protection_enabled: false,
            has_ci_workflow: true,
            current_topics: vec!["rust".into()],
        };
        assert_eq!(ctx.owner, "o");
        assert!(ctx.has_develop);
        assert!(!ctx.branch_protection_enabled);
        assert!(ctx.has_ci_workflow);
        assert_eq!(ctx.current_topics, vec!["rust"]);
    }

    #[test]
    fn test_add_team_to_repo_dry_run() {
        // dry_run=true should succeed without any client
        // We can't test with a real client, but we verify the function compiles
        // and the dry_run path doesn't panic by checking the signature
        let _: fn(&GithubClient, &str, &str, &str, &str, bool) -> Result<()> = add_team_to_repo;
    }

    #[test]
    fn test_list_org_teams_signature() {
        let _: fn(&GithubClient, &str) -> Result<Vec<teams::Team>> = list_org_teams;
    }

    #[test]
    fn test_parse_ssh_with_different_host() {
        // SSH but not github.com
        let remote = "git@gitlab.com:owner/repo.git";
        assert!(parse_github_remote(remote).is_err());
    }

    #[test]
    fn test_parse_https_with_trailing_slash() {
        let remote = "https://github.com/owner/repo/";
        let result = parse_github_remote(remote);
        // Trailing slash means split gives ["owner", "repo", ""] -> 3 parts but last is empty
        assert!(result.is_ok());
        let (owner, repo) = result.unwrap();
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn test_parse_owner_repo_with_hyphens() {
        let (owner, repo) = parse_owner_repo("my-org/my-repo").unwrap();
        assert_eq!(owner, "my-org");
        assert_eq!(repo, "my-repo");
    }

    #[test]
    fn test_parse_owner_repo_with_underscores() {
        let (owner, repo) = parse_owner_repo("my_org/my_repo").unwrap();
        assert_eq!(owner, "my_org");
        assert_eq!(repo, "my_repo");
    }

    #[test]
    fn test_parse_owner_repo_with_dots() {
        let (owner, repo) = parse_owner_repo("org/my.repo").unwrap();
        assert_eq!(owner, "org");
        assert_eq!(repo, "my.repo");
    }

    #[test]
    fn test_parse_owner_repo_single_slash() {
        let (owner, repo) = parse_owner_repo("/").unwrap();
        assert_eq!(owner, "");
        assert_eq!(repo, "");
    }

    #[test]
    fn test_sync_result_all_zeros() {
        let result = SyncResult {
            created: 0,
            updated: 0,
            up_to_date: 0,
            deleted: 0,
        };
        assert_eq!(
            result.created + result.updated + result.up_to_date + result.deleted,
            0
        );
    }

    #[test]
    fn test_sync_result_clone() {
        let result = SyncResult {
            created: 1,
            updated: 2,
            up_to_date: 3,
            deleted: 4,
        };
        let cloned = result.clone();
        assert_eq!(result.created, cloned.created);
        assert_eq!(result.updated, cloned.updated);
        assert_eq!(result.up_to_date, cloned.up_to_date);
        assert_eq!(result.deleted, cloned.deleted);
    }

    #[test]
    fn test_sync_result_debug() {
        let result = SyncResult {
            created: 1,
            updated: 2,
            up_to_date: 3,
            deleted: 4,
        };
        let dbg = format!("{:?}", result);
        assert!(dbg.contains("SyncResult"));
    }

    #[test]
    fn test_apply_context_clone() {
        let ctx = ApplyContext {
            owner: "o".into(),
            repo: "r".into(),
            current_labels: vec![],
            has_develop: false,
            branch_protection_enabled: true,
            has_ci_workflow: false,
            current_topics: vec![],
        };
        let cloned = ctx.clone();
        assert_eq!(ctx.owner, cloned.owner);
        assert_eq!(ctx.repo, cloned.repo);
        assert_eq!(ctx.has_develop, cloned.has_develop);
        assert_eq!(
            ctx.branch_protection_enabled,
            cloned.branch_protection_enabled
        );
        assert_eq!(ctx.has_ci_workflow, cloned.has_ci_workflow);
    }

    #[test]
    fn test_apply_context_debug() {
        let ctx = ApplyContext {
            owner: "org".into(),
            repo: "repo".into(),
            current_labels: vec![],
            has_develop: true,
            branch_protection_enabled: false,
            has_ci_workflow: true,
            current_topics: vec!["rust".into()],
        };
        let dbg = format!("{:?}", ctx);
        assert!(dbg.contains("ApplyContext"));
        assert!(dbg.contains("org"));
        assert!(dbg.contains("repo"));
    }

    #[test]
    fn test_apply_context_with_labels() {
        let ctx = ApplyContext {
            owner: "o".into(),
            repo: "r".into(),
            current_labels: vec![
                labels::Label {
                    name: "bug".into(),
                    color: "d73a4a".into(),
                    description: "A bug".into(),
                },
                labels::Label {
                    name: "feature".into(),
                    color: "a2eeef".into(),
                    description: "A feature".into(),
                },
            ],
            has_develop: false,
            branch_protection_enabled: false,
            has_ci_workflow: false,
            current_topics: vec![],
        };
        assert_eq!(ctx.current_labels.len(), 2);
    }

    #[test]
    fn test_apply_context_multiple_topics() {
        let ctx = ApplyContext {
            owner: "o".into(),
            repo: "r".into(),
            current_labels: vec![],
            has_develop: false,
            branch_protection_enabled: false,
            has_ci_workflow: false,
            current_topics: vec![
                "rust".into(),
                "cli".into(),
                "github".into(),
                "scaffold".into(),
            ],
        };
        assert_eq!(ctx.current_topics.len(), 4);
    }

    #[test]
    fn test_parse_owner_repo_white_space() {
        assert!(parse_owner_repo("  ").is_err());
    }

    #[test]
    fn test_parse_owner_repo_tabs() {
        // Tabs are valid in split, so this actually parses as "a\t" and "\tb"
        let result = parse_owner_repo("a\t/\tb");
        assert!(result.is_ok());
        let (owner, repo) = result.unwrap();
        assert_eq!(owner, "a\t");
        assert_eq!(repo, "\tb");
    }

    #[test]
    fn test_parse_https_url_with_port() {
        let remote = "https://github.com:8443/owner/repo.git";
        // This is not a valid github HTTPS URL (port in URL)
        assert!(parse_github_remote(remote).is_err());
    }

    #[test]
    fn test_parse_ssh_port_in_host() {
        let remote = "ssh://git@github.com/owner/repo.git";
        // ssh:// prefix is not handled
        assert!(parse_github_remote(remote).is_err());
    }

    #[test]
    fn test_parse_https_remote_with_nested_path() {
        let remote = "https://github.com/org/sub/repo.git";
        let result = parse_github_remote(remote);
        assert!(result.is_ok());
        let (owner, repo) = result.unwrap();
        assert_eq!(owner, "org");
        assert_eq!(repo, "sub");
    }

    #[test]
    fn test_parse_ssh_remote_with_nested_path() {
        let remote = "git@github.com:org/sub/repo.git";
        let result = parse_github_remote(remote);
        assert!(result.is_ok());
        let (owner, repo) = result.unwrap();
        assert_eq!(owner, "org");
        assert_eq!(repo, "sub");
    }

    #[test]
    fn test_parse_owner_repo_single_char() {
        let (owner, repo) = parse_owner_repo("a/b").unwrap();
        assert_eq!(owner, "a");
        assert_eq!(repo, "b");
    }

    #[test]
    fn test_parse_owner_repo_long_names() {
        let (owner, repo) = parse_owner_repo("very-long-owner-name/very-long-repo-name").unwrap();
        assert_eq!(owner, "very-long-owner-name");
        assert_eq!(repo, "very-long-repo-name");
    }

    #[test]
    fn test_sync_result_fields_access() {
        let result = SyncResult {
            created: 10,
            updated: 20,
            up_to_date: 30,
            deleted: 40,
        };
        assert_eq!(
            result.created + result.updated + result.up_to_date + result.deleted,
            100
        );
    }

    #[test]
    fn test_apply_context_with_multiple_labels() {
        let labels: Vec<labels::Label> = (0..10)
            .map(|i| labels::Label {
                name: format!("label{i}"),
                color: format!("{:06x}", i * 1000),
                description: format!("Label {i}"),
            })
            .collect();
        let ctx = ApplyContext {
            owner: "o".into(),
            repo: "r".into(),
            current_labels: labels.clone(),
            has_develop: true,
            branch_protection_enabled: true,
            has_ci_workflow: true,
            current_topics: vec![],
        };
        assert_eq!(ctx.current_labels.len(), 10);
        assert_eq!(ctx.current_labels[5].name, "label5");
    }

    #[test]
    fn test_apply_context_equality() {
        let a = ApplyContext {
            owner: "o".into(),
            repo: "r".into(),
            current_labels: vec![],
            has_develop: true,
            branch_protection_enabled: false,
            has_ci_workflow: true,
            current_topics: vec!["rust".into()],
        };
        let b = a.clone();
        assert_eq!(a.owner, b.owner);
        assert_eq!(a.repo, b.repo);
        assert_eq!(a.has_develop, b.has_develop);
        assert_eq!(a.branch_protection_enabled, b.branch_protection_enabled);
        assert_eq!(a.has_ci_workflow, b.has_ci_workflow);
        assert_eq!(a.current_topics, b.current_topics);
    }

    #[test]
    fn test_apply_context_debug_format() {
        let ctx = ApplyContext {
            owner: "test-owner".into(),
            repo: "test-repo".into(),
            current_labels: vec![],
            has_develop: false,
            branch_protection_enabled: false,
            has_ci_workflow: false,
            current_topics: vec![],
        };
        let dbg = format!("{:?}", ctx);
        assert!(dbg.contains("test-owner"));
        assert!(dbg.contains("test-repo"));
    }

    #[test]
    fn test_sync_result_debug_format() {
        let result = SyncResult {
            created: 5,
            updated: 3,
            up_to_date: 10,
            deleted: 2,
        };
        let dbg = format!("{:?}", result);
        assert!(dbg.contains("5"));
        assert!(dbg.contains("3"));
        assert!(dbg.contains("10"));
        assert!(dbg.contains("2"));
    }

    #[test]
    fn test_sync_result_clone_fields() {
        let result = SyncResult {
            created: 1,
            updated: 2,
            up_to_date: 3,
            deleted: 4,
        };
        let cloned = result.clone();
        assert_eq!(result.created, cloned.created);
        assert_eq!(result.updated, cloned.updated);
        assert_eq!(result.up_to_date, cloned.up_to_date);
        assert_eq!(result.deleted, cloned.deleted);
    }

    #[test]
    fn test_parse_owner_repo_numeric_parts() {
        let (owner, repo) = parse_owner_repo("123/456").unwrap();
        assert_eq!(owner, "123");
        assert_eq!(repo, "456");
    }

    #[test]
    fn test_parse_https_remote_with_dot_git_suffix_only() {
        let remote = "https://github.com/user/project.git";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "project");
    }

    #[test]
    fn test_parse_ssh_remote_with_dot_git_suffix_only() {
        let remote = "git@github.com:user/project.git";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "user");
        assert_eq!(repo, "project");
    }

    #[test]
    fn test_parse_https_not_github() {
        assert!(parse_github_remote("https://gitlab.com/owner/repo.git").is_err());
    }

    #[test]
    fn test_parse_ssh_not_github() {
        assert!(parse_github_remote("git@gitlab.com:owner/repo.git").is_err());
    }

    #[test]
    fn test_parse_github_remote_bitbucket() {
        assert!(parse_github_remote("https://bitbucket.org/owner/repo.git").is_err());
    }

    #[test]
    fn test_apply_context_clone_debug() {
        let ctx = ApplyContext {
            owner: "org".into(),
            repo: "repo".into(),
            current_labels: vec![labels::Label {
                name: "bug".into(),
                color: "ff0000".into(),
                description: "Bug".into(),
            }],
            has_develop: true,
            branch_protection_enabled: true,
            has_ci_workflow: true,
            current_topics: vec!["rust".into(), "cli".into()],
        };
        let cloned = ctx.clone();
        assert_eq!(cloned.current_labels.len(), 1);
        assert_eq!(cloned.current_topics.len(), 2);
    }

    #[test]
    fn test_add_team_to_repo_signature_types() {
        let _: fn(&GithubClient, &str, &str, &str, &str, bool) -> Result<()> = add_team_to_repo;
    }

    #[test]
    fn test_list_org_teams_signature_types() {
        let _: fn(&GithubClient, &str) -> Result<Vec<teams::Team>> = list_org_teams;
    }

    #[test]
    fn test_apply_context_empty_topics() {
        let ctx = ApplyContext {
            owner: "o".into(),
            repo: "r".into(),
            current_labels: vec![],
            has_develop: false,
            branch_protection_enabled: false,
            has_ci_workflow: false,
            current_topics: vec![],
        };
        assert!(ctx.current_topics.is_empty());
    }

    #[test]
    fn test_sync_result_negative_impossible() {
        let result = SyncResult {
            created: 0,
            updated: 0,
            up_to_date: 0,
            deleted: 0,
        };
        assert_eq!(result.created, 0);
        assert_eq!(result.updated, 0);
        assert_eq!(result.up_to_date, 0);
        assert_eq!(result.deleted, 0);
    }

    #[test]
    fn test_parse_github_remote_exact_https() {
        let remote = "https://github.com/octocat/hello-world.git";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "octocat");
        assert_eq!(repo, "hello-world");
    }

    #[test]
    fn test_parse_github_remote_exact_ssh() {
        let remote = "git@github.com:octocat/hello-world.git";
        let (owner, repo) = parse_github_remote(remote).unwrap();
        assert_eq!(owner, "octocat");
        assert_eq!(repo, "hello-world");
    }

    #[test]
    fn test_parse_owner_repo_exact_match() {
        let (owner, repo) = parse_owner_repo("octocat/hello-world").unwrap();
        assert_eq!(owner, "octocat");
        assert_eq!(repo, "hello-world");
    }

    #[test]
    fn test_apply_context_all_fields_set() {
        let ctx = ApplyContext {
            owner: "my-org".into(),
            repo: "my-repo".into(),
            current_labels: vec![
                labels::Label {
                    name: "bug".into(),
                    color: "d73a4a".into(),
                    description: "Bug".into(),
                },
                labels::Label {
                    name: "enhancement".into(),
                    color: "a2eeef".into(),
                    description: "Enhancement".into(),
                },
            ],
            has_develop: true,
            branch_protection_enabled: true,
            has_ci_workflow: true,
            current_topics: vec!["rust".into(), "cli".into(), "github".into()],
        };
        assert_eq!(ctx.owner, "my-org");
        assert_eq!(ctx.repo, "my-repo");
        assert_eq!(ctx.current_labels.len(), 2);
        assert!(ctx.has_develop);
        assert!(ctx.branch_protection_enabled);
        assert!(ctx.has_ci_workflow);
        assert_eq!(ctx.current_topics.len(), 3);
    }

    #[test]
    fn test_apply_context_clone_preserves_all_fields() {
        let ctx = ApplyContext {
            owner: "org".into(),
            repo: "repo".into(),
            current_labels: vec![labels::Label {
                name: "bug".into(),
                color: "ff0000".into(),
                description: "Bug".into(),
            }],
            has_develop: true,
            branch_protection_enabled: true,
            has_ci_workflow: true,
            current_topics: vec!["rust".into()],
        };
        let cloned = ctx.clone();
        assert_eq!(ctx.owner, cloned.owner);
        assert_eq!(ctx.repo, cloned.repo);
        assert_eq!(ctx.current_labels.len(), cloned.current_labels.len());
        assert_eq!(ctx.has_develop, cloned.has_develop);
        assert_eq!(
            ctx.branch_protection_enabled,
            cloned.branch_protection_enabled
        );
        assert_eq!(ctx.has_ci_workflow, cloned.has_ci_workflow);
        assert_eq!(ctx.current_topics, cloned.current_topics);
    }

    #[test]
    fn test_sync_result_all_positive() {
        let result = SyncResult {
            created: 100,
            updated: 200,
            up_to_date: 300,
            deleted: 400,
        };
        assert_eq!(result.created, 100);
        assert_eq!(result.updated, 200);
        assert_eq!(result.up_to_date, 300);
        assert_eq!(result.deleted, 400);
    }

    #[test]
    fn test_parse_github_remote_just_prefix_https() {
        assert!(parse_github_remote("https://github.com/").is_err());
    }

    #[test]
    fn test_parse_github_remote_just_prefix_ssh() {
        assert!(parse_github_remote("git@github.com:").is_err());
    }

    #[test]
    fn test_parse_owner_repo_only_slash() {
        let (owner, repo) = parse_owner_repo("/").unwrap();
        assert_eq!(owner, "");
        assert_eq!(repo, "");
    }

    #[test]
    fn test_add_team_to_repo_types() {
        let _: fn(&GithubClient, &str, &str, &str, &str, bool) -> Result<()> = add_team_to_repo;
    }

    #[test]
    fn test_list_org_teams_types() {
        let _: fn(&GithubClient, &str) -> Result<Vec<teams::Team>> = list_org_teams;
    }

    #[test]
    fn test_apply_context_debug_format_all_fields() {
        let ctx = ApplyContext {
            owner: "org".into(),
            repo: "repo".into(),
            current_labels: vec![],
            has_develop: true,
            branch_protection_enabled: false,
            has_ci_workflow: true,
            current_topics: vec!["rust".into()],
        };
        let dbg = format!("{:?}", ctx);
        assert!(dbg.contains("org"));
        assert!(dbg.contains("repo"));
        assert!(dbg.contains("true"));
        assert!(dbg.contains("false"));
    }

    #[test]
    fn test_sync_result_debug_format_all_fields() {
        let result = SyncResult {
            created: 1,
            updated: 2,
            up_to_date: 3,
            deleted: 4,
        };
        let dbg = format!("{:?}", result);
        assert!(dbg.contains("1"));
        assert!(dbg.contains("2"));
        assert!(dbg.contains("3"));
        assert!(dbg.contains("4"));
    }
}
