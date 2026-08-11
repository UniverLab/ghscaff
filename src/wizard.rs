use anyhow::Result;
use inquire::{Confirm, MultiSelect, Password, Select, Text};

use crate::github::{
    branches,
    client::{resolve_token, GithubClient},
    contents, labels, repo, secrets, sponsors, teams,
};
use crate::templates;

const BANNER: &str = r#"
          █████       █████████                        ██████     ██████ 
         ░░███       ███░░░░░███                      ███░░███   ███░░███
  ███████ ░███████  ░███    ░░░   ██████   ██████    ░███ ░░░   ░███ ░░░ 
 ███░░███ ░███░░███ ░░█████████  ███░░███ ░░░░░███  ███████    ███████   
░███ ░███ ░███ ░███  ░░░░░░░░███░███ ░░░   ███████ ░░░███░    ░░░███░    
░███ ░███ ░███ ░███  ███    ░███░███  ███ ███░░███   ░███       ░███     
░░███████ ████ █████░░█████████ ░░██████ ░░████████  █████      █████    
 ░░░░░███░░░░ ░░░░░  ░░░░░░░░░   ░░░░░░   ░░░░░░░░  ░░░░░      ░░░░░     
 ███ ░███                                                                
░░██████                                                                 
 ░░░░░░                                                                  
"#;

pub struct WizardConfig {
    pub name: String,
    pub description: String,
    #[allow(dead_code)]
    pub topics: Vec<String>,
    pub private: bool,
    pub owner: String,
    pub is_org: bool,
    pub language: Option<String>,
    pub default_branch: String,
    pub create_develop: bool,
    pub license: Option<String>,
    pub create_labels: bool,
    pub team_access: Vec<teams::TeamAccess>,
}

pub fn run(dry_run: bool) -> Result<()> {
    println!("{BANNER}");
    println!("  Create a new GitHub repository\n");

    // Fail fast — validate token before asking anything
    let (token, passphrase) = resolve_token()?;
    let client = GithubClient::new(&token);

    print!("  Validating token... ");
    let user = repo::get_user(&client)?;
    println!("ok  ({})", user.login);
    println!();

    let config = collect_config(&client, &user.login)?;

    println!();
    let confirmed = Confirm::new("Apply these changes?")
        .with_default(true)
        .prompt()?;

    if !confirmed {
        println!("  Aborted.");
        return Ok(());
    }

    execute(&client, &config, dry_run, &token, &passphrase)
}

fn collect_team_access(client: &GithubClient, _org: &str) -> Result<Vec<teams::TeamAccess>> {
    print!("  Fetching teams... ");
    let org_teams = teams::list_teams(client).unwrap_or_else(|_| {
        eprintln!();
        eprintln!("  ⚠  Could not list teams (token may need 'read:org' scope)");
        vec![]
    });
    println!("ok");

    if org_teams.is_empty() {
        println!("  ℹ  No teams found in organization");
        return Ok(vec![]);
    }

    let team_names: Vec<String> = org_teams.iter().map(|t| t.name.clone()).collect();

    let selected_teams = MultiSelect::new("Select teams to add:", team_names)
        .with_help_message("space select  enter confirm  (leave empty for no teams)")
        .prompt_skippable()?
        .unwrap_or_default();

    if selected_teams.is_empty() {
        return Ok(vec![]);
    }

    let mut team_access = vec![];

    for selected_team_display in selected_teams {
        let team = org_teams
            .iter()
            .find(|t| t.name == selected_team_display)
            .unwrap();

        let permission = Select::new(
            &format!("Permission for {} team:", team.name),
            vec!["pull", "triage", "push", "admin"],
        )
        .prompt()?;

        team_access.push(teams::TeamAccess {
            team_slug: team.slug.clone(),
            permission: permission.to_string(),
        });
    }

    Ok(team_access)
}

