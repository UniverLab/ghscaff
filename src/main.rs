use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::atomic::{AtomicBool, Ordering};

mod apply;
mod github;
mod templates;
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    set_debug(cli.debug);
    check_for_update();
    match cli.command {
        None | Some(Command::New { .. }) => wizard::run(cli.dry_run),
        Some(Command::Apply { repo, dry_run }) => apply::run_apply(repo.as_deref(), dry_run),
        Some(Command::Config) => run_config(),
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

fn check_for_update() {
    if std::env::var("GHSCAFF_NO_UPDATE_CHECK").is_ok() {
        return;
    }
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .user_agent("ghscaff")
        .build()
    else {
        return;
    };
    let Ok(resp) = client
        .get("https://api.github.com/repos/UniverLab/ghscaff/releases/latest")
        .send()
    else {
        return;
    };
    let Ok(json) = resp.json::<serde_json::Value>() else {
        return;
    };
    let Some(latest_tag) = json["tag_name"].as_str() else {
        return;
    };
    let current = format!("v{}", env!("CARGO_PKG_VERSION"));
    if !is_newer(&current, latest_tag) {
        return;
    }
    println!("  \x1b[33m⬆  Update available:\x1b[0m {current} → {latest_tag}");
    let Ok(install) = inquire::Confirm::new("Install now?")
        .with_default(true)
        .prompt()
    else {
        return;
    };
    if !install {
        println!();
        return;
    }
    run_installer();
}

fn is_newer(current: &str, latest: &str) -> bool {
    let parse = |v: &str| -> (u64, u64, u64) {
        let v = v.trim_start_matches('v');
        let p: Vec<u64> = v.split('.').filter_map(|s| s.parse().ok()).collect();
        (
            *p.first().unwrap_or(&0),
            *p.get(1).unwrap_or(&0),
            *p.get(2).unwrap_or(&0),
        )
    };
    parse(latest) > parse(current)
}

fn run_installer() {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("powershell")
            .args([
                "-Command",
                "irm https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.ps1 | iex",
            ])
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        match std::process::Command::new("sh")
            .args([
                "-c",
                "curl -fsSL https://raw.githubusercontent.com/UniverLab/ghscaff/main/scripts/install.sh | sh",
            ])
            .status()
        {
            Ok(_) => {
                println!("  \x1b[32m✓\x1b[0m Updated! Restart your terminal to use the new version.");
                std::process::exit(0);
            }
            Err(e) => eprintln!("  ⚠ Installer failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_major_version() {
        assert!(is_newer("v0.5.0", "v1.0.0"));
    }

    #[test]
    fn test_is_newer_minor_version() {
        assert!(is_newer("v0.5.0", "v0.6.0"));
    }

    #[test]
    fn test_is_newer_patch_version() {
        assert!(is_newer("v0.5.0", "v0.5.1"));
    }

    #[test]
    fn test_is_newer_same_version() {
        assert!(!is_newer("v0.5.0", "v0.5.0"));
    }

    #[test]
    fn test_is_newer_older() {
        assert!(!is_newer("v1.0.0", "v0.9.0"));
    }

    #[test]
    fn test_is_newer_without_v_prefix() {
        assert!(is_newer("0.5.0", "0.6.0"));
    }

    #[test]
    fn test_is_newer_mixed_prefix() {
        assert!(is_newer("v0.5.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_major_only() {
        assert!(is_newer("v1.0.0", "v2.0.0"));
    }

    #[test]
    fn test_is_newer_minor_only() {
        assert!(!is_newer("v2.0.0", "v1.9.0"));
    }

    #[test]
    fn test_is_newer_patch_bumps_not_enough() {
        assert!(!is_newer("v1.0.1", "v1.0.0"));
    }

    #[test]
    fn test_is_newer_zero_versions() {
        assert!(!is_newer("v0.0.0", "v0.0.0"));
    }

    #[test]
    fn test_is_newer_large_numbers() {
        assert!(is_newer("v0.0.1", "v99.99.99"));
    }

    #[test]
    fn test_is_newer_incomplete_version_latest() {
        assert!(is_newer("v0.5.0", "v1"));
    }

    #[test]
    fn test_is_newer_incomplete_version_current() {
        assert!(!is_newer("v1", "v1.0.0"));
    }

    #[test]
    fn test_is_newer_both_incomplete() {
        assert!(!is_newer("v1", "v1"));
    }

    #[test]
    fn test_is_newer_invalid_input() {
        assert!(!is_newer("abc", "xyz"));
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
    fn test_is_newer_equal_major_minor() {
        assert!(!is_newer("v1.2.3", "v1.2.3"));
    }

    #[test]
    fn test_is_newer_patch_only_difference() {
        assert!(is_newer("v1.2.0", "v1.2.1"));
        assert!(!is_newer("v1.2.1", "v1.2.0"));
    }

    #[test]
    fn test_is_newer_major_difference_overrides_minor() {
        assert!(is_newer("v1.9.9", "v2.0.0"));
        assert!(!is_newer("v2.0.0", "v1.9.9"));
    }

    #[test]
    fn test_is_newer_minor_difference_overrides_patch() {
        assert!(is_newer("v1.1.9", "v1.2.0"));
        assert!(!is_newer("v1.2.0", "v1.1.9"));
    }

    #[test]
    fn test_is_newer_leading_v_stripped() {
        assert!(is_newer("v1.0.0", "v2.0.0"));
        assert!(!is_newer("v2.0.0", "v1.0.0"));
    }

    #[test]
    fn test_is_newer_no_v_prefix() {
        assert!(is_newer("1.0.0", "2.0.0"));
        assert!(!is_newer("2.0.0", "1.0.0"));
    }

    #[test]
    fn test_is_newer_mixed_v_prefix() {
        assert!(is_newer("v1.0.0", "2.0.0"));
        assert!(is_newer("1.0.0", "v2.0.0"));
    }

    #[test]
    fn test_is_newer_empty_strings() {
        assert!(!is_newer("", ""));
    }

    #[test]
    fn test_is_newer_single_number() {
        assert!(is_newer("v1", "v2"));
        assert!(!is_newer("v2", "v1"));
    }

    #[test]
    fn test_is_newer_non_numeric_parts() {
        assert!(!is_newer("vabc", "vdef"));
    }

    #[test]
    fn test_is_newer_very_large_versions() {
        assert!(is_newer("v999.999.999", "v1000.0.0"));
    }

    #[test]
    fn test_is_newer_zero_vs_nonzero() {
        assert!(is_newer("v0.0.0", "v0.0.1"));
        assert!(!is_newer("v0.0.1", "v0.0.0"));
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
    fn test_is_newer_major_minor_patch_all_different() {
        assert!(is_newer("v1.2.3", "v2.3.4"));
        assert!(!is_newer("v2.3.4", "v1.2.3"));
    }

    #[test]
    fn test_is_newer_major_minor_same_patch_different() {
        assert!(is_newer("v1.1.0", "v1.1.1"));
        assert!(!is_newer("v1.1.1", "v1.1.0"));
    }

    #[test]
    fn test_is_newer_major_same_minor_different() {
        assert!(is_newer("v1.1.0", "v1.2.0"));
        assert!(!is_newer("v1.2.0", "v1.1.0"));
    }

    #[test]
    fn test_is_newer_with_whitespace() {
        assert!(!is_newer("v1.0.0 ", "v1.0.0"));
        assert!(!is_newer("v1.0.0", " v1.0.0"));
    }

    #[test]
    fn test_is_newer_four_part_version() {
        assert!(is_newer("v1.0.0.0", "v1.0.1.0"));
    }

    #[test]
    fn test_is_newer_latest_is_shorter() {
        assert!(!is_newer("v1.0.0", "v1.0"));
    }

    #[test]
    fn test_is_newer_current_is_shorter() {
        assert!(!is_newer("v1.0", "v1.0.0"));
    }

    #[test]
    fn test_is_newer_both_short() {
        assert!(!is_newer("v1", "v1"));
    }

    #[test]
    fn test_is_newer_non_version_string() {
        assert!(!is_newer("latest", "stable"));
    }

    #[test]
    fn test_set_debug_idempotent() {
        set_debug(true);
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        set_debug(false);
        assert!(!is_debug());
    }

    #[test]
    fn test_is_newer_boundary_values() {
        assert!(is_newer("v0.0.0", "v1.0.0"));
        assert!(!is_newer("v1.0.0", "v0.0.0"));
        assert!(is_newer("v0.0.0", "v0.1.0"));
        assert!(is_newer("v0.0.0", "v0.0.1"));
    }

    #[test]
    fn test_is_newer_with_extra_dots() {
        assert!(!is_newer("v1..0", "v1..0"));
    }

    #[test]
    fn test_is_newer_with_leading_zeros() {
        assert!(!is_newer("v01.00.00", "v1.0.0"));
    }

    #[test]
    fn test_is_newer_major_10_vs_9() {
        assert!(is_newer("v9.0.0", "v10.0.0"));
    }

    #[test]
    fn test_is_newer_minor_99_vs_100() {
        assert!(is_newer("v1.99.0", "v1.100.0"));
    }

    #[test]
    fn test_is_newer_patch_999_vs_1000() {
        assert!(is_newer("v1.0.999", "v1.0.1000"));
    }

    #[test]
    fn test_is_newer_all_zero_vs_one() {
        assert!(is_newer("v0.0.0", "v0.0.1"));
        assert!(is_newer("v0.0.0", "v0.1.0"));
        assert!(is_newer("v0.0.0", "v1.0.0"));
    }

    #[test]
    fn test_is_newer_identical_complex() {
        assert!(!is_newer("v1.2.3", "v1.2.3"));
    }

    #[test]
    fn test_is_newer_current_greater_patch() {
        assert!(!is_newer("v1.0.5", "v1.0.3"));
    }

    #[test]
    fn test_is_newer_current_greater_minor() {
        assert!(!is_newer("v1.5.0", "v1.3.0"));
    }

    #[test]
    fn test_is_newer_current_greater_major() {
        assert!(!is_newer("v5.0.0", "v3.0.0"));
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
        set_debug(true);
        assert!(is_debug());
        set_debug(false);
        assert!(!is_debug());
    }
}
