use anyhow::Result;
use inquire::{Confirm, MultiSelect, Select, Text};

use crate::github::{
    branches,
    client::{token_from_env, GithubClient},
    contents, labels, repo,
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
    pub branch_protection: bool,
    pub license: Option<String>,
    pub create_readme: bool,
    pub create_ci: bool,
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
    print_summary(&config);

    println!();
    let confirmed = Confirm::new("Apply these changes?")
        .with_default(true)
        .prompt()?;

    if !confirmed {
        println!("  Aborted.");
        return Ok(());
    }

    execute(&client, &config, dry_run)
}

fn collect_config(client: &GithubClient, username: &str) -> Result<WizardConfig> {
    // Step 1 — Repository basics
    let name = Text::new("Repository name").prompt()?;
    let description = Text::new("Description").with_default("").prompt()?;
    let topics_raw = Text::new("Topics (comma-separated)")
        .with_default("")
        .with_help_message("e.g. rust,cli,tool")
        .prompt()?;
    let topics: Vec<String> = topics_raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    // Step 2 — Visibility & ownership
    let visibility = Select::new("Visibility", vec!["Private", "Public"]).prompt()?;
    let private = visibility == "Private";

    let mut owner_options = vec![format!("{username} (personal)")];
    let orgs = repo::list_orgs(client).unwrap_or_default();
    for org in &orgs {
        owner_options.push(org.login.clone());
    }
    let owner_selection = Select::new("Owner", owner_options).prompt()?;
    let (owner, is_org) = if owner_selection.ends_with("(personal)") {
        (username.to_string(), false)
    } else {
        (owner_selection.clone(), true)
    };

    // Step 3 — Language
    let language = Select::new("Language / template", templates::AVAILABLE.to_vec())
        .with_help_message("Drives .gitignore, CI workflow, and boilerplate")
        .prompt()?
        .to_string();

    // Step 4 — Branches
    let default_branch = Text::new("Default branch").with_default("main").prompt()?;
    let create_develop = Confirm::new("Create develop branch?")
        .with_default(true)
        .prompt()?;
    let branch_protection = Confirm::new("Enable branch protection on main?")
        .with_default(true)
        .prompt()?;

    // Step 5 — Features
    let feature_items = vec![
        "LICENSE",
        "README.md",
        "GitHub Actions CI workflow",
        "Standard label set (12 labels)",
    ];
    let feature_defaults = vec![0usize, 1, 2, 3];
    let features = MultiSelect::new("Features", feature_items.clone())
        .with_default(&feature_defaults)
        .with_help_message("space select  enter confirm")
        .prompt()?;

    let license = if features.contains(&"LICENSE") {
        let lic = Select::new("License", vec!["MIT", "Apache-2.0", "GPL-3.0", "None"]).prompt()?;
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
        branch_protection,
        license,
        create_readme: features.contains(&"README.md"),
        create_ci: features.contains(&"GitHub Actions CI workflow"),
        create_labels: features.contains(&"Standard label set (12 labels)"),
    })
}

fn print_summary(c: &WizardConfig) {
    println!("\n  Summary:");
    println!("  ◆ {}/{}", c.owner, c.name);
    if !c.description.is_empty() {
        println!("  ◆ description: {}", c.description);
    }
    println!(
        "  ◆ visibility: {}",
        if c.private { "private" } else { "public" }
    );
    println!("  ◆ language: {}", c.language);
    println!("  ◆ default branch: {}", c.default_branch);
    if c.create_develop {
        println!("  ◆ develop branch: yes");
    }
    if c.branch_protection {
        println!("  ◆ branch protection: yes");
    }
    if let Some(lic) = &c.license {
        println!("  ◆ license: {lic}");
    }
    let mut features = vec![];
    if c.create_readme {
        features.push("README");
    }
    if c.create_ci {
        features.push("CI workflow");
    }
    if c.create_labels {
        features.push("labels");
    }
    if !features.is_empty() {
        println!("  ◆ features: {}", features.join(", "));
    }
}

fn execute(client: &GithubClient, c: &WizardConfig, dry_run: bool) -> Result<()> {
    println!();

    let tmpl = templates::resolve(&c.language)?;
    let total = count_steps(c);
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

    // 1. Create repo
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

    // 2. Commit initial files
    let files = tmpl.boilerplate_files(name, &c.description);
    let mut last_sha = String::new();

    for file in &files {
        step!(&format!("commit {}", file.path), {
            let sha = contents::create_file(
                client,
                owner,
                name,
                &file.path,
                &file.content,
                &file.commit_message,
            )?;
            last_sha = sha;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 3. .gitignore
    step!("commit .gitignore", {
        let content = repo::get_gitignore_template(client, tmpl.gitignore_name())?;
        contents::create_file(
            client,
            owner,
            name,
            ".gitignore",
            &content,
            "chore: add .gitignore",
        )?;
        Ok::<(), anyhow::Error>(())
    });

    // 4. LICENSE
    if c.license.is_some() {
        step!("commit LICENSE", {
            contents::create_file(
                client,
                owner,
                name,
                "LICENSE",
                "# License placeholder",
                "chore: add LICENSE",
            )?;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 5. CI workflow
    if c.create_ci {
        step!("commit CI workflow", {
            contents::create_file(
                client,
                owner,
                name,
                ".github/workflows/ci.yml",
                tmpl.ci_workflow(),
                "ci: add GitHub Actions workflow",
            )?;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 6. develop branch
    if c.create_develop {
        step!("create develop branch", {
            let sha = if last_sha.is_empty() {
                branches::get_branch_sha(client, owner, name, &c.default_branch)?
            } else {
                last_sha.clone()
            };
            branches::create_branch(client, owner, name, "develop", &sha)?;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 7. Branch protection
    if c.branch_protection {
        step!("apply branch protection", {
            branches::apply_branch_protection(client, owner, name, &c.default_branch, "build")?;
            Ok::<(), anyhow::Error>(())
        });
    }

    // 8. Labels
    if c.create_labels {
        step!("sync labels", {
            let existing = labels::list_labels(client, owner, name)?;
            let standard = labels::standard_labels();
            let mut created = 0u32;
            for label in &standard {
                if existing.iter().any(|e| e.name == label.name) {
                    labels::update_label(client, owner, name, &label.name, label)?;
                } else {
                    labels::create_label(client, owner, name, label)?;
                    created += 1;
                }
            }
            println!("ok  ({created} created)");
            Ok::<(), anyhow::Error>(())
        });
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

fn count_steps(c: &WizardConfig) -> usize {
    let tmpl = templates::resolve(&c.language).unwrap();
    let mut n = 1; // create repo
    n += tmpl.boilerplate_files(&c.name, &c.description).len(); // files
    n += 1; // .gitignore
    if c.license.is_some() {
        n += 1;
    }
    if c.create_ci {
        n += 1;
    }
    if c.create_develop {
        n += 1;
    }
    if c.branch_protection {
        n += 1;
    }
    if c.create_labels {
        n += 1;
    }
    n
}
