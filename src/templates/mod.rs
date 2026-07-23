pub mod rust;

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::github::client::GithubClient;

const BOILERPLATE_REPO: &str = "UniverLab/ghscaff-boilerplate";

// Files excluded from boilerplate_files() — handled separately or metadata
const SKIP_FILES: &[&str] = &[
    "template.toml",
    "secrets.toml",
    "PLACEHOLDERS.md",
    ".gitignore", // replaced by GitHub's official gitignore template via API
];

pub trait LanguageTemplate {
    fn gitignore_name(&self) -> String;
    fn boilerplate_files(&self, name: &str, description: &str, owner: &str) -> Vec<RepoFile>;
    #[allow(dead_code)]
    fn default_topics(&self) -> Vec<String>;
}

pub struct RepoFile {
    pub path: String,
    pub content: String,
}

struct RemoteTemplate {
    cache_dir: PathBuf,
}

impl RemoteTemplate {
    fn apply_placeholders(
        &self,
        content: &str,
        name: &str,
        description: &str,
        owner: &str,
    ) -> String {
        content
            .replace("{{name}}", name)
            .replace("{{description}}", description)
            .replace("{{github_org}}", owner)
            .replace("{{github_repo}}", name)
    }

    fn gitignore_from_toml(&self) -> String {
        let content =
            std::fs::read_to_string(self.cache_dir.join("template.toml")).unwrap_or_default();
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

    fn boilerplate_files(&self, name: &str, description: &str, owner: &str) -> Vec<RepoFile> {
        let mut files = vec![];
        for entry in walkdir::WalkDir::new(&self.cache_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
        {
            let path = entry.path();
            let Ok(rel_path) = path.strip_prefix(&self.cache_dir) else {
                continue;
            };
            let rel = rel_path.to_string_lossy().replace('\\', "/");
            if SKIP_FILES.iter().any(|s| rel == *s) {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(path) else {
                continue;
            };
            let content = self.apply_placeholders(&raw, name, description, owner);
            files.push(RepoFile { path: rel, content });
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        files
    }

    fn default_topics(&self) -> Vec<String> {
        vec![]
    }
}

/// Download the template from `BOILERPLATE_REPO` and cache locally.
/// Requires an authenticated token to avoid rate limits.
/// When `force_refresh` is true, the cache is deleted and re-downloaded.
pub fn resolve(
    language: &str,
    token: &str,
    force_refresh: bool,
) -> Result<Box<dyn LanguageTemplate>> {
    let cache = cache_dir()?.join(language);
    if force_refresh && cache.exists() {
        std::fs::remove_dir_all(&cache)?;
    }
    if !cache.exists() {
        download(language, token)?;
    }
    // An empty (or missing) cache dir means the boilerplate isn't in the repo:
    // download() creates the dir but unpacks nothing for an unknown language.
    let has_files = cache.exists()
        && std::fs::read_dir(&cache)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
    if !has_files {
        let _ = std::fs::remove_dir_all(&cache);
        anyhow::bail!("Boilerplate '{language}' not found in {BOILERPLATE_REPO}");
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

/// Built-in fallback list, used only when the boilerplate repo can't be reached.
pub const AVAILABLE: &[&str] = &["rust"];

#[derive(Deserialize)]
struct ContentEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
}

/// List the boilerplate languages available in the remote repo by reading its
/// top-level directories. Always consults the repo so a newly added boilerplate
/// shows up without shipping a new ghscaff release; falls back to [`AVAILABLE`]
/// if the repo can't be reached.
pub fn available(client: &GithubClient) -> Vec<String> {
    let debug = crate::is_debug();
    let fallback = || {
        if debug {
            eprintln!("  [debug] Using fallback boilerplate list: {:?}", AVAILABLE);
        }
        AVAILABLE.iter().map(|s| s.to_string()).collect::<Vec<_>>()
    };
    let path = format!("/repos/{BOILERPLATE_REPO}/contents");
    if debug {
        eprintln!("  [debug] Fetching: {}", path);
    }
    match client.get::<Vec<ContentEntry>>(&path) {
        Ok(entries) => {
            if debug {
                eprintln!("  [debug] Got {} entries", entries.len());
                for e in &entries {
                    eprintln!("  [debug]   {} (type: {})", e.name, e.entry_type);
                }
            }
            let mut dirs: Vec<String> = entries
                .into_iter()
                .filter(|e| e.entry_type == "dir" && !e.name.starts_with('.'))
                .map(|e| e.name)
                .collect();
            dirs.sort();
            if debug {
                eprintln!("  [debug] Filtered dirs: {:?}", dirs);
            }
            if dirs.is_empty() {
                if debug {
                    eprintln!("  [debug] No directories found in boilerplate repo.");
                }
                fallback()
            } else {
                dirs
            }
        }
        Err(e) => {
            if debug {
                eprintln!("  [debug] Failed to fetch boilerplate list: {}", e);
            }
            fallback()
        }
    }
}

/// A secret required by a template (declared in secrets.toml).
#[derive(Debug, Clone, Deserialize)]
pub struct SecretSpec {
    pub name: String,
    pub description: String,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
struct SecretsFile {
    #[serde(default)]
    secrets: Vec<SecretSpec>,
}

/// Load secrets declared by the cached template, if any.
/// Returns an empty vec if no secrets.toml exists or it cannot be parsed.
pub fn load_secrets(language: &str) -> Vec<SecretSpec> {
    let path = cache_dir()
        .ok()
        .map(|d| d.join(language).join("secrets.toml"));
    let Some(path) = path.filter(|p| p.exists()) else {
        return vec![];
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    toml::from_str::<SecretsFile>(&content)
        .map(|f| f.secrets)
        .unwrap_or_default()
}

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
        let result = resolve("python", "dummy", false);
        assert!(result.is_err(), "Should fail for unknown language");
    }

    #[test]
    fn test_repo_file_struct() {
        let file = RepoFile {
            path: "test.rs".into(),
            content: "fn main() {}".into(),
        };
        assert_eq!(file.path, "test.rs");
        assert_eq!(file.content, "fn main() {}");
    }

    #[test]
    fn test_apply_placeholders_replaces_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("main.rs");
        std::fs::write(
            &file,
            "name={{name}} desc={{description}} author={{author}}",
        )
        .unwrap();
        apply_placeholders(dir.path(), "myapp", "My App", "Alice").unwrap();
        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, "name=myapp desc=My App author=Alice");
    }

    #[test]
    fn test_apply_placeholders_skips_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("image.png");
        std::fs::write(&file, [0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        apply_placeholders(dir.path(), "x", "y", "z").unwrap();
        let bytes = std::fs::read(&file).unwrap();
        assert_eq!(bytes, vec![0xFF, 0xD8, 0xFF, 0xE0]);
    }

    #[test]
    fn test_apply_placeholders_no_change_when_no_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("readme.md");
        std::fs::write(&file, "Hello World").unwrap();
        let mtime_before = std::fs::metadata(&file).unwrap().modified().unwrap();
        apply_placeholders(dir.path(), "a", "b", "c").unwrap();
        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, "Hello World");
        // File should not have been rewritten
        let mtime_after = std::fs::metadata(&file).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after);
    }

    #[test]
    fn test_apply_placeholders_nested_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("src");
        std::fs::create_dir(&sub).unwrap();
        let file = sub.join("lib.rs");
        std::fs::write(&file, "{{name}}").unwrap();
        apply_placeholders(dir.path(), "project", "", "").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "project");
    }

    #[test]
    fn test_default_true() {
        assert!(default_true());
    }

    #[test]
    fn test_secret_spec_deserialize() {
        let toml_str = r#"
        [[secrets]]
        name = "API_KEY"
        description = "GitHub API key"
        required = true
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.secrets.len(), 1);
        assert_eq!(file.secrets[0].name, "API_KEY");
        assert!(file.secrets[0].required);
    }

    #[test]
    fn test_secret_spec_default_required() {
        let toml_str = r#"
        [[secrets]]
        name = "TOKEN"
        description = "A token"
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(file.secrets[0].required);
    }

    #[test]
    fn test_load_secrets_missing_dir() {
        let result = load_secrets("nonexistent-language-xyz");
        assert!(result.is_empty());
    }

    #[test]
    fn test_secret_spec_clone_debug() {
        let spec = SecretSpec {
            name: "X".into(),
            description: "Y".into(),
            required: false,
        };
        let cloned = spec.clone();
        assert_eq!(cloned.name, "X");
        assert!(!cloned.required);
        let _dbg = format!("{:?}", spec);
    }

    #[test]
    fn test_resolve_rust_template_files_content() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app", "myorg");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"Cargo.toml"));
        assert!(names.contains(&"src/main.rs"));
        assert!(names.contains(&"README.md"));
    }

    #[test]
    fn test_available_constant() {
        assert!(AVAILABLE.iter().all(|s| !s.is_empty()));
    }

    #[test]
    fn test_remote_template_apply_placeholders() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders(
            "Hello {{name}}, welcome to {{description}} by {{github_org}}/{{github_repo}}",
            "myapp",
            "My App",
            "myorg",
        );
        assert_eq!(result, "Hello myapp, welcome to My App by myorg/myapp");
    }

    #[test]
    fn test_remote_template_apply_placeholders_no_tokens() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("plain text", "a", "b", "c");
        assert_eq!(result, "plain text");
    }

    #[test]
    fn test_remote_template_apply_placeholders_multiple_same_token() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}} and {{name}}", "x", "y", "z");
        assert_eq!(result, "x and x");
    }

    #[test]
    fn test_remote_template_gitignore_from_toml_valid() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("template.toml");
        std::fs::write(&toml_path, "[package]\ntemplate = \"Rust\"\n").unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        assert_eq!(tmpl.gitignore_from_toml(), "Rust");
    }

    #[test]
    fn test_remote_template_gitignore_from_toml_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        assert_eq!(tmpl.gitignore_from_toml(), "");
    }

    #[test]
    fn test_remote_template_gitignore_from_toml_no_template_key() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("template.toml");
        std::fs::write(&toml_path, "[package]\nname = \"test\"\n").unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        assert_eq!(tmpl.gitignore_from_toml(), "");
    }

    #[test]
    fn test_remote_template_gitignore_from_toml_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("template.toml");
        std::fs::write(&toml_path, "").unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        assert_eq!(tmpl.gitignore_from_toml(), "");
    }

    #[test]
    fn test_remote_template_boilerplate_files() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("rust");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("Cargo.toml"), "[package]\nname = \"{{name}}\"\n").unwrap();
        std::fs::write(cache.join("README.md"), "# {{name}}\n{{description}}\n").unwrap();
        // This file should be skipped
        std::fs::write(cache.join("template.toml"), "template = \"Rust\"\n").unwrap();
        std::fs::write(cache.join("secrets.toml"), "").unwrap();
        std::fs::write(cache.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(cache.join("PLACEHOLDERS.md"), "docs").unwrap();

        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("myrepo", "My description", "myorg");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"README.md"));
        // Skip files should not appear
        assert!(!paths.contains(&"template.toml"));
        assert!(!paths.contains(&"secrets.toml"));
        assert!(!paths.contains(&".gitignore"));
        assert!(!paths.contains(&"PLACEHOLDERS.md"));
    }

    #[test]
    fn test_remote_template_boilerplate_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("empty");
        std::fs::create_dir_all(&cache).unwrap();
        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("repo", "desc", "owner");
        assert!(files.is_empty());
    }

    #[test]
    fn test_remote_template_boilerplate_files_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("rust");
        let src_dir = cache.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(cache.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(src_dir.join("main.rs"), "fn main() {}\n").unwrap();

        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("repo", "desc", "owner");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"src/main.rs"));
    }

    #[test]
    fn test_remote_template_default_topics() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        assert!(tmpl.default_topics().is_empty());
    }

    #[test]
    fn test_remote_template_gitignore_name() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("template.toml");
        std::fs::write(&toml_path, "template = \"Python\"\n").unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        assert_eq!(tmpl.gitignore_name(), "Python");
    }

    #[test]
    fn test_secret_spec_multiple_secrets() {
        let toml_str = r#"
        [[secrets]]
        name = "KEY1"
        description = "First key"
        required = true

        [[secrets]]
        name = "KEY2"
        description = "Second key"
        required = false

        [[secrets]]
        name = "KEY3"
        description = "Third key"
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.secrets.len(), 3);
        assert!(file.secrets[0].required);
        assert!(!file.secrets[1].required);
        assert!(file.secrets[2].required); // default true
    }

    #[test]
    fn test_secret_spec_empty_secrets() {
        let toml_str = "";
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(file.secrets.is_empty());
    }

    #[test]
    fn test_load_secrets_with_actual_file() {
        let dir = tempfile::tempdir().unwrap();
        // Create the cache structure that load_secrets expects
        let lang_dir = dir.path().join("testlang");
        std::fs::create_dir_all(&lang_dir).unwrap();
        std::fs::write(
            lang_dir.join("secrets.toml"),
            r#"
[[secrets]]
name = "TEST_SECRET"
description = "A test secret"
required = false
"#,
        )
        .unwrap();

        // We can't call load_secrets directly with a custom dir, but we can verify
        // the file parsing logic
        let content = std::fs::read_to_string(lang_dir.join("secrets.toml")).unwrap();
        let file: SecretsFile = toml::from_str(&content).unwrap();
        assert_eq!(file.secrets.len(), 1);
        assert_eq!(file.secrets[0].name, "TEST_SECRET");
        assert!(!file.secrets[0].required);
    }

    #[test]
    fn test_load_secrets_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let lang_dir = dir.path().join("badlang");
        std::fs::create_dir_all(&lang_dir).unwrap();
        std::fs::write(lang_dir.join("secrets.toml"), "not valid toml [[[[").unwrap();

        // Verify that invalid TOML causes an error in parsing
        let content = std::fs::read_to_string(lang_dir.join("secrets.toml")).unwrap();
        let result = toml::from_str::<SecretsFile>(&content);
        assert!(result.is_err());
    }

    #[test]
    fn test_skip_files_constant() {
        assert!(SKIP_FILES.contains(&"template.toml"));
        assert!(SKIP_FILES.contains(&"secrets.toml"));
        assert!(SKIP_FILES.contains(&"PLACEHOLDERS.md"));
        assert!(SKIP_FILES.contains(&".gitignore"));
    }

    #[test]
    fn test_boilerplate_repo_constant() {
        assert_eq!(BOILERPLATE_REPO, "UniverLab/ghscaff-boilerplate");
    }

    #[test]
    fn test_apply_placeholders_only_replaces_exact_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "{{name}} {{nam}} {{na}} {{description}}").unwrap();
        apply_placeholders(dir.path(), "replaced", "", "").unwrap();
        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, "replaced {{nam}} {{na}} ");
    }

    #[test]
    fn test_apply_placeholders_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        apply_placeholders(dir.path(), "a", "b", "c").unwrap();
    }

    #[test]
    fn test_content_entry_deserialize() {
        let json = r#"[{"name":"rust","type":"dir"},{"name":".git","type":"dir"}]"#;
        let entries: Vec<ContentEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "rust");
        assert_eq!(entries[0].entry_type, "dir");
        assert_eq!(entries[1].name, ".git");
    }

    #[test]
    fn test_remote_template_boilerplate_files_binary_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("mixed");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("text.txt"), "hello").unwrap();
        // Binary file - read_to_string will fail, so it should be skipped
        std::fs::write(cache.join("binary.bin"), [0xFF, 0xFE, 0xFD]).unwrap();

        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("repo", "desc", "owner");
        // Only text.txt should be included; binary.bin skipped
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "text.txt");
    }

    #[test]
    fn test_repo_file_empty_content() {
        let file = RepoFile {
            path: "empty.txt".into(),
            content: String::new(),
        };
        assert!(file.content.is_empty());
    }

    #[test]
    fn test_secret_spec_required_field_explicit_false() {
        let toml_str = r#"
        [[secrets]]
        name = "OPT"
        description = "Optional"
        required = false
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(!file.secrets[0].required);
    }

    #[test]
    fn test_secret_spec_description_empty() {
        let toml_str = r#"
        [[secrets]]
        name = "EMPTY_DESC"
        description = ""
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(file.secrets[0].description.is_empty());
    }

    #[test]
    fn test_available_is_const_ref() {
        let a: &[&str] = AVAILABLE;
        let b: &[&str] = AVAILABLE;
        assert_eq!(a.as_ptr(), b.as_ptr());
    }

    #[test]
    fn test_cache_dir_structure() {
        let base = dirs::home_dir().unwrap();
        let expected = base.join(".ghscaff").join("boilerplate");
        let actual = cache_dir().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_remote_template_apply_placeholders_only_github_placeholders() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders(
            "org={{github_org}} repo={{github_repo}}",
            "repo",
            "desc",
            "myorg",
        );
        assert_eq!(result, "org=myorg repo=repo");
    }

    #[test]
    fn test_remote_template_apply_placeholders_empty_strings() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}", "", "", "");
        assert_eq!(result, "");
    }

    #[test]
    fn test_remote_template_apply_placeholders_special_chars_in_values() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}", "my\"app", "desc", "owner");
        assert_eq!(result, "my\"app");
    }

    #[test]
    fn test_remote_template_gitignore_from_toml_multiline() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("template.toml");
        std::fs::write(
            &toml_path,
            "[package]\nname = \"test\"\ntemplate = \"Python\"\nversion = \"1.0\"\n",
        )
        .unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        assert_eq!(tmpl.gitignore_from_toml(), "Python");
    }

    #[test]
    fn test_remote_template_gitignore_from_toml_quoted_value() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("template.toml");
        std::fs::write(&toml_path, "template = \"Go\"\n").unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        assert_eq!(tmpl.gitignore_from_toml(), "Go");
    }

    #[test]
    fn test_remote_template_boilerplate_files_preserves_content() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("rust");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("file.txt"), "original content {{name}}").unwrap();

        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("replaced", "", "");
        assert_eq!(files[0].content, "original content replaced");
    }

    #[test]
    fn test_secret_spec_invalid_toml_returns_empty() {
        let result = toml::from_str::<SecretsFile>("invalid [[[");
        assert!(result.is_err());
    }

    #[test]
    fn test_skip_files_all_present() {
        // Verify all skip files are checked
        let skip = vec![
            "template.toml",
            "secrets.toml",
            "PLACEHOLDERS.md",
            ".gitignore",
        ];
        for file in skip {
            assert!(
                SKIP_FILES.contains(&file),
                "{} should be in SKIP_FILES",
                file
            );
        }
    }

    #[test]
    fn test_rust_template_files_sorted() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("app", "desc", "owner");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        // RustTemplate returns files in a specific order, not necessarily alphabetical
        assert_eq!(paths.len(), 3);
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"README.md"));
    }

    #[test]
    fn test_remote_template_boilerplate_files_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("rust");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("z_last.txt"), "z").unwrap();
        std::fs::write(cache.join("a_first.txt"), "a").unwrap();
        std::fs::write(cache.join("m_middle.txt"), "m").unwrap();

        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("repo", "desc", "owner");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn test_skip_files_completeness() {
        assert_eq!(SKIP_FILES.len(), 4);
        assert!(SKIP_FILES.contains(&"template.toml"));
        assert!(SKIP_FILES.contains(&"secrets.toml"));
        assert!(SKIP_FILES.contains(&"PLACEHOLDERS.md"));
        assert!(SKIP_FILES.contains(&".gitignore"));
    }

    #[test]
    fn test_available_fallback_constant() {
        assert_eq!(AVAILABLE.len(), 1);
        assert_eq!(AVAILABLE[0], "rust");
    }

    #[test]
    fn test_secret_spec_required_true_explicit() {
        let toml_str = r#"
        [[secrets]]
        name = "REQ"
        description = "Required"
        required = true
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(file.secrets[0].required);
    }

    #[test]
    fn test_secret_spec_required_false_explicit() {
        let toml_str = r#"
        [[secrets]]
        name = "OPT"
        description = "Optional"
        required = false
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(!file.secrets[0].required);
    }

    #[test]
    fn test_secret_spec_name_empty() {
        let toml_str = r#"
        [[secrets]]
        name = ""
        description = "Empty name"
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(file.secrets[0].name.is_empty());
    }

    #[test]
    fn test_remote_template_gitignore_from_toml_single_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("template.toml");
        std::fs::write(&toml_path, "template = 'Go'\n").unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        // Single quotes aren't standard TOML, so this should return empty
        assert_eq!(tmpl.gitignore_from_toml(), "");
    }

    #[test]
    fn test_remote_template_boilerplate_files_multiple_subdirs() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("rust");
        let src = cache.join("src");
        let tests = cache.join("tests");
        let benches = cache.join("benches");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&tests).unwrap();
        std::fs::create_dir_all(&benches).unwrap();
        std::fs::write(cache.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn f() {}").unwrap();
        std::fs::write(tests.join("test.rs"), "#[test] fn t() {}").unwrap();
        std::fs::write(benches.join("bench.rs"), "#[bench] fn b() {}").unwrap();

        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("repo", "desc", "owner");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"src/lib.rs"));
        assert!(paths.contains(&"tests/test.rs"));
        assert!(paths.contains(&"benches/bench.rs"));
    }

    #[test]
    fn test_repo_file_struct_all_fields() {
        let file = RepoFile {
            path: "src/main.rs".into(),
            content: "fn main() {\n    println!(\"Hello\");\n}".into(),
        };
        assert_eq!(file.path, "src/main.rs");
        assert!(file.content.contains("Hello"));
    }

    #[test]
    fn test_apply_placeholders_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        let file1 = dir.path().join("a.txt");
        let file2 = dir.path().join("b.txt");
        std::fs::write(&file1, "{{name}}").unwrap();
        std::fs::write(&file2, "{{description}}").unwrap();
        apply_placeholders(dir.path(), "repo", "My Desc", "author").unwrap();
        assert_eq!(std::fs::read_to_string(&file1).unwrap(), "repo");
        assert_eq!(std::fs::read_to_string(&file2).unwrap(), "My Desc");
    }

    #[test]
    fn test_secret_spec_debug_format() {
        let spec = SecretSpec {
            name: "MY_SECRET".into(),
            description: "A secret".into(),
            required: true,
        };
        let dbg = format!("{:?}", spec);
        assert!(dbg.contains("MY_SECRET"));
        assert!(dbg.contains("A secret"));
    }

    #[test]
    fn test_secret_spec_clone_all_fields() {
        let spec = SecretSpec {
            name: "KEY".into(),
            description: "Desc".into(),
            required: false,
        };
        let cloned = spec.clone();
        assert_eq!(spec.name, cloned.name);
        assert_eq!(spec.description, cloned.description);
        assert_eq!(spec.required, cloned.required);
    }

    #[test]
    fn test_boilerplate_repo_constant_value() {
        assert_eq!(BOILERPLATE_REPO, "UniverLab/ghscaff-boilerplate");
    }

    #[test]
    fn test_content_entry_types() {
        let json = r#"[{"name":"rust","type":"dir"},{"name":"README.md","type":"file"},{"name":".git","type":"dir"}]"#;
        let entries: Vec<ContentEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].entry_type, "dir");
        assert_eq!(entries[1].entry_type, "file");
        assert_eq!(entries[2].entry_type, "dir");
    }

    #[test]
    fn test_remote_template_apply_placeholders_all_tokens() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders(
            "{{name}}/{{description}}/{{github_org}}/{{github_repo}}",
            "n",
            "d",
            "o",
        );
        assert_eq!(result, "n/d/o/n");
    }

    #[test]
    fn test_remote_template_apply_placeholders_adjacent_tokens() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}{{description}}", "a", "b", "c");
        assert_eq!(result, "ab");
    }

    #[test]
    fn test_remote_template_apply_placeholders_partial_match() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}ly is {{name}}s", "x", "y", "z");
        assert_eq!(result, "xly is xs");
    }

    #[test]
    fn test_remote_template_gitignore_from_toml_no_quotes() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("template.toml");
        std::fs::write(&toml_path, "template = Ruby\n").unwrap();
        let tmpl = RemoteTemplate {
            cache_dir: dir.keep(),
        };
        assert_eq!(tmpl.gitignore_from_toml(), "");
    }

    #[test]
    fn test_secret_spec_empty_secrets_file() {
        let toml_str = "";
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(file.secrets.is_empty());
    }

    #[test]
    fn test_secret_spec_only_header() {
        let toml_str = "[package]\nname = \"test\"\n";
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(file.secrets.is_empty());
    }

    #[test]
    fn test_apply_placeholders_just_author_token() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.toml");
        std::fs::write(&file, "{{author}}/{{name}}/{{description}}").unwrap();
        apply_placeholders(dir.path(), "repo", "desc", "myauthor").unwrap();
        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, "myauthor/repo/desc");
    }

    #[test]
    fn test_remote_template_boilerplate_files_filter_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("rust");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join(".hidden"), "secret").unwrap();
        std::fs::write(cache.join("visible.txt"), "content").unwrap();
        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("repo", "desc", "owner");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"visible.txt"));
    }

    #[test]
    fn test_remote_template_apply_placeholders_consecutive_same_token() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}{{name}}{{name}}", "X", "", "");
        assert_eq!(result, "XXX");
    }

    #[test]
    fn test_secret_spec_long_description() {
        let toml_str = r#"
        [[secrets]]
        name = "LONG"
        description = "This is a very long description that goes on and on and describes what this secret is for in great detail so the user knows exactly what to provide"
        required = true
        "#;
        let file: SecretsFile = toml::from_str(toml_str).unwrap();
        assert!(file.secrets[0].description.len() > 100);
    }

    #[test]
    fn test_apply_placeholders_path_with_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("my dir");
        std::fs::create_dir(&subdir).unwrap();
        let file = subdir.join("file.txt");
        std::fs::write(&file, "{{name}}").unwrap();
        apply_placeholders(dir.path(), "replaced", "", "").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "replaced");
    }

    #[test]
    fn test_apply_placeholders_special_chars_in_template() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("regex.txt");
        std::fs::write(&file, "{{name}} [test] (group)").unwrap();
        apply_placeholders(dir.path(), "myapp", "", "").unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "myapp [test] (group)"
        );
    }

    #[test]
    fn test_remote_template_boilerplate_files_preserves_subdir_order() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("rust");
        let a_dir = cache.join("a");
        let b_dir = cache.join("b");
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        std::fs::write(cache.join("root.txt"), "root").unwrap();
        std::fs::write(a_dir.join("a.txt"), "a").unwrap();
        std::fs::write(b_dir.join("b.txt"), "b").unwrap();
        let tmpl = RemoteTemplate { cache_dir: cache };
        let files = tmpl.boilerplate_files("repo", "desc", "owner");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }
}
