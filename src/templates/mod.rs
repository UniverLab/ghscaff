#![allow(dead_code)]
pub mod rust;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const BOILERPLATE_REPO: &str = "JheisonMB/ghscaff-boilerplate";

#[allow(dead_code)]
pub struct BoilerplateTemplate {
    pub name: String,
    pub description: String,
    pub gitignore: String,
    pub topics: Vec<String>,
}

pub trait LanguageTemplate {
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
    fn gitignore_name(&self) -> &'static str;
    fn ci_workflow(&self) -> &'static str;
    fn boilerplate_files(&self, repo_name: &str, description: &str) -> Vec<RepoFile>;
    #[allow(dead_code)]
    fn default_topics(&self) -> Vec<&'static str>;
}

pub struct RepoFile {
    pub path: String,
    pub content: String,
    pub commit_message: String,
}

/// Resolve a boilerplate: local cache → download from ghscaff-boilerplate → embedded fallback.
pub fn resolve_dir(language: &str) -> Result<PathBuf> {
    let cache = cache_dir()?.join(language);
    if cache.exists() {
        return Ok(cache);
    }
    download(language)?;
    if cache.exists() {
        return Ok(cache);
    }
    anyhow::bail!(
        "Boilerplate '{}' not found. Run 'ghscaff template add {}' to install.",
        language,
        language
    )
}

pub fn list_cached() -> Result<Vec<String>> {
    let dir = cache_dir()?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let names = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    Ok(names)
}

pub fn list_remote() -> Result<Vec<String>> {
    let url = format!("https://api.github.com/repos/{BOILERPLATE_REPO}/contents");
    let resp: Vec<serde_json::Value> = reqwest::blocking::Client::new()
        .get(&url)
        .header("User-Agent", "ghscaff")
        .send()
        .context("Failed to fetch boilerplate list")?
        .json()
        .context("Failed to parse boilerplate list")?;
    Ok(resp
        .iter()
        .filter(|e| e["type"] == "dir")
        .filter_map(|e| e["name"].as_str().map(String::from))
        .collect())
}

fn download(language: &str) -> Result<()> {
    let url = format!("https://api.github.com/repos/{BOILERPLATE_REPO}/tarball/main");
    // Download full tarball and extract only the language subdirectory
    let bytes = reqwest::blocking::Client::new()
        .get(&url)
        .header("User-Agent", "ghscaff")
        .send()
        .context("Failed to download boilerplate")?
        .bytes()
        .context("Failed to read response")?;

    let gz = flate2::read::GzDecoder::new(bytes.as_ref());
    let mut archive = tar::Archive::new(gz);
    let dest = cache_dir()?.join(language);
    std::fs::create_dir_all(&dest)?;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        // Strip the top-level directory (e.g. JheisonMB-ghscaff-boilerplate-abc123/)
        let stripped = path.components().skip(1).collect::<PathBuf>();
        // Only extract files under the requested language directory
        if stripped.starts_with(language) {
            let rel = stripped.components().skip(1).collect::<PathBuf>();
            if rel.as_os_str().is_empty() {
                continue;
            }
            let target = dest.join(&rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            entry.unpack(&target)?;
        }
    }
    Ok(())
}

fn cache_dir() -> Result<PathBuf> {
    let base = dirs::home_dir().context("Cannot resolve home directory")?;
    Ok(base.join(".ghscaff").join("boilerplate"))
}

/// Apply placeholders in all files under `dir`.
pub fn apply_placeholders(dir: &Path, name: &str, description: &str, author: &str) -> Result<()> {
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // Skip binary files
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let replaced = content
            .replace("{{name}}", name)
            .replace("{{description}}", description)
            .replace("{{author}}", author);
        if replaced != content {
            std::fs::write(path, replaced)?;
        }
    }
    Ok(())
}

pub const AVAILABLE: &[&str] = &["rust"];

// Keep the old in-memory trait for fallback / testing
pub fn resolve(language: &str) -> Result<Box<dyn LanguageTemplate>> {
    match language {
        "rust" => Ok(Box::new(rust::RustTemplate)),
        other => anyhow::bail!("Unknown language: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_languages() {
        assert!(AVAILABLE.len() > 0, "Should have at least one language");
        assert!(AVAILABLE.contains(&"rust"));
    }

    #[test]
    fn test_resolve_rust_template() {
        let tmpl = resolve("rust");
        assert!(tmpl.is_ok(), "Should successfully resolve rust template");
    }

    #[test]
    fn test_resolve_unknown_language() {
        let result = resolve("python");
        assert!(result.is_err(), "Should fail for unknown language");
    }

    #[test]
    fn test_repo_file_struct() {
        let file = RepoFile {
            path: "test.rs".into(),
            content: "fn main() {}".into(),
            commit_message: "chore: init".into(),
        };
        assert_eq!(file.path, "test.rs");
        assert_eq!(file.content, "fn main() {}");
        assert_eq!(file.commit_message, "chore: init");
    }
}
