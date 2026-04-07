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

    for std_label in standard {
        if let Some(existing) = current.iter().find(|l| l.name == std_label.name) {
            // Check if needs update
            if existing.color != std_label.color || existing.description != std_label.description {
                if !dry_run {
                    labels::update_label(client, owner, repo_name, &std_label.name, &std_label)?;
                }
                updated += 1;
            } else {
                up_to_date += 1;
            }
        } else {
            // Create new label
            if !dry_run {
                labels::create_label(client, owner, repo_name, &std_label)?;
            }
            created += 1;
        }
    }

    Ok(SyncResult {
        created,
        updated,
        up_to_date,
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
}

/// Main apply mode orchestrator
pub fn run_apply(repo_arg: Option<&str>, dry_run: bool) -> Result<()> {
    // Get token
    let token = crate::github::client::token_from_env()?;
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
    if label_result.created > 0 || label_result.updated > 0 {
        println!(
            "    • {} to create, {} to update, {} up to date",
            label_result.created, label_result.updated, label_result.up_to_date
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
                let team_names: Vec<String> = org_teams
                    .iter()
                    .map(|t| format!("{} ({})", t.name, t.slug))
                    .collect();

                if let Ok(Some(selections)) =
                    inquire::MultiSelect::new("Select teams:", team_names.clone())
                        .with_help_message("space select  enter confirm")
                        .prompt_skippable()
                {
                    for selected_team_display in selections {
                        if let Some(team) = org_teams.iter().find(|t| {
                            format!("{} ({})", t.name, t.slug) == selected_team_display
                        }) {
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

    // 2. Branch protection (skip if already enabled, warn on failure)
    if !ctx.branch_protection_enabled {
        match crate::github::branches::apply_branch_protection(
            &client, &owner, &repo_name, "main", "CI",
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
    } else {
        println!("  ✓ Branch protection (already enabled)");
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

    // 6. Secrets from template
    let secret_specs = crate::templates::load_secrets("rust");
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
                if let Ok(env_val) = std::env::var(&spec.name) {
                    match secrets::set_secret(&client, &owner, &repo_name, &spec.name, &env_val) {
                        Ok(()) => {
                            println!("  ✓ Secret {} configured (from environment)", spec.name)
                        }
                        Err(e) => println!("  ⚠ Failed to set {}: {e:#}", spec.name),
                    }
                } else {
                    let ans =
                        inquire::Password::new(&format!("Secret {} (enter to skip):", spec.name))
                            .with_help_message(&spec.description)
                            .without_confirmation()
                            .prompt_skippable()?;
                    match ans.as_deref() {
                        Some(v) if !v.is_empty() => {
                            match secrets::set_secret(&client, &owner, &repo_name, &spec.name, v) {
                                Ok(()) => println!("  ✓ Secret {} configured", spec.name),
                                Err(e) => println!("  ⚠ Failed to set {}: {e:#}", spec.name),
                            }
                        }
                        _ => println!(
                            "  ⚠ Secret {} skipped — set ${} and re-run `ghscaff apply`",
                            spec.name, spec.name
                        ),
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
        };
        assert_eq!(result.created, 2);
        assert_eq!(result.updated, 1);
        assert_eq!(result.up_to_date, 9);
    }
}