fn collect_config(client: &GithubClient, username: &str) -> Result<WizardConfig> {
    // Step 1 — Repository basics
    let name = Text::new("Repository name:")
        .with_validator(|input: &str| {
            if input.trim().is_empty() {
                Ok(inquire::validator::Validation::Invalid(
                    "Repository name cannot be empty".into(),
                ))
            } else {
                Ok(inquire::validator::Validation::Valid)
            }
        })
        .prompt()?;
    let description = Text::new("Description:").with_default("").prompt()?;
    let topics_raw = Text::new("Topics:")
        .with_default("")
        .with_help_message("comma-separated, e.g. rust,cli,tool")
        .prompt()?;
    let topics: Vec<String> = topics_raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    // Step 2 — Visibility & ownership
    let visibility = Select::new("Visibility:", vec!["Public", "Private"]).prompt()?;
    let private = visibility == "Private";

    let mut owner_options = vec![username.to_string()];
    let orgs = repo::list_orgs(client).unwrap_or_else(|_| {
        eprintln!("  ⚠  Could not list orgs (token may need 'read:org' scope)");
        vec![]
    });
    for org in &orgs {
        owner_options.push(org.login.clone());
    }

    let owner_selection = Select::new("Owner:", owner_options).prompt()?;
    let (owner, is_org) = if owner_selection == username {
        (owner_selection, false)
    } else {
        (owner_selection, true)
    };

    // Step 2.5 — Team access (only for organizations)
    let team_access = if is_org {
        collect_team_access(client, &owner)?
    } else {
        vec![]
    };

    // Step 3 — Language. Read the live list from the boilerplate repo so new
    // templates appear without a new ghscaff release.
    let mut lang_options: Vec<String> = templates::available(client);
    lang_options.push("none".to_string());
    let lang_choice = Select::new("Template:", lang_options)
        .with_help_message("Drives .gitignore, CI workflow, and boilerplate. 'none' = empty repo")
        .prompt()?;
    let language = if lang_choice == "none" {
        None
    } else {
        Some(lang_choice)
    };

    // Step 4 — Branches
    let default_branch = Text::new("Default branch:").with_default("main").prompt()?;
    let create_develop = Confirm::new("Create develop branch?")
        .with_default(true)
        .prompt()?;

    // Step 5 — Features
    let feature_items = vec!["LICENSE", "Standard labels"];
    let feature_defaults = vec![0usize, 1];
    let features = MultiSelect::new("Features:", feature_items.clone())
        .with_default(&feature_defaults)
        .with_help_message("space select  enter confirm")
        .prompt()?;

    let license = if features.contains(&"LICENSE") {
        let lic = Select::new("License:", vec!["MIT", "Apache-2.0", "GPL-3.0", "None"]).prompt()?;
        if lic == "None" {
            None
        } else {
            Some(lic.to_string())
        }
    } else {
        None
    };

    Ok(WizardConfig {
        name,
        description,
        topics,
        private,
        owner,
        is_org,
        language,
        default_branch,
        create_develop,
        license,
        create_labels: features.contains(&"Standard labels"),
        team_access,
    })
}

