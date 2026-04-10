use anyhow::Result;
use inquire::{Confirm, MultiSelect, Password, Select, Text};

use crate::github::{
    branches,
    client::{resolve_token, GithubClient},
    contents, labels, repo, secrets, teams,
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
    pub language: String,
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
    let name = Text::new("Repository name:").prompt()?;
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

    // Step 3 — Language
    let language = Select::new("Language:", templates::AVAILABLE.to_vec())
        .with_help_message("Drives .gitignore, CI workflow, and boilerplate")
        .prompt()?
        .to_string();

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

    // Always download fresh template for `new` so cache is never stale
    print!("  Fetching boilerplate template... ");
    let tmpl = templates::resolve(&c.language, token, true)?;
    println!("ok");
    let secret_specs = templates::load_secrets(&c.language);
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

    // Template files (boilerplate — all files including ci.yml, release.yml, etc.)
    for f in tmpl.boilerplate_files(name, &c.description, owner) {
        init_files.push(contents::TreeFile {
            path: f.path,
            content: f.content,
        });
    }

    // .gitignore — fetched fresh from GitHub's gitignore API
    let gitignore = repo::get_gitignore_template(client, &tmpl.gitignore_name())?;
    init_files.push(contents::TreeFile {
        path: ".gitignore".into(),
        content: gitignore,
    });

    // LICENSE (placeholder — user replaces it or CI generates it)
    if let Some(lic) = &c.license {
        let license_text = format!(
            "# {} License\n\nSee https://opensource.org/licenses/{} for the full license text.\n",
            lic, lic
        );
        init_files.push(contents::TreeFile {
            path: "LICENSE".into(),
            content: license_text,
        });
    }

    // 3. Single init commit with all files
    let mut init_sha = String::new();
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

    // 4. develop branch
    if c.create_develop {
        step!("create develop branch", {
            branches::create_branch(client, owner, name, "develop", &init_sha)?;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 5. Branch protection — always applied to main (and develop if created)
    step!(
        &format!("apply branch protection ({})", c.default_branch),
        {
            branches::apply_branch_protection(
                client,
                owner,
                name,
                &c.default_branch,
                "rust-ci / Format, Lint & Test",
            )?;
            Ok::<(), anyhow::Error>(())
        }
    );
    if c.create_develop {
        step!("apply branch protection (develop)", {
            branches::apply_branch_protection(
                client,
                owner,
                name,
                "develop",
                "rust-ci / Format, Lint & Test",
            )?;
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
            let ans = Password::new(&format!("Secret {} (enter to skip):", spec.name))
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
                    Some(v.to_string())
                }
                _ => {
                    println!(
                        "  ⚠ Secret {} not configured — re-run `ghscaff apply` to set it later",
                        spec.name
                    );
                    None
                }
            }
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
        if let Some(r) = &created_repo {
            offer_gitkit_clone(&r.html_url);
        }
    }

    Ok(())
}

fn offer_gitkit_clone(repo_url: &str) {
    let Ok(want_clone) = Confirm::new("Clone the repository with gitkit?")
        .with_default(true)
        .prompt()
    else {
        return;
    };
    if !want_clone {
        return;
    }

    if !is_command_available("gitkit") {
        println!("  gitkit not found. Installing...\n");
        install_gitkit();
        if !is_command_available("gitkit") {
            println!("  ⚠ gitkit installation failed — clone manually with:");
            println!("    gitkit clone {repo_url}");
            return;
        }
    }

    let _ = std::process::Command::new("gitkit")
        .args(["clone", repo_url])
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

fn count_steps(c: &WizardConfig, secrets: &[templates::SecretSpec]) -> usize {
    let mut n = 1; // create repo
    n += 1; // init commit (all boilerplate files in one shot)
    if c.create_develop {
        n += 1; // create develop
        n += 1; // protect develop
    }
    n += 1; // protect main (always)
    if c.create_labels {
        n += 1;
    }
    if !c.topics.is_empty() {
        n += 1;
    }
    n += c.team_access.len(); // one step per team
    n += secrets.len();
    n
}

#[cfg(test)]
mod tests {
    use super::*;

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
            language: "rust".to_string(),
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
            language: "rust".to_string(),
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
            language: "rust".to_string(),
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
            language: "rust".to_string(),
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
            language: "rust".to_string(),
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
}
