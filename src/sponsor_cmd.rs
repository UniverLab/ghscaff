//! Standalone `--sponsor <owner/repo>` command: enable the GitHub Sponsor
//! button on a single existing repository without walking the creation
//! wizard. Built on GS1's [`crate::github::sponsors::enable_sponsorships`].

use anyhow::{bail, Result};

use crate::github::{client, sponsors};

/// Parse `owner/repo` into its two non-empty parts.
///
/// Follows the same `owner/repo` split-on-`/` convention as
/// `apply::parse_owner_repo`, but additionally rejects empty owner/repo
/// segments so a malformed argument is caught before any network call.
fn parse_owner_repo(input: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = input.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        bail!("Invalid repository '{input}'. Expected format: owner/repo");
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Enable the GitHub Sponsor button on `target` (`owner/repo`) and print a
/// single result line. Does not run the wizard and touches nothing else
/// about the repository.
pub fn run_sponsor(target: &str) -> Result<()> {
    let (owner, name) = parse_owner_repo(target)?;

    let (token, _) = client::resolve_token()?;
    let gh = client::GithubClient::new(&token);

    let status = sponsors::enable_sponsorships(&gh, &owner, &name)?;
    let outcome = match status {
        sponsors::SponsorshipStatus::Enabled => "Sponsor button enabled",
        sponsors::SponsorshipStatus::AlreadyEnabled => "Sponsor button already enabled",
    };
    println!("✓ {owner}/{name}: {outcome}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_owner_repo_valid() {
        let (owner, repo) = parse_owner_repo("UniverLab/ghscaff").unwrap();
        assert_eq!(owner, "UniverLab");
        assert_eq!(repo, "ghscaff");
    }

    #[test]
    fn test_parse_owner_repo_missing_slash() {
        let err = parse_owner_repo("UniverLab-ghscaff")
            .unwrap_err()
            .to_string();
        assert!(err.contains("owner/repo"));
    }

    #[test]
    fn test_parse_owner_repo_too_many_slashes() {
        assert!(parse_owner_repo("a/b/c").is_err());
    }

    #[test]
    fn test_parse_owner_repo_empty_owner() {
        assert!(parse_owner_repo("/repo").is_err());
    }

    #[test]
    fn test_parse_owner_repo_empty_repo() {
        assert!(parse_owner_repo("owner/").is_err());
    }

    #[test]
    fn test_parse_owner_repo_empty_string() {
        assert!(parse_owner_repo("").is_err());
    }

    #[test]
    fn test_parse_owner_repo_only_slash() {
        assert!(parse_owner_repo("/").is_err());
    }

    #[test]
    fn test_parse_owner_repo_error_names_expected_format() {
        let err = parse_owner_repo("bad").unwrap_err().to_string();
        assert!(err.contains("Expected format: owner/repo"));
    }

    #[test]
    fn test_parse_owner_repo_error_names_input() {
        let err = parse_owner_repo("nonsense").unwrap_err().to_string();
        assert!(err.contains("nonsense"));
    }

    #[test]
    fn test_parse_owner_repo_with_hyphens() {
        let (owner, repo) = parse_owner_repo("my-org/my-repo").unwrap();
        assert_eq!(owner, "my-org");
        assert_eq!(repo, "my-repo");
    }

    #[test]
    fn test_parse_owner_repo_with_dots() {
        let (owner, repo) = parse_owner_repo("org/my.repo").unwrap();
        assert_eq!(owner, "org");
        assert_eq!(repo, "my.repo");
    }

    #[test]
    fn test_parse_owner_repo_whitespace_only() {
        assert!(parse_owner_repo("   ").is_err());
    }

    #[test]
    fn test_parse_owner_repo_leading_slash_only() {
        assert!(parse_owner_repo("//repo").is_err());
    }
}