fn execute(
    client: &GithubClient,
    c: &WizardConfig,
    dry_run: bool,
    token: &str,
    passphrase: &str,
) -> Result<()> {
    println!();

    // Fetch template if selected
    let tmpl = if let Some(lang) = &c.language {
        print!("  Fetching boilerplate template... ");
        let t = templates::resolve(lang, token, true)?;
        println!("ok");
        Some(t)
    } else {
        None
    };
    let secret_specs = c
        .language
        .as_deref()
        .map(templates::load_secrets)
        .unwrap_or_default();
    let total = count_steps(c, &secret_specs);
    let mut step = 0usize;

    macro_rules! step {
        ($msg:expr, $op:expr) => {{
            step += 1;
            if dry_run {
                println!("  [{step}/{total}] [dry-run] {}", $msg);
            } else {
                print!("  [{step}/{total}] {}... ", $msg);
                $op?;
                println!("ok");
            }
        }};
    }

    // 1. Create repo (empty — initial commit via Trees API below)
    let created_repo = if dry_run {
        step += 1;
        println!(
            "  [{step}/{total}] [dry-run] create repo {}/{}",
            c.owner, c.name
        );
        None
    } else {
        print!(
            "  [{}/{total}] create repo {}/{}... ",
            step + 1,
            c.owner,
            c.name
        );
        step += 1;
        let r = repo::create_repo(
            client,
            &c.owner,
            &c.name,
            &c.description,
            c.private,
            c.is_org,
        )?;
        println!("ok  ({})", r.html_url);
        Some(r)
    };

    let owner = &c.owner;
    let name = &c.name;

    // 2. Collect all boilerplate files for a single init commit
    let mut init_files: Vec<contents::TreeFile> = vec![];

    if let Some(tmpl) = &tmpl {
        for f in tmpl.boilerplate_files(name, &c.description, owner) {
            init_files.push(contents::TreeFile {
                path: f.path,
                content: f.content,
            });
        }

        let gitignore = repo::get_gitignore_template(client, &tmpl.gitignore_name())?;
        init_files.push(contents::TreeFile {
            path: ".gitignore".into(),
            content: gitignore,
        });
    }

    // LICENSE — full text from GitHub's license API, with copyright placeholders
    // filled in (owner + current year). MIT uses [year]/[fullname]; the Apache
    // and GPL appendices use [yyyy]/[name of copyright owner].
    if let Some(lic) = &c.license {
        let year = current_year();
        let license_text = repo::get_license_template(client, &lic.to_lowercase())?
            .replace("[year]", &year)
            .replace("[yyyy]", &year)
            .replace("[fullname]", owner)
            .replace("[name of copyright owner]", owner);
        init_files.push(contents::TreeFile {
            path: "LICENSE".into(),
            content: license_text,
        });
    }

    // 3. Single init commit with all files (skip if empty repo with no LICENSE)
    let mut init_sha = String::new();
    if !init_files.is_empty() {
        step!("init repository", {
            let sha = contents::create_tree_commit(
                client,
                owner,
                name,
                &init_files,
                "chore: init repository",
                &c.default_branch,
            )?;
            init_sha = sha;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 4. develop branch
    if c.create_develop {
        if init_sha.is_empty() {
            init_sha = branches::get_branch_sha(client, owner, name, &c.default_branch)?;
        }
        step!("create develop branch", {
            branches::create_branch(client, owner, name, "develop", &init_sha)?;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 5. Branch protection — required contexts are derived from the workflow
    // files just committed, never hardcoded, so a renamed job can't leave a
    // required check pointing at a name nothing will ever report.
    let workflow_sources: Vec<crate::checks::WorkflowSource> = init_files
        .iter()
        .map(|f| crate::checks::WorkflowSource {
            path: &f.path,
            content: &f.content,
        })
        .collect();
    let required_contexts = crate::checks::derive_required_contexts(&workflow_sources);
    step!(
        &format!("apply branch protection ({})", c.default_branch),
        {
            branches::apply_branch_protection(
                client,
                owner,
                name,
                &c.default_branch,
                &required_contexts,
            )?;
            Ok::<(), anyhow::Error>(())
        }
    );
    if c.create_develop {
        step!("apply branch protection (develop)", {
            branches::apply_branch_protection(client, owner, name, "develop", &required_contexts)?;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 6. Labels
    if c.create_labels {
        step!("sync labels", {
            let existing = labels::list_labels(client, owner, name)?;
            let standard = labels::standard_labels();
            for label in &standard {
                if existing.iter().any(|e| e.name == label.name) {
                    labels::update_label(client, owner, name, &label.name, label)?;
                } else {
                    labels::create_label(client, owner, name, label)?;
                }
            }
            for existing_label in &existing {
                if !standard.iter().any(|s| s.name == existing_label.name) {
                    let _ = labels::delete_label(client, owner, name, &existing_label.name);
                }
            }
            Ok::<(), anyhow::Error>(())
        });
    }

    // 7. Topics
    if !c.topics.is_empty() {
        step!("set topics", {
            repo::set_topics(client, owner, name, &c.topics)?;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 8. Team access
    for team in &c.team_access {
        step!(
            &format!(
                "add team {} with {} access",
                team.team_slug, team.permission
            ),
            {
                teams::add_team_to_repo(client, owner, name, &team.team_slug, &team.permission)?;
                Ok::<(), anyhow::Error>(())
            }
        );
    }

    // 9. Template secrets: env → vault → prompt
    for spec in &secret_specs {
        let value = if let Some(val) = crate::vault::resolve_secret(&spec.name, passphrase)? {
            println!("  ◆ Secret {}: found", spec.name);
            Some(val)
        } else {
            prompt_secret_value(spec, passphrase)?
        };
        if let Some(val) = value {
            step!(&format!("configure secret {}", spec.name), {
                secrets::set_secret(client, owner, name, &spec.name, &val)?;
                Ok::<(), anyhow::Error>(())
            });
        } else {
            step += 1; // keep total consistent even when skipped
        }
    }

    println!();
    if let Some(r) = &created_repo {
        println!("  Done  —  {}", r.html_url);
    } else {
        println!("  Done  (dry-run)");
    }
    println!();

    if !dry_run {
        if let Some(_r) = &created_repo {
            offer_gitkit_clone(owner, name);
            offer_sponsor_button(client, owner, name);
        }
    }

    Ok(())
}

/// Which way the user answered the Sponsor-button prompt, or that there was
/// no way to ask (non-interactive run — no TTY, or an explicit cancel).
enum SponsorAnswer {
    Yes,
    No,
    Skipped,
}

fn ask_sponsor_button() -> SponsorAnswer {
    match Confirm::new("Enable the Sponsor button for this repository?")
        .with_default(false)
        .prompt()
    {
        Ok(true) => SponsorAnswer::Yes,
        Ok(false) => SponsorAnswer::No,
        Err(_) => SponsorAnswer::Skipped,
    }
}

/// Asks whether to enable the GitHub Sponsor button and, only on an explicit
/// "yes", calls GS1's `enable_sponsorships`. A non-interactive run (no TTY)
/// or a "no" answer leaves the repository untouched — the repo is already
/// created and configured, so this step never fails the scaffold.
fn offer_sponsor_button(client: &GithubClient, owner: &str, name: &str) {
    apply_sponsor_answer(ask_sponsor_button(), || {
        sponsors::enable_sponsorships(client, owner, name)
    });
}

fn apply_sponsor_answer(
    answer: SponsorAnswer,
    enable: impl FnOnce() -> Result<sponsors::SponsorshipStatus>,
) {
    if !matches!(answer, SponsorAnswer::Yes) {
        return;
    }
    print!("  Enabling Sponsor button... ");
    println!("{}", sponsor_outcome_message(&enable()));
}

fn sponsor_outcome_message(result: &Result<sponsors::SponsorshipStatus>) -> String {
    match result {
        Ok(sponsors::SponsorshipStatus::Enabled) => "ok".to_string(),
        Ok(sponsors::SponsorshipStatus::AlreadyEnabled) => "ok  (already enabled)".to_string(),
        Err(e) => format!("failed: {e}"),
    }
}

fn offer_gitkit_clone(owner: &str, repo: &str) {
    let Ok(want_clone) = Confirm::new("Clone the repository with gitkit?")
        .with_default(true)
        .prompt()
    else {
        return;
    };
    if !want_clone {
        return;
    }

    let protocol = Select::new("Clone via:", vec!["SSH", "HTTPS"])
        .prompt()
        .unwrap_or("HTTPS");

    let clone_url = match protocol {
        "SSH" => format!("git@github.com:{owner}/{repo}.git"),
        _ => format!("https://github.com/{owner}/{repo}.git"),
    };

    if !is_command_available("gitkit") {
        println!("  gitkit not found. Installing...\n");
        install_gitkit();
        if !is_command_available("gitkit") {
            println!("  ⚠ gitkit installation failed — clone manually with:");
            println!("    gitkit clone {clone_url}");
            return;
        }
    }

    let _ = std::process::Command::new("gitkit")
        .args(["clone", &clone_url])
        .status();
}

fn is_command_available(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn install_gitkit() {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("sh")
            .args([
                "-c",
                "curl -fsSL https://raw.githubusercontent.com/UniverLab/gitkit/main/scripts/install.sh | sh",
            ])
            .status();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("powershell")
            .args([
                "-Command",
                "irm https://raw.githubusercontent.com/UniverLab/gitkit/main/scripts/install.ps1 | iex",
            ])
            .status();
    }
}

/// Prompt the user for a template secret that wasn't found in env/vault.
///
/// A required secret re-prompts on an empty entry instead of silently skipping —
/// this both enforces the requirement and defends against a stray buffered newline
/// (left over from earlier prompts) auto-submitting an empty value before the user
/// can type. Skipping a required secret takes an explicit confirmation. An explicit
/// cancel (ESC) or a non-interactive stdin returns `None` without looping.
pub(crate) fn prompt_secret_value(
    spec: &templates::SecretSpec,
    passphrase: &str,
) -> Result<Option<String>> {
    let skipped = || {
        println!(
            "  ⚠ Secret {} not configured — re-run `ghscaff apply` to set it later",
            spec.name
        );
    };
    loop {
        let ans = Password::new(&format!("Secret {}:", spec.name))
            .with_help_message(&spec.description)
            .without_confirmation()
            .prompt_skippable()?;
        match ans.as_deref() {
            Some(v) if !v.is_empty() => {
                let save_it = Confirm::new("  Save this secret in the vault for future use?")
                    .with_default(true)
                    .prompt()
                    .unwrap_or(false);
                if save_it {
                    crate::vault::save_secret(&spec.name, v, passphrase)?;
                    println!("  \x1b[32m✓\x1b[0m Secret saved to vault");
                }
                return Ok(Some(v.to_string()));
            }
            // Empty entry on a required secret — likely an accidental/auto-submitted
            // skip. Ask for explicit intent; re-prompt unless the user confirms skip.
            Some(_) if spec.required => {
                let skip = Confirm::new(&format!(
                    "  Secret {} is required by this template. Skip and set it later?",
                    spec.name
                ))
                .with_default(false)
                .prompt()
                .unwrap_or(false);
                if skip {
                    skipped();
                    return Ok(None);
                }
                // otherwise loop and prompt again
            }
            // Explicit cancel (None) or an empty optional secret — skip without looping.
            _ => {
                skipped();
                return Ok(None);
            }
        }
    }
}

/// Current calendar year as a string, for the LICENSE copyright line. Uses the
/// average Gregorian year length — accurate to the day, which is fine here and
/// avoids pulling in a date crate.
fn current_year() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (1970 + secs / 31_556_952).to_string()
}

fn count_steps(c: &WizardConfig, secrets: &[templates::SecretSpec]) -> usize {
    let mut n = 1; // create repo
    let has_files = c.language.is_some() || c.license.is_some();
    if has_files {
        n += 1; // init commit
    }
    if c.create_develop {
        n += 1;
        n += 1; // protect develop
    }
    n += 1; // protect main
    if c.create_labels {
        n += 1;
    }
    if !c.topics.is_empty() {
        n += 1;
    }
    n += c.team_access.len();
    n += secrets.len();
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_year_is_sane() {
        let year: u64 = current_year().parse().expect("year should be numeric");
        assert!((2024..=2100).contains(&year), "unexpected year: {year}");
    }

    #[test]
    fn test_wizard_config_with_team_access() {
        let team_access = vec![
            teams::TeamAccess {
                team_slug: "backend".to_string(),
                permission: "push".to_string(),
            },
            teams::TeamAccess {
                team_slug: "devops".to_string(),
                permission: "pull".to_string(),
            },
        ];

        let config = WizardConfig {
            name: "my-repo".to_string(),
            description: "Test repo".to_string(),
            topics: vec!["rust".to_string(), "cli".to_string()],
            private: false,
            owner: "my-org".to_string(),
            is_org: true,
            language: Some("rust".to_string()),
            default_branch: "main".to_string(),
            create_develop: true,
            license: Some("MIT".to_string()),
            create_labels: true,
            team_access,
        };

        assert_eq!(config.name, "my-repo");
        assert_eq!(config.owner, "my-org");
        assert!(config.is_org);
        assert_eq!(config.team_access.len(), 2);
        assert_eq!(config.team_access[0].team_slug, "backend");
        assert_eq!(config.team_access[1].permission, "pull");
    }

    #[test]
    fn test_wizard_config_without_team_access() {
        let config = WizardConfig {
            name: "my-repo".to_string(),
            description: "Test repo".to_string(),
            topics: vec![],
            private: true,
            owner: "my-user".to_string(),
            is_org: false,
            language: Some("rust".to_string()),
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: false,
            team_access: vec![],
        };

        assert!(!config.is_org);
        assert!(config.team_access.is_empty());
    }

    #[test]
    fn test_count_steps_with_teams() {
        let team_access = vec![
            teams::TeamAccess {
                team_slug: "team1".to_string(),
                permission: "push".to_string(),
            },
            teams::TeamAccess {
                team_slug: "team2".to_string(),
                permission: "pull".to_string(),
            },
        ];

        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec!["test".to_string()],
            private: false,
            owner: "org".to_string(),
            is_org: true,
            language: Some("rust".to_string()),
            default_branch: "main".to_string(),
            create_develop: true,
            license: Some("MIT".to_string()),
            create_labels: true,
            team_access,
        };

        // 1: create repo
        // 2: init commit
        // 3: create develop
        // 4: protect develop
        // 5: protect main
        // 6: labels
        // 7: topics
        // 8-9: 2 teams
        // Total: 9 (no secrets)
        let steps = count_steps(&config, &[]);
        assert_eq!(steps, 9);
    }

    #[test]
    fn test_count_steps_without_teams() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec![],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: Some("rust".to_string()),
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: false,
            team_access: vec![],
        };

        // 1: create repo
        // 2: init commit
        // 3: protect main
        // Total: 3 (no develop, no labels, no topics, no teams, no secrets)
        let steps = count_steps(&config, &[]);
        assert_eq!(steps, 3);
    }

    #[test]
    fn test_count_steps_all_features() {
        let team_access = vec![teams::TeamAccess {
            team_slug: "team".to_string(),
            permission: "admin".to_string(),
        }];

        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec!["test".to_string()],
            private: false,
            owner: "org".to_string(),
            is_org: true,
            language: Some("rust".to_string()),
            default_branch: "main".to_string(),
            create_develop: true,
            license: Some("Apache-2.0".to_string()),
            create_labels: true,
            team_access,
        };

        let secret_specs = vec![
            templates::SecretSpec {
                name: "SECRET1".to_string(),
                description: "Test secret".to_string(),
                required: true,
            },
            templates::SecretSpec {
                name: "SECRET2".to_string(),
                description: "Another secret".to_string(),
                required: false,
            },
        ];

        // 1: create repo
        // 2: init commit
        // 3: create develop
        // 4: protect develop
        // 5: protect main
        // 6: labels
        // 7: topics
        // 8: 1 team
        // 9-10: 2 secrets
        // Total: 10
        let steps = count_steps(&config, &secret_specs);
        assert_eq!(steps, 10);
    }

    #[test]
    fn test_is_command_available_existing() {
        assert!(is_command_available("sh"));
    }

    #[test]
    fn test_is_command_available_nonexistent() {
        assert!(!is_command_available(
            "definitely-not-a-real-command-xyz123"
        ));
    }

    #[test]
    fn test_current_year_numeric_value() {
        let year: u64 = current_year().parse().unwrap();
        assert!(year >= 2025);
        assert!(year <= 2200);
    }

    #[test]
    fn test_current_year_length() {
        let y = current_year();
        assert!(y.len() >= 4);
        assert!(y.len() <= 5);
    }

    #[test]
    fn test_count_steps_no_language_no_license() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec![],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: None,
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: false,
            team_access: vec![],
        };
        // 1: create repo
        // 3: protect main
        let steps = count_steps(&config, &[]);
        assert_eq!(steps, 2);
    }

    #[test]
    fn test_count_steps_only_develop() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec![],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: None,
            default_branch: "main".to_string(),
            create_develop: true,
            license: None,
            create_labels: false,
            team_access: vec![],
        };
        // 1: create repo
        // 3: create develop
        // 4: protect develop
        // 5: protect main
        let steps = count_steps(&config, &[]);
        assert_eq!(steps, 4);
    }

    #[test]
    fn test_count_steps_only_labels() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec![],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: None,
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: true,
            team_access: vec![],
        };
        let steps = count_steps(&config, &[]);
        assert_eq!(steps, 3);
    }

    #[test]
    fn test_count_steps_only_topics() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec!["rust".to_string()],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: None,
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: false,
            team_access: vec![],
        };
        let steps = count_steps(&config, &[]);
        assert_eq!(steps, 3);
    }

    #[test]
    fn test_count_steps_only_secrets() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec![],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: None,
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: false,
            team_access: vec![],
        };
        let secrets = vec![templates::SecretSpec {
            name: "KEY".to_string(),
            description: "d".to_string(),
            required: true,
        }];
        let steps = count_steps(&config, &secrets);
        assert_eq!(steps, 3);
    }

    #[test]
    fn test_count_steps_many_teams() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec![],
            private: false,
            owner: "org".to_string(),
            is_org: true,
            language: None,
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: false,
            team_access: (0..5)
                .map(|i| teams::TeamAccess {
                    team_slug: format!("t{i}"),
                    permission: "push".to_string(),
                })
                .collect(),
        };
        let steps = count_steps(&config, &[]);
        assert_eq!(steps, 7);
    }

    #[test]
    fn test_count_steps_many_secrets() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec![],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: None,
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: false,
            team_access: vec![],
        };
        let secrets: Vec<templates::SecretSpec> = (0..3)
            .map(|i| templates::SecretSpec {
                name: format!("S{i}"),
                description: format!("d{i}"),
                required: false,
            })
            .collect();
        let steps = count_steps(&config, &secrets);
        assert_eq!(steps, 5);
    }

    #[test]
    fn test_is_command_available_ls() {
        assert!(is_command_available("ls"));
    }

    #[test]
    fn test_is_command_available_cat() {
        assert!(is_command_available("cat"));
    }

    #[test]
    fn test_is_command_available_with_path_characters() {
        assert!(!is_command_available("/usr/bin/definitely-fake"));
    }

    #[test]
    fn test_wizard_config_clone() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec!["rust".to_string()],
            private: true,
            owner: "org".to_string(),
            is_org: true,
            language: Some("rust".to_string()),
            default_branch: "develop".to_string(),
            create_develop: true,
            license: Some("MIT".to_string()),
            create_labels: true,
            team_access: vec![teams::TeamAccess {
                team_slug: "t".to_string(),
                permission: "push".to_string(),
            }],
        };
        assert_eq!(config.name, "repo");
        assert_eq!(config.description, "test");
        assert_eq!(config.topics, vec!["rust"]);
        assert!(config.private);
        assert_eq!(config.owner, "org");
        assert!(config.is_org);
        assert_eq!(config.language, Some("rust".to_string()));
        assert_eq!(config.default_branch, "develop");
        assert!(config.create_develop);
        assert_eq!(config.license, Some("MIT".to_string()));
        assert!(config.create_labels);
        assert_eq!(config.team_access.len(), 1);
    }

    #[test]
    fn test_wizard_config_minimal() {
        let config = WizardConfig {
            name: String::new(),
            description: String::new(),
            topics: vec![],
            private: false,
            owner: String::new(),
            is_org: false,
            language: None,
            default_branch: String::new(),
            create_develop: false,
            license: None,
            create_labels: false,
            team_access: vec![],
        };
        assert!(config.name.is_empty());
        assert!(config.topics.is_empty());
        assert!(!config.private);
        assert!(!config.is_org);
        assert!(config.language.is_none());
        assert!(!config.create_develop);
        assert!(config.license.is_none());
        assert!(!config.create_labels);
        assert!(config.team_access.is_empty());
    }

    #[test]
    fn test_count_steps_develop_no_files() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec![],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: None,
            default_branch: "main".to_string(),
            create_develop: true,
            license: None,
            create_labels: false,
            team_access: vec![],
        };
        let steps = count_steps(&config, &[]);
        // create repo + create develop + protect develop + protect main = 4
        assert_eq!(steps, 4);
    }

    #[test]
    fn test_count_steps_labels_and_topics_and_secrets() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec!["rust".to_string(), "cli".to_string()],
            private: false,
            owner: "user".to_string(),
            is_org: false,
            language: None,
            default_branch: "main".to_string(),
            create_develop: false,
            license: None,
            create_labels: true,
            team_access: vec![],
        };
        let secrets = vec![templates::SecretSpec {
            name: "K".to_string(),
            description: "D".to_string(),
            required: true,
        }];
        let steps = count_steps(&config, &secrets);
        // create repo + protect main + labels + topics + secret = 5
        assert_eq!(steps, 5);
    }

    #[test]
    fn test_count_steps_everything_enabled() {
        let config = WizardConfig {
            name: "repo".to_string(),
            description: "test".to_string(),
            topics: vec!["a".to_string()],
            private: true,
            owner: "org".to_string(),
            is_org: true,
            language: Some("rust".to_string()),
            default_branch: "main".to_string(),
            create_develop: true,
            license: Some("MIT".to_string()),
            create_labels: true,
            team_access: (0..3)
                .map(|i| teams::TeamAccess {
                    team_slug: format!("t{i}"),
                    permission: "push".to_string(),
                })
                .collect(),
        };
        let secrets = vec![
            templates::SecretSpec {
                name: "S1".to_string(),
                description: "D1".to_string(),
                required: true,
            },
            templates::SecretSpec {
                name: "S2".to_string(),
                description: "D2".to_string(),
                required: false,
            },
        ];
        let steps = count_steps(&config, &secrets);
        // create repo + init commit + create develop + protect develop + protect main
        // + labels + topics + 3 teams + 2 secrets = 12
        assert_eq!(steps, 12);
    }

    #[test]
    fn test_banner_is_const_str() {
        assert!(!BANNER.is_empty());
        assert!(BANNER.contains("██"));
    }

    #[test]
    fn test_apply_sponsor_answer_yes_calls_enable() {
        let called = std::cell::Cell::new(false);
        apply_sponsor_answer(SponsorAnswer::Yes, || {
            called.set(true);
            Ok(sponsors::SponsorshipStatus::Enabled)
        });
        assert!(called.get(), "answering yes must invoke GS1's function");
    }

    #[test]
    fn test_apply_sponsor_answer_no_skips_enable() {
        apply_sponsor_answer(
            SponsorAnswer::No,
            || -> Result<sponsors::SponsorshipStatus> {
                panic!("answering no must never call GS1's function")
            },
        );
    }

    #[test]
    fn test_apply_sponsor_answer_skipped_skips_enable() {
        apply_sponsor_answer(
            SponsorAnswer::Skipped,
            || -> Result<sponsors::SponsorshipStatus> {
                panic!("a non-interactive run must never call GS1's function")
            },
        );
    }

    #[test]
    fn test_sponsor_outcome_message_enabled() {
        let result = Ok(sponsors::SponsorshipStatus::Enabled);
        assert_eq!(sponsor_outcome_message(&result), "ok");
    }

    #[test]
    fn test_sponsor_outcome_message_already_enabled() {
        let result = Ok(sponsors::SponsorshipStatus::AlreadyEnabled);
        assert_eq!(sponsor_outcome_message(&result), "ok  (already enabled)");
    }

    #[test]
    fn test_sponsor_outcome_message_error() {
        let result: Result<sponsors::SponsorshipStatus> = Err(anyhow::anyhow!("boom"));
        assert_eq!(sponsor_outcome_message(&result), "failed: boom");
    }
}
