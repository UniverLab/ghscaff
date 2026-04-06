use anyhow::Result;
use inquire::{Confirm, MultiSelect, Password, Select, Text};

use crate::github::{
    branches,
    client::{token_from_env, GithubClient},
    contents, labels, repo, secrets,
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
}

pub fn run(dry_run: bool) -> Result<()> {
    println!("{BANNER}");
    println!("  Create a new GitHub repository\n");

    // Fail fast — validate token before asking anything
    let token = token_from_env()?;
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

    execute(&client, &config, dry_run, &token)
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
    })
}

fn execute(client: &GithubClient, c: &WizardConfig, dry_run: bool, token: &str) -> Result<()> {
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
        init_files.push(contents::TreeFile { path: f.path, content: f.content });
    }

    // .gitignore — fetched fresh from GitHub's gitignore API
    let gitignore = repo::get_gitignore_template(client, &tmpl.gitignore_name())?;
    init_files.push(contents::TreeFile { path: ".gitignore".into(), content: gitignore });

    // LICENSE (placeholder — user replaces it or CI generates it)
    if let Some(lic) = &c.license {
        let license_text = format!(
            "# {} License\n\nSee https://opensource.org/licenses/{} for the full license text.\n",
            lic, lic
        );
        init_files.push(contents::TreeFile { path: "LICENSE".into(), content: license_text });
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
    step!(&format!("apply branch protection ({})", c.default_branch), {
        branches::apply_branch_protection(client, owner, name, &c.default_branch, "build")?;
        Ok::<(), anyhow::Error>(())
    });
    if c.create_develop {
        step!("apply branch protection (develop)", {
            branches::apply_branch_protection(client, owner, name, "develop", "build")?;
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

    // 8. Secrets from template — read from env first, prompt if missing, warn if skipped
    for spec in &secret_specs {
        let value = if let Ok(env_val) = std::env::var(&spec.name) {
            println!("  ◆ Secret {}: using value from environment", spec.name);
            Some(env_val)
        } else {
            let ans = Password::new(&format!("Secret {} (enter to skip):", spec.name))
                .with_help_message(&spec.description)
                .without_confirmation()
                .prompt_skippable()?;
            if ans.as_deref().map(str::is_empty).unwrap_or(true) {
                println!(
                    "  ⚠ Secret {} not configured — set ${} and re-run `ghscaff apply`",
                    spec.name, spec.name
                );
                None
            } else {
                ans
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
    if let Some(r) = created_repo {
        println!("  Done  —  {}", r.html_url);
    } else {
        println!("  Done  (dry-run)");
    }
    println!();
    Ok(())
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
    n += secrets.len();
    n
}
