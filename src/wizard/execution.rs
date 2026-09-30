use anyhow::Result;

use crate::github::{branches, client::GithubClient, contents, labels, repo, secrets, teams};
use crate::templates;

use super::{
    count_steps, current_year, offer_gitkit_clone, offer_sponsor_button, prompt_secret_value,
    WizardConfig,
};

/// Counts and prints the wizard's "[n/total]" progress lines. In dry-run
/// mode the same lines are printed with a `[dry-run]` marker and the step
/// body is never run.
struct StepCounter {
    current: usize,
    total: usize,
    dry_run: bool,
}

impl StepCounter {
    fn new(total: usize, dry_run: bool) -> Self {
        Self {
            current: 0,
            total,
            dry_run,
        }
    }

    /// Report the start of the next step and return its `(n, total)` numbers.
    fn advance(&mut self) -> (usize, usize) {
        self.current += 1;
        (self.current, self.total)
    }

    /// Print one progress line and, unless this is a dry run, run `op`.
    fn run(&mut self, msg: &str, op: impl FnOnce() -> Result<()>) -> Result<()> {
        let (n, total) = self.advance();
        if self.dry_run {
            println!("  [{n}/{total}] [dry-run] {msg}");
            return Ok(());
        }
        print!("  [{n}/{total}] {msg}... ");
        op()?;
        println!("ok");
        Ok(())
    }

    /// Count a step that printed nothing of its own (a skipped secret).
    fn skip(&mut self) {
        self.advance();
    }
}

/// The boilerplate template the wizard fetched (when a language was chosen)
/// plus the secrets that template declares.
struct WizardInitPlan {
    template: Option<Box<dyn templates::LanguageTemplate>>,
    secret_specs: Vec<templates::SecretSpec>,
}

