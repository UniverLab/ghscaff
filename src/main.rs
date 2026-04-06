use anyhow::Result;
use clap::{Parser, Subcommand};

mod apply;
mod github;
mod templates;
mod wizard;

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
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    check_for_update();
    match cli.command {
        None | Some(Command::New { .. }) => wizard::run(cli.dry_run),
        Some(Command::Apply { repo, dry_run }) => apply::run_apply(repo.as_deref(), dry_run),
    }
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
        (*p.first().unwrap_or(&0), *p.get(1).unwrap_or(&0), *p.get(2).unwrap_or(&0))
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
