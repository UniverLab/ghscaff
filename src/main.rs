use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::atomic::{AtomicBool, Ordering};

mod apply;
mod checks;
mod doctor;
mod github;
mod sponsor_cmd;
mod templates;
mod updater;
mod vault;
mod wizard;

static DEBUG_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_debug(enabled: bool) {
    DEBUG_MODE.store(enabled, Ordering::Relaxed);
}

pub fn is_debug() -> bool {
    DEBUG_MODE.load(Ordering::Relaxed)
}

#[derive(Parser)]
#[command(
    name = "ghscaff",
    version,
    about = "Interactive wizard for creating and configuring GitHub repositories"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Enable the GitHub Sponsor button on an existing repository (owner/repo), then exit
    #[arg(long, value_name = "OWNER/REPO")]
    sponsor: Option<String>,

    /// Preview changes without making any API call
    #[arg(long, global = true)]
    dry_run: bool,

    /// Enable debug logging
    #[arg(long, global = true, hide = true)]
    debug: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new GitHub repository (default when no subcommand given)
    New {
        #[arg(long)]
        dry_run: bool,
    },
    /// Configure an existing repository
    Apply {
        /// owner/repo (auto-detected from git remote if omitted)
        repo: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Reconfigure ghscaff credentials (wipes vault and starts fresh)
    Config,
    /// Check whether a repo's required status checks can ever be satisfied
    Doctor {
        /// owner/repo (auto-detected from git remote if omitted)
        repo: Option<String>,
    },
    /// Update ghscaff to the latest stable release (always asks first)
    ///
    /// Exit codes: 0 = up to date (or update installed / declined /
    /// cargo-managed refusal), 1 = an update is available (--check mode),
    /// 2 = the update check could not be completed (network, HTTP, or an
    /// unparsable response; the cause is printed on stderr).
    Update {
        /// Only report whether an update is available (exit 0 = up to date, 1 = update available, 2 = check failed)
        #[arg(long)]
        check: bool,
        /// Do not prompt; proceed if an update is available
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    set_debug(cli.debug);

    if let Some(target) = &cli.sponsor {
        return sponsor_cmd::run_sponsor(target);
    }

    // The explicit update command is handled before the silent startup
    // notice: `--check` must not pay for a duplicate lookup, and
    // GHSCAFF_NO_UPDATE_CHECK only silences the notice, never this.
    if let Some(Command::Update { check, yes }) = &cli.command {
        if cli.dry_run {
            anyhow::bail!("--dry-run has no effect on update");
        }
        let code = updater::run_update(
            *check,
            *yes,
            &updater::RealFetcher::new(),
            &updater::RealDownloader,
        )?;
        std::process::exit(code);
    }

    check_for_update();
    match cli.command {
        None | Some(Command::New { .. }) => wizard::run(cli.dry_run),
        Some(Command::Apply { repo, dry_run }) => apply::run_apply(repo.as_deref(), dry_run),
        Some(Command::Config) => run_config(),
        Some(Command::Doctor { repo }) => doctor::run_doctor(repo.as_deref()),
        Some(Command::Update { .. }) => unreachable!("ghscaff update is handled above"),
    }
}

fn run_config() -> Result<()> {
    println!();
    println!(
        "  \x1b[33m⚠  This will delete ALL stored credentials and secrets from the vault.\x1b[0m"
    );
    println!("  \x1b[33m   This action cannot be undone.\x1b[0m");
    println!();

    let confirmed = inquire::Confirm::new("Continue with reconfiguration?")
        .with_default(false)
        .prompt()?;

    if !confirmed {
        println!("  Aborted.");
        return Ok(());
    }

    if vault::destroy()? {
        println!("  \x1b[32m✓\x1b[0m Vault deleted");
    } else {
        println!("  ℹ  No vault found");
    }

    println!();
    let (token, _) = vault::prompt_and_save_github_token()?;

    // Validate token
    let client = github::client::GithubClient::new(&token);
    print!("  Validating token... ");
    client.validate_scopes()?;
    let user = github::repo::get_user(&client)?;
    println!("ok  ({})", user.login);

    println!();
    println!(
        "  \x1b[32m✓\x1b[0m ghscaff reconfigured. Template secrets will be requested on next run."
    );
    println!();
    Ok(())
}

/// Silent, read-only startup notice (CM34): when a newer stable release
/// exists, print one line pointing at `ghscaff update` and return. It never
/// prompts and never installs — the explicit command is the only path that
/// downloads anything, and it always asks first. Every failure (DNS, HTTP,
/// rate limit, JSON) is silent here.
fn check_for_update() {
    if std::env::var("GHSCAFF_NO_UPDATE_CHECK").is_ok() {
        return;
    }
    let fetcher = updater::RealFetcher::with_timeout(std::time::Duration::from_secs(3));
    let current = updater::current_version();
    if let Some(tag) = updater::check_notice(&fetcher, &current) {
        println!(
            "  \x1b[33m⬆  Update available:\x1b[0m {} → {} — run 'ghscaff update'",
            updater::display_version(&current),
            updater::display_version(&tag)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parses_sponsor_flag() {
        let cli = Cli::try_parse_from(["ghscaff", "--sponsor", "UniverLab/ghscaff"]).unwrap();
        assert_eq!(cli.sponsor.as_deref(), Some("UniverLab/ghscaff"));
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_cli_sponsor_flag_absent_by_default() {
        let cli = Cli::try_parse_from(["ghscaff"]).unwrap();
        assert!(cli.sponsor.is_none());
    }

    #[test]
    fn test_cli_sponsor_flag_requires_value() {
        assert!(Cli::try_parse_from(["ghscaff", "--sponsor"]).is_err());
    }

    #[test]
    fn test_cli_parses_update() {
        let cli = Cli::try_parse_from(["ghscaff", "update"]).unwrap();
        match cli.command {
            Some(Command::Update { check, yes }) => {
                assert!(!check, "--check defaults to false");
                assert!(!yes, "--yes defaults to false");
            }
            _ => panic!("expected Command::Update"),
        }
    }

    #[test]
    fn test_cli_parses_update_check_yes() {
        let cli = Cli::try_parse_from(["ghscaff", "update", "--check", "--yes"]).unwrap();
        match cli.command {
            Some(Command::Update { check, yes }) => {
                assert!(check);
                assert!(yes);
            }
            _ => panic!("expected Command::Update"),
        }
    }

    #[test]
    fn test_debug_mode_default() {
        assert!(!is_debug());
    }

    #[test]
    fn test_set_debug_enable() {
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
    }

    #[test]
    fn test_set_debug_disable() {
        set_debug(true);
        set_debug(false);
        assert!(!is_debug());
    }

    #[test]
    fn test_debug_mode_toggle() {
        set_debug(false);
        assert!(!is_debug());
        set_debug(true);
        assert!(is_debug());
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }

    #[test]
    fn test_set_debug_idempotent() {
        set_debug(true);
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }

    #[test]
    fn test_debug_mode_default_is_false() {
        set_debug(false);
        assert!(!is_debug());
    }

    #[test]
    fn test_debug_mode_toggle_sequence() {
        set_debug(false);
        assert!(!is_debug());
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }
}