/// Fetch the selected template (if any) and the secrets it needs.
fn fetch_wizard_template(c: &WizardConfig, token: &str) -> Result<WizardInitPlan> {
    let template = if let Some(lang) = &c.language {
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
    Ok(WizardInitPlan {
        template,
        secret_specs,
    })
}

/// Collect every file for the single init commit: template boilerplate plus
/// .gitignore, and the LICENSE with its copyright placeholders filled in.
fn build_wizard_init_files(
    client: &GithubClient,
    c: &WizardConfig,
    template: Option<&dyn templates::LanguageTemplate>,
) -> Result<Vec<contents::TreeFile>> {
    let mut init_files: Vec<contents::TreeFile> = vec![];

    if let Some(tmpl) = template {
        for f in tmpl.boilerplate_files(&c.name, &c.description, &c.owner) {
            init_files.push(contents::TreeFile {
                path: f.path,
                content: f.content,
            });
        }

        let gitignore = repo::get_gitignore_template(client, &tmpl.gitignore_name())
            .unwrap_or_else(|_| {
                eprintln!("  ⚠  Could not fetch .gitignore template from GitHub");
                String::new()
            });
        init_files.push(contents::TreeFile {
            path: ".gitignore".into(),
            content: templates::assemble_gitignore(&gitignore),
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
            .replace("[fullname]", &c.owner)
            .replace("[name of copyright owner]", &c.owner);
        init_files.push(contents::TreeFile {
            path: "LICENSE".into(),
            content: license_text,
        });
    }

    Ok(init_files)
}

/// Step 1 — create the repo (empty; the initial commit follows via the Trees API).
fn create_wizard_repo_step(
    progress: &mut StepCounter,
    client: &GithubClient,
    c: &WizardConfig,
) -> Result<Option<repo::Repo>> {
    let (n, total) = progress.advance();
    if progress.dry_run {
        println!(
            "  [{n}/{total}] [dry-run] create repo {}/{}",
            c.owner, c.name
        );
        return Ok(None);
    }
    print!("  [{n}/{total}] create repo {}/{}... ", c.owner, c.name);
    let r = repo::create_repo(
        client,
        &c.owner,
        &c.name,
        &c.description,
        c.private,
        c.is_org,
    )?;
    println!("ok  ({})", r.html_url);
    Ok(Some(r))
}

/// Step 3 — one init commit with all the files (skipped for an empty repo
/// with no LICENSE); records the commit SHA the develop branch step needs.
fn commit_init_files_step(
    progress: &mut StepCounter,
    client: &GithubClient,
    c: &WizardConfig,
    init_files: &[contents::TreeFile],
    init_sha: &mut String,
) -> Result<()> {
    if init_files.is_empty() {
        return Ok(());
    }
    progress.run("init repository", || {
        *init_sha = contents::create_tree_commit(
            client,
            &c.owner,
            &c.name,
            init_files,
            "chore: init repository",
            &c.default_branch,
        )?;
        Ok(())
    })
}

/// Step 4 — develop branch off the just-committed default branch.
fn create_develop_step(
    progress: &mut StepCounter,
    client: &GithubClient,
    c: &WizardConfig,
    init_sha: &mut String,
) -> Result<()> {
    if !c.create_develop {
        return Ok(());
    }
    if init_sha.is_empty() {
        *init_sha = branches::get_branch_sha(client, &c.owner, &c.name, &c.default_branch)?;
    }
    let sha = init_sha.as_str();
    progress.run("create develop branch", || {
        branches::create_branch(client, &c.owner, &c.name, "develop", sha)?;
        Ok(())
    })
}

/// Step 5 — branch protection. Required contexts are derived from the
/// workflow files just committed, never hardcoded, so a renamed job can't
/// leave a required check pointing at a name nothing will ever report.
fn apply_wizard_protections_step(
    progress: &mut StepCounter,
    client: &GithubClient,
    c: &WizardConfig,
    init_files: &[contents::TreeFile],
) -> Result<()> {
    let workflow_sources: Vec<crate::checks::WorkflowSource> = init_files
        .iter()
        .map(|f| crate::checks::WorkflowSource {
            path: &f.path,
            content: &f.content,
        })
        .collect();
    let required_contexts = crate::checks::derive_required_contexts(&workflow_sources);

    progress.run(
        &format!("apply branch protection ({})", c.default_branch),
        || {
            branches::apply_branch_protection(
                client,
                &c.owner,
                &c.name,
                &c.default_branch,
                &required_contexts,
            )?;
            Ok(())
        },
    )?;
    if c.create_develop {
        progress.run("apply branch protection (develop)", || {
            branches::apply_branch_protection(
                client,
                &c.owner,
                &c.name,
                "develop",
                &required_contexts,
            )?;
            Ok(())
        })?;
    }
    Ok(())
}

/// Step 6 — labels.
fn sync_wizard_labels_step(
    progress: &mut StepCounter,
    client: &GithubClient,
    c: &WizardConfig,
) -> Result<()> {
    if !c.create_labels {
        return Ok(());
    }
    progress.run("sync labels", || {
        let existing = labels::list_labels(client, &c.owner, &c.name)?;
        let standard = labels::standard_labels();
        for label in &standard {
            if existing.iter().any(|e| e.name == label.name) {
                labels::update_label(client, &c.owner, &c.name, &label.name, label)?;
            } else {
                labels::create_label(client, &c.owner, &c.name, label)?;
            }
        }
        for existing_label in &existing {
            if !standard.iter().any(|s| s.name == existing_label.name) {
                let _ = labels::delete_label(client, &c.owner, &c.name, &existing_label.name);
            }
        }
        Ok(())
    })
}

/// Step 7 — topics.
fn set_wizard_topics_step(
    progress: &mut StepCounter,
    client: &GithubClient,
    c: &WizardConfig,
) -> Result<()> {
    if c.topics.is_empty() {
        return Ok(());
    }
    progress.run("set topics", || {
        repo::set_topics(client, &c.owner, &c.name, &c.topics)?;
        Ok(())
    })
}

/// Step 8 — team access for each team picked during configuration.
fn grant_wizard_team_access_step(
    progress: &mut StepCounter,
    client: &GithubClient,
    c: &WizardConfig,
) -> Result<()> {
    for team in &c.team_access {
        progress.run(
            &format!(
                "add team {} with {} access",
                team.team_slug, team.permission
            ),
            || {
                teams::add_team_to_repo(
                    client,
                    &c.owner,
                    &c.name,
                    &team.team_slug,
                    &team.permission,
                )?;
                Ok(())
            },
        )?;
    }
    Ok(())
}

/// Step 9 — template secrets: env → vault → prompt.
fn configure_wizard_secrets_step(
    progress: &mut StepCounter,
    client: &GithubClient,
    c: &WizardConfig,
    passphrase: &str,
    secret_specs: &[templates::SecretSpec],
) -> Result<()> {
    for spec in secret_specs {
        let value = if let Some(val) = crate::vault::resolve_secret(&spec.name, passphrase)? {
            println!("  ◆ Secret {}: found", spec.name);
            Some(val)
        } else {
            prompt_secret_value(spec, passphrase)?
        };
        if let Some(val) = value {
            progress.run(&format!("configure secret {}", spec.name), || {
                secrets::set_secret(client, &c.owner, &c.name, &spec.name, &val)?;
                Ok(())
            })?;
        } else {
            progress.skip(); // keep total consistent even when skipped
        }
    }
    Ok(())
}

pub(super) fn execute(
    client: &GithubClient,
    c: &WizardConfig,
    dry_run: bool,
    token: &str,
    passphrase: &str,
) -> Result<()> {
    println!();

    // Fetch template if selected
    let plan = fetch_wizard_template(c, token)?;
    let total = count_steps(c, &plan.secret_specs);
    let mut progress = StepCounter::new(total, dry_run);

    // 1. Create repo (empty — initial commit via Trees API below)
    let created_repo = create_wizard_repo_step(&mut progress, client, c)?;

    // 2. Collect all boilerplate files for a single init commit
    let init_files = build_wizard_init_files(client, c, plan.template.as_deref())?;

    // 3. Single init commit with all files (skip if empty repo with no LICENSE)
    let mut init_sha = String::new();
    commit_init_files_step(&mut progress, client, c, &init_files, &mut init_sha)?;

    // 4. develop branch
    create_develop_step(&mut progress, client, c, &mut init_sha)?;

    // 5. Branch protection (default branch, then develop when requested)
    apply_wizard_protections_step(&mut progress, client, c, &init_files)?;

    // 6. Labels
    sync_wizard_labels_step(&mut progress, client, c)?;

    // 7. Topics
    set_wizard_topics_step(&mut progress, client, c)?;

    // 8. Team access
    grant_wizard_team_access_step(&mut progress, client, c)?;

    // 9. Template secrets: env → vault → prompt
    configure_wizard_secrets_step(&mut progress, client, c, passphrase, &plan.secret_specs)?;

    println!();
    if let Some(r) = &created_repo {
        println!("  Done  —  {}", r.html_url);
    } else {
        println!("  Done  (dry-run)");
    }
    println!();

    if !dry_run && created_repo.is_some() {
        offer_gitkit_clone(&c.owner, &c.name);
        offer_sponsor_button(client, &c.owner, &c.name);
    }

    Ok(())
}
