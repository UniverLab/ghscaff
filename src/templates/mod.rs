pub mod rust;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const BOILERPLATE_REPO: &str = "UniverLab/ghscaff-boilerplate";

// Files excluded from boilerplate_files() — handled separately or metadata
const SKIP_FILES: &[&str] = &[
    "template.toml",
    "PLACEHOLDERS.md",
    ".gitignore",               // wizard fetches this from GitHub API
    ".github/workflows/ci.yml", // returned by ci_workflow()
];

pub trait LanguageTemplate {
    fn gitignore_name(&self) -> String;
    fn ci_workflow(&self, name: &str, description: &str, owner: &str) -> String;
    fn boilerplate_files(&self, name: &str, description: &str, owner: &str) -> Vec<RepoFile>;
    #[allow(dead_code)]
    fn default_topics(&self) -> Vec<String>;
}

pub struct RepoFile {
    pub path: String,
    pub content: String,
    pub commit_message: String,
}

struct RemoteTemplate {
    cache_dir: PathBuf,
}

impl RemoteTemplate {
    fn apply_placeholders(&self, content: &str, name: &str, description: &str, owner: &str) -> String {
        content
            .replace("{{name}}", name)
            .replace("{{description}}", description)
            .replace("{{github_org}}", owner)
            .replace("{{github_repo}}", name)
    }

    fn read_file(&self, rel: &str, name: &str, description: &str, owner: &str) -> Option<String> {
        let content = std::fs::read_to_string(self.cache_dir.join(rel)).ok()?;
        Some(self.apply_placeholders(&content, name, description, owner))
    }

    fn gitignore_from_toml(&self) -> String {
        let content = std::fs::read_to_string(self.cache_dir.join("template.toml"))
            .unwrap_or_default();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("template = ") {
                if let Some(val) = trimmed.split('"').nth(1) {
                    return val.to_string();
                }
            }
        }
        String::new()
    }
}

impl LanguageTemplate for RemoteTemplate {
    fn gitignore_name(&self) -> String {
        self.gitignore_from_toml()
    }

    fn ci_workflow(&self, name: &str, description: &str, owner: &str) -> String {
        self.read_file(".github/workflows/ci.yml", name, description, owner)
            .unwrap_or_default()
    }

    fn boilerplate_files(&self, name: &str, description: &str, owner: &str) -> Vec<RepoFile> {
        let mut files = vec![];
        for entry in walkdir::WalkDir::new(&self.cache_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path();
            let Ok(rel_path) = path.strip_prefix(&self.cache_dir) else { continue };
            // Normalize path separators to forward slash
            let rel = rel_path.to_string_lossy().replace('\\', "/");
            // Skip metadata and files handled separately by the wizard
            if SKIP_FILES.iter().any(|s| rel == *s) {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(path) else { continue };
            let content = self.apply_placeholders(&raw, name, description, owner);
            let commit_message = derive_commit_message(&rel);
            files.push(RepoFile { path: rel, content, commit_message });
        }
        // Sort for a deterministic commit order
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files
    }

    fn default_topics(&self) -> Vec<String> {
        vec![]
    }
}

fn derive_commit_message(path: &str) -> String {
    if path.starts_with(".github/workflows/") {
        let file = path.rsplit('/').next().unwrap_or(path);
        format!("ci: add {file}")
    } else if path == "README.md" {
        "docs: init README.md".into()
    } else if path.starts_with("src/") {
        format!("chore: init {path}")
    } else {
        format!("chore: add {path}")
    }
}

/// Download the template from `BOILERPLATE_REPO` and cache locally.
/// Requires an authenticated token to avoid rate limits.
pub fn resolve(language: &str, token: &str) -> Result<Box<dyn LanguageTemplate>> {
    if !AVAILABLE.contains(&language) {
        anyhow::bail!(
            "Unknown language: {language}. Available: {}",
            AVAILABLE.join(", ")
        );
    }
    let cache = cache_dir()?.join(language);
    if !cache.exists() {
        download(language, token)?;
    }
    if !cache.exists() {
        anyhow::bail!("Template '{language}' could not be fetched from {BOILERPLATE_REPO}");
    }
    Ok(Box::new(RemoteTemplate { cache_dir: cache }))
}

fn download(language: &str, token: &str) -> Result<()> {
    let url = format!("https://api.github.com/repos/{BOILERPLATE_REPO}/tarball/main");
    let bytes = reqwest::blocking::Client::new()
        .get(&url)
        .header("Authorization", format!("token {token}"))
        .header("User-Agent", "ghscaff")
        .send()
        .context("Failed to download boilerplate")?
        .bytes()
        .context("Failed to read boilerplate response")?;

    let gz = flate2::read::GzDecoder::new(bytes.as_ref());
    let mut archive = tar::Archive::new(gz);
    let dest = cache_dir()?.join(language);
    std::fs::create_dir_all(&dest)?;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        // Strip the top-level tarball directory (e.g. UniverLab-ghscaff-boilerplate-abc123/)
        let stripped: PathBuf = path.components().skip(1).collect();
        if stripped.starts_with(language) {
            let rel: PathBuf = stripped.components().skip(1).collect();
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

#[allow(dead_code)]
pub fn apply_placeholders(dir: &Path, name: &str, description: &str, author: &str) -> Result<()> {
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else { continue };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::rust::RustTemplate;
    #[test]
    fn test_available_languages() {
        assert!(!AVAILABLE.is_empty(), "Should have at least one language");
        assert!(AVAILABLE.contains(&"rust"));
    }

    #[test]
    fn test_resolve_rust_template_embedded() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app", "myorg");
        assert!(!files.is_empty());
    }

    #[test]
    fn test_resolve_unknown_language() {
        // Fails before any network call because "python" is not in AVAILABLE
        let result = resolve("python", "dummy");
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

    #[test]
    fn test_derive_commit_message() {
        assert_eq!(derive_commit_message("README.md"), "docs: init README.md");
        assert_eq!(derive_commit_message("src/main.rs"), "chore: init src/main.rs");
        assert_eq!(derive_commit_message("Cargo.toml"), "chore: add Cargo.toml");
        assert_eq!(
            derive_commit_message(".github/workflows/release.yml"),
            "ci: add release.yml"
        );
    }
}
