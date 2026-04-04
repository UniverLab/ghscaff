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
    match cli.command {
        None | Some(Command::New { .. }) => wizard::run(cli.dry_run),
        Some(Command::Apply { repo, dry_run }) => apply::run_apply(repo.as_deref(), dry_run),
    }
}
