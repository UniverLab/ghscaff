mod agent_registry;
mod agentic;
pub mod rust;

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::github::client::GithubClient;

const BOILERPLATE_REPO: &str = "UniverLab/ghscaff-boilerplate";

/// Snake_case module name derived from the kebab-case project name:
/// lowercased with every `-` replaced by `_` (`astro-denoise` →
/// `astro_denoise`). Used for the `{{module}}` placeholder so python
/// boilerplates can ship `src/{{module}}/…` instead of a package literally
/// named `src`.
fn module_name(name: &str) -> String {
    name.to_lowercase().replace('-', "_")
}

/// SPDX identifier rendered by the `{{license}}` placeholder. The wizard
/// stores the choice as a display string; `GPL-3.0` is not an SPDX id, so it
/// maps to `GPL-3.0-only`. `None` (no license chosen) renders empty.
fn license_spdx_id(license: Option<&str>) -> &str {
    match license {
        Some("GPL-3.0") => "GPL-3.0-only",
        Some(other) => other,
        None => "",
    }
}

/// Printed once when the rust template is rendered with no license chosen.
pub const NO_LICENSE_NOTICE: &str = "No license chosen: the crate is not publishable (publish = false) and no release workflow was created. Add a LICENSE and restore them to publish.";

/// True only for the rust no-license case, so the notice is printed once and
/// never for a python boilerplate or for a chosen license.
pub fn is_rust_without_license(language: &str, license: Option<&str>) -> bool {
    language == "rust" && license.is_none()
}

// Files excluded from boilerplate_files() — handled separately or metadata
const SKIP_FILES: &[&str] = &[
    "template.toml",
    "secrets.toml",
    "PLACEHOLDERS.md",
    ".gitignore", // replaced by GitHub's official gitignore template via API
];

/// Appends the registry-derived agentic block to a fetched (or empty/failed)
/// GitHub gitignore template. `fetched` is preserved unmodified as a prefix;
/// the agentic block always follows, separated by a blank line, so it
/// survives even when the template fetch failed and `fetched` is empty.
pub fn assemble_gitignore(fetched: &str) -> String {
    let mut out = String::from(fetched);
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(&agentic::block());
    out
}

pub trait LanguageTemplate {
    fn gitignore_name(&self) -> String;
    fn boilerplate_files(
        &self,
        name: &str,
        description: &str,
        owner: &str,
        license: Option<&str>,
    ) -> Vec<RepoFile>;
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
        license: &str,
    ) -> String {
        let module = module_name(name);
        content
            .replace("{{name}}", name)
            .replace("{{description}}", description)
            .replace("{{github_org}}", owner)
            .replace("{{github_repo}}", name)
            .replace("{{license}}", license)
            .replace("{{module}}", &module)
    }

    /// The boilerplate language, taken from the cache directory name that
    /// [`resolve`] created (`cache_dir()?.join(language)`).
    fn language(&self) -> String {
        self.cache_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
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

    fn boilerplate_files(
        &self,
        name: &str,
        description: &str,
        owner: &str,
        license: Option<&str>,
    ) -> Vec<RepoFile> {
        let language = self.language();
        let spdx = license_spdx_id(license);
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
            let content = self.apply_placeholders(&raw, name, description, owner, spdx);
            let path = self.apply_placeholders(&rel, name, description, owner, spdx);
            let Some(content) = adjust_for_no_license(&language, &path, content, license) else {
                continue;
            };
            files.push(RepoFile { path, content });
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

/// Adjust one already-rendered file for the "no license chosen" case. `None`
/// means the file must be omitted from the init commit; `Some(text)` is the
/// rewritten content. A chosen license leaves every file untouched.
///
/// In addition, no license means no `LICENSE` file — and for rust no
/// `release.yml` — so the rendered docs stop describing them: every
/// language's `README.md` loses its `## License` section and, for rust,
/// `CONTRIBUTING.md` loses its `## Release process` section plus the rest of
/// its dropped-publish story (the `release.yml` clause in the CI paragraph and
/// the `CARGO_REGISTRY_TOKEN` in `### Required repository secrets`).
///
/// * rust — the crate cannot be published without a LICENSE, so drop the
///   publish-only release workflow, stop referencing a LICENSE file, and pass
///   `publish-check: false` to the shared rust-ci workflow.
/// * python — drop the `license = …` line from `pyproject.toml`.
fn adjust_for_no_license(
    language: &str,
    rel: &str,
    content: String,
    license: Option<&str>,
) -> Option<String> {
    if license.is_some() {
        return Some(content);
    }
    let content = match rel {
        "README.md" => remove_markdown_section(&content, "## License"),
        "CONTRIBUTING.md" if language == "rust" => strip_rust_release_docs(&content),
        _ => content,
    };
    if language == "rust" {
        return match rel {
            ".github/workflows/release.yml" => None,
            "Cargo.toml" => Some(disable_rust_publish(&content)),
            ".github/workflows/ci.yml" => Some(add_publish_check_false(&content)),
            _ => Some(content),
        };
    }
    if language.starts_with("python") && rel == "pyproject.toml" {
        return Some(strip_python_license(&content));
    }
    Some(content)
}

/// `license-file = "LICENSE"` would make cargo look for a file that is not
/// created; `publish = false` is the correct marker for an unpublished crate.
fn disable_rust_publish(cargo_toml: &str) -> String {
    cargo_toml.replace("license-file = \"LICENSE\"", "publish = false")
}

/// Remove the `license = …` line from a pyproject.toml. `license-files` is a
/// different key and is left untouched.
fn strip_python_license(pyproject: &str) -> String {
    let kept: Vec<String> = pyproject
        .lines()
        .filter(|line| !is_python_license_line(line))
        .map(str::to_string)
        .collect();
    join_lines(kept, pyproject.ends_with('\n'))
}

/// True for a TOML `license = …` / `license=…` key line, however much
/// whitespace surrounds the `=`. `license-files = […]` is a different key and
/// does not match: the text right after `license` must lead to `=`.
fn is_python_license_line(line: &str) -> bool {
    line.trim_start()
        .strip_prefix("license")
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Add `publish-check: false` to the `rust-ci` job. If the job already has a
/// `with:` block the flag goes under it; otherwise a `with:` block is inserted
/// right after the job's `uses:` line. An existing `publish-check` line is
/// rewritten to `false` — a `publish-check: true` left in place would keep
/// failing the first PR of a crate that cannot be published, which is exactly
/// what this transform exists to prevent. Insertion is scoped to the `rust-ci`
/// job so a later job is never modified, and the file's trailing newline is
/// preserved.
fn add_publish_check_false(ci_yml: &str) -> String {
    let trailing = ci_yml.ends_with('\n');
    let mut lines: Vec<String> = ci_yml.lines().map(str::to_string).collect();
    let Some(job) = lines.iter().position(|l| l.trim() == "rust-ci:") else {
        return ci_yml.to_string();
    };
    let job_indent = indent_of(&lines[job]);
    let end = job_block_end(&lines, job, job_indent);
    if set_publish_check_false(&mut lines, job, end) {
        return join_lines(lines, trailing);
    }
    insert_publish_check(&mut lines, job, end, job_indent);
    join_lines(lines, trailing)
}

/// Rewrite an existing `publish-check:` line inside the job to `false`,
/// keeping its indentation; `false` when the job has no such line yet (the
/// caller then inserts one).
fn set_publish_check_false(lines: &mut [String], job: usize, end: usize) -> bool {
    let Some(i) = (job..end).find(|&i| lines[i].trim_start().starts_with("publish-check:")) else {
        return false;
    };
    let indent = indent_of(&lines[i]);
    lines[i] = format!("{}publish-check: false", " ".repeat(indent));
    true
}

fn insert_publish_check(lines: &mut Vec<String>, job: usize, end: usize, job_indent: usize) {
    let child = " ".repeat(job_indent + 2);
    let leaf = " ".repeat(job_indent + 4);
    let flag = format!("{leaf}publish-check: false");
    let with_line = format!("{child}with:");
    match (job..end).find(|&i| lines[i] == with_line) {
        Some(i) => lines.insert(i + 1, flag),
        None => {
            let anchor = (job..end)
                .find(|&i| lines[i].trim_start().starts_with("uses:"))
                .unwrap_or(job);
            lines.insert(anchor + 1, flag);
            lines.insert(anchor + 1, with_line);
        }
    }
}

/// Index one past the last line of the job starting at `job`: the next
/// non-blank line indented at or below the job's own indent, or end of file.
fn job_block_end(lines: &[String], job: usize, job_indent: usize) -> usize {
    (job + 1..lines.len())
        .find(|&i| {
            let t = lines[i].trim_start();
            !t.is_empty() && indent_of(&lines[i]) <= job_indent
        })
        .unwrap_or(lines.len())
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// `str::lines()` drops the final newline, so it is re-added when the source
/// had one — otherwise the rewritten file would lose it.
fn join_lines(lines: Vec<String>, trailing_newline: bool) -> String {
    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    out
}

/// Remove a markdown section: the line equal to `heading` (after trimming
/// trailing whitespace), every line up to (not including) the next line
/// starting with `## ` or the end of file, and the single blank line directly
/// above the heading if there is one. Returns `text` unchanged when the
/// heading is absent.
fn remove_markdown_section(text: &str, heading: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|l| l.trim_end() == heading) else {
        return text.to_string();
    };
    let end = (start + 1..lines.len())
        .find(|&i| lines[i].starts_with("## "))
        .unwrap_or(lines.len());
    let cut = usize::from(start > 0 && lines[start - 1].trim_end().is_empty());
    let mut kept: Vec<String> = lines[..start - cut]
        .iter()
        .copied()
        .map(str::to_string)
        .collect();
    kept.extend(lines[end..].iter().copied().map(str::to_string));
    join_lines(kept, text.ends_with('\n'))
}

/// CONTRIBUTING.md tells contributors how the crate is published in three
/// places, all of which become false for an unpublished crate: the `## Release
/// process` section describing the dropped `release.yml` workflow, the "and
/// automated releases (`.github/workflows/release.yml`)" clause in the CI
/// paragraph, and the `### Required repository secrets` subsection whose only
/// row is the `CARGO_REGISTRY_TOKEN` publish token. Trim the two sections and
/// the dead clause, keeping the true ci.yml sentence and the section heading.
fn strip_rust_release_docs(contributing: &str) -> String {
    let trimmed = remove_markdown_section(
        &remove_markdown_section(contributing, "## Release process"),
        "### Required repository secrets",
    );
    trimmed.replace(
        " and automated releases (`.github/workflows/release.yml`)",
        "",
    )
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
        let files = tmpl.boilerplate_files("my-app", "A test app", "myorg", Some("MIT"));
        assert!(!files.is_empty());
    }

    #[test]
    fn test_resolve_unknown_language() {
        let result = resolve("python", "dummy", false);
        assert!(result.is_err(), "Should fail for unknown language");
    }

    #[test]
    fn test_assemble_gitignore_rust_contains_language_and_agentic_patterns() {
        let fetched = "target/\nCargo.lock\n";
        let result = assemble_gitignore(fetched);
        assert!(result.contains("target/"), "missing Rust-specific pattern");
        assert!(result.contains(".claude/"), "missing agentic pattern");
    }

    #[test]
    fn test_assemble_gitignore_preserves_fetched_content_verbatim() {
        let fetched = "# Rust\ntarget/\nCargo.lock\n**/*.rs.bk\n";
        let result = assemble_gitignore(fetched);
        assert!(
            result.contains(fetched),
            "fetched template must appear unmodified and in full as a contiguous substring"
        );
    }

    #[test]
    fn test_assemble_gitignore_empty_fetched_still_yields_agentic_block() {
        let result = assemble_gitignore("");
        assert!(result.contains(".claude/"));
        assert!(result.contains(".cursor/"));
        assert!(result.contains("skills-lock.json"));
    }

    #[test]
    fn test_assemble_gitignore_no_pattern_matches_claude_or_agents_md() {
        let result = assemble_gitignore("target/\n");
        for line in result.lines() {
            assert_ne!(line.trim(), "CLAUDE.md", "must not ignore CLAUDE.md");
            assert_ne!(line.trim(), "AGENTS.md", "must not ignore AGENTS.md");
        }
    }

    /// With nothing fetched (the fetch failed or there is no template), the
    /// assembled `.gitignore` is exactly the block `gitkit ignore add agentic`
    /// writes from the same registry snapshot.
    #[test]
    fn test_assemble_gitignore_empty_fetched_is_exactly_the_gitkit_block() {
        assert_eq!(
            assemble_gitignore(""),
            include_str!("../../tests/fixtures/agentic-block.txt")
        );
    }

    #[test]
    fn test_assemble_gitignore_nonempty_contains_the_gitkit_block() {
        let result = assemble_gitignore("target/\n");
        assert!(result.starts_with("target/"), "{}", result);
        assert!(
            result.contains(include_str!("../../tests/fixtures/agentic-block.txt")),
            "the agentic block must follow the fetched template verbatim"
        );
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
        let files = tmpl.boilerplate_files("my-app", "A test app", "myorg", Some("MIT"));
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
            "",
        );
        assert_eq!(result, "Hello myapp, welcome to My App by myorg/myapp");
    }

    #[test]
    fn test_remote_template_apply_placeholders_no_tokens() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("plain text", "a", "b", "c", "");
        assert_eq!(result, "plain text");
    }

    #[test]
    fn test_remote_template_apply_placeholders_multiple_same_token() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}} and {{name}}", "x", "y", "z", "");
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
        let files = tmpl.boilerplate_files("myrepo", "My description", "myorg", Some("MIT"));
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
        let files = tmpl.boilerplate_files("repo", "desc", "owner", Some("MIT"));
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
        let files = tmpl.boilerplate_files("repo", "desc", "owner", Some("MIT"));
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
        let files = tmpl.boilerplate_files("repo", "desc", "owner", Some("MIT"));
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
            "",
        );
        assert_eq!(result, "org=myorg repo=repo");
    }

    #[test]
    fn test_remote_template_apply_placeholders_empty_strings() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}", "", "", "", "");
        assert_eq!(result, "");
    }

    #[test]
    fn test_remote_template_apply_placeholders_special_chars_in_values() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}", "my\"app", "desc", "owner", "");
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
        let files = tmpl.boilerplate_files("replaced", "", "", Some("MIT"));
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
        let files = tmpl.boilerplate_files("app", "desc", "owner", Some("MIT"));
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
        let files = tmpl.boilerplate_files("repo", "desc", "owner", Some("MIT"));
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
        let files = tmpl.boilerplate_files("repo", "desc", "owner", Some("MIT"));
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
            "{{name}}/{{description}}/{{github_org}}/{{github_repo}}/{{license}}/{{module}}",
            "my-app",
            "d",
            "o",
            "MIT",
        );
        assert_eq!(result, "my-app/d/o/my-app/MIT/my_app");
    }

    #[test]
    fn test_remote_template_apply_placeholders_adjacent_tokens() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}{{description}}", "a", "b", "c", "");
        assert_eq!(result, "ab");
    }

    #[test]
    fn test_remote_template_apply_placeholders_partial_match() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}ly is {{name}}s", "x", "y", "z", "");
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
        let files = tmpl.boilerplate_files("repo", "desc", "owner", Some("MIT"));
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"visible.txt"));
    }

    #[test]
    fn test_remote_template_apply_placeholders_consecutive_same_token() {
        let tmpl = RemoteTemplate {
            cache_dir: tempfile::tempdir().unwrap().keep(),
        };
        let result = tmpl.apply_placeholders("{{name}}{{name}}{{name}}", "X", "", "", "");
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
        let files = tmpl.boilerplate_files("repo", "desc", "owner", Some("MIT"));
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    // ── license choice: SPDX mapping, notice gate, no-license transforms ──

    /// The rust boilerplate as it exists upstream (`license-file = "LICENSE"`
    /// and a publish-only release workflow).
    const FIXTURE_CARGO_TOML: &str = r#"[package]
name = "{{name}}"
version = "0.1.0"
edition = "2021"
description = "{{description}}"
license-file = "LICENSE"
repository = "https://github.com/{{github_org}}/{{github_repo}}"
readme = "README.md"

[dependencies]
"#;

    const FIXTURE_CI_YML: &str = r#"name: CI

on:
  pull_request:
  workflow_dispatch:

jobs:
  rust-ci:
    uses: UniverLab/workflows/.github/workflows/rust-ci.yml@main
"#;

    const FIXTURE_RELEASE_YML: &str = r#"name: Release

on:
  push:
    tags:
      - "v*"

jobs:
  release:
    uses: UniverLab/workflows/.github/workflows/rust-release.yml@main
"#;

    /// Build a throwaway boilerplate cache whose directory is named after the
    /// language (that name is how `RemoteTemplate::language()` reads it) and
    /// return it with the `TempDir` that owns it.
    fn fixture(language: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, RemoteTemplate) {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join(language);
        std::fs::create_dir_all(&cache).unwrap();
        for (rel, content) in files {
            let target = cache.join(rel);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, content).unwrap();
        }
        let tmpl = RemoteTemplate { cache_dir: cache };
        (dir, tmpl)
    }

    fn rust_fixture() -> (tempfile::TempDir, RemoteTemplate) {
        fixture(
            "rust",
            &[
                ("Cargo.toml", FIXTURE_CARGO_TOML),
                (".github/workflows/ci.yml", FIXTURE_CI_YML),
                (".github/workflows/release.yml", FIXTURE_RELEASE_YML),
                ("src/main.rs", "fn main() {}\n"),
                ("README.md", "# {{name}}\n\n{{description}}\n"),
            ],
        )
    }

    fn find<'a>(files: &'a [RepoFile], path: &str) -> &'a str {
        files
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.content.as_str())
            .unwrap_or_else(|| panic!("{path} missing from rendered files"))
    }

    #[test]
    fn license_spdx_id_maps_wizard_choices() {
        assert_eq!(license_spdx_id(None), "");
        assert_eq!(license_spdx_id(Some("MIT")), "MIT");
        assert_eq!(license_spdx_id(Some("Apache-2.0")), "Apache-2.0");
        assert_eq!(license_spdx_id(Some("GPL-3.0")), "GPL-3.0-only");
    }

    #[test]
    fn is_rust_without_license_truth_table() {
        assert!(is_rust_without_license("rust", None));
        assert!(!is_rust_without_license("rust", Some("MIT")));
        assert!(!is_rust_without_license("python-module", None));
        assert!(!is_rust_without_license("go", Some("MIT")));
    }

    #[test]
    fn no_license_notice_is_the_documented_text() {
        assert_eq!(
            NO_LICENSE_NOTICE,
            "No license chosen: the crate is not publishable (publish = false) and no release workflow was created. Add a LICENSE and restore them to publish."
        );
    }

    #[test]
    fn disable_rust_publish_replaces_license_file() {
        let out = disable_rust_publish(FIXTURE_CARGO_TOML);
        assert!(out.contains("publish = false"));
        assert!(!out.contains("license-file"));
        assert!(out.contains("name = \"{{name}}\""));
        assert!(out.ends_with("[dependencies]\n"));
    }

    #[test]
    fn strip_python_license_removes_license_line() {
        let input = "[project]\nname = \"demo\"\nlicense = \"MIT\"\nlicense-files = [\"LICENSE\"]\nversion = \"1\"\n";
        let out = strip_python_license(input);
        assert!(!out.contains("license ="));
        assert!(!out.contains("license=\""));
        assert!(out.contains("license-files = [\"LICENSE\"]"));
        assert_eq!(
            out,
            "[project]\nname = \"demo\"\nlicense-files = [\"LICENSE\"]\nversion = \"1\"\n"
        );
    }

    #[test]
    fn strip_python_license_handles_compact_and_trailing_newline() {
        let out = strip_python_license("[project]\nlicense=\"MIT\"\nname = \"demo\"");
        assert_eq!(out, "[project]\nname = \"demo\"");
    }

    #[test]
    fn strip_python_license_tolerates_extra_whitespace_around_equals() {
        let input = "[project]\nname  = \"demo\"\nlicense  = \"MIT\"\nlicense\t= \"GPL\"\n";
        assert_eq!(strip_python_license(input), "[project]\nname  = \"demo\"\n");
    }

    #[test]
    fn add_publish_check_false_inserts_with_block() {
        let out = add_publish_check_false(FIXTURE_CI_YML);
        assert!(out.contains("    with:\n      publish-check: false"));
        assert!(out.ends_with("publish-check: false\n"));
        assert!(!out.contains("with:\n    with:"));
    }

    #[test]
    fn add_publish_check_false_uses_existing_with_block() {
        let input = "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n    with:\n      args: --all\n";
        let out = add_publish_check_false(input);
        assert_eq!(
            out,
            "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n    with:\n      publish-check: false\n      args: --all\n"
        );
    }

    #[test]
    fn add_publish_check_false_rewrites_existing_publish_check() {
        // A `publish-check: true` would keep failing the first PR of an
        // unpublished crate, so it must be corrected, not preserved.
        let input =
            "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n    with:\n      publish-check: true\n";
        assert_eq!(
            add_publish_check_false(input),
            "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n    with:\n      publish-check: false\n"
        );
    }

    #[test]
    fn add_publish_check_false_is_idempotent_when_already_false() {
        let input =
            "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n    with:\n      publish-check: false\n";
        assert_eq!(add_publish_check_false(input), input);
    }

    #[test]
    fn add_publish_check_false_without_rust_ci_is_unchanged() {
        let input = "jobs:\n  python-ci:\n    uses: org/ci.yml@main\n";
        assert_eq!(add_publish_check_false(input), input);
    }

    #[test]
    fn add_publish_check_false_stops_at_next_job() {
        let input = "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n  other:\n    uses: org/other.yml@main\n";
        let out = add_publish_check_false(input);
        assert_eq!(
            out,
            "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n    with:\n      publish-check: false\n  other:\n    uses: org/other.yml@main\n"
        );
    }

    /// A blank line INSIDE the `rust-ci` job does not end it: the `with:` block
    /// below the blank line still belongs to the job and its stale
    /// `publish-check: true` must be rewritten, not duplicated.
    #[test]
    fn add_publish_check_false_reaches_past_an_inner_blank_line() {
        let input = "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n\n    with:\n      publish-check: true\n  other:\n    uses: org/other.yml@main\n";
        assert_eq!(
            add_publish_check_false(input),
            "jobs:\n  rust-ci:\n    uses: org/ci.yml@main\n\n    with:\n      publish-check: false\n  other:\n    uses: org/other.yml@main\n"
        );
    }

    /// The inserted `with:` key sits `job_indent + 2` under the job key, so a
    /// job indented deeper than the usual two spaces still yields valid YAML.
    #[test]
    fn add_publish_check_false_indents_insertion_under_a_deep_job() {
        let input = "name: CI\non: push\njobs:\n    rust-ci:\n      uses: org/ci.yml@main\n";
        assert_eq!(
            add_publish_check_false(input),
            "name: CI\non: push\njobs:\n    rust-ci:\n      uses: org/ci.yml@main\n      with:\n        publish-check: false\n"
        );
    }

    /// Only `pyproject.toml` of a python project loses its license line: a
    /// python project's other files and a non-python project's pyproject.toml
    /// are left untouched (`&&`, not `||`).
    #[test]
    fn adjust_for_no_license_touches_only_python_pyproject_files() {
        let body = "name = \"demo\"\nlicense = \"MIT\"\n";
        for (lang, rel) in [("python-module", "main.py"), ("go", "pyproject.toml")] {
            assert_eq!(
                adjust_for_no_license(lang, rel, body.to_string(), None).as_deref(),
                Some(body),
                "{lang}/{rel} must keep its license line untouched"
            );
        }
    }

    #[test]
    fn adjust_for_no_license_routing() {
        let cargo = adjust_for_no_license("rust", "Cargo.toml", FIXTURE_CARGO_TOML.into(), None)
            .expect("Cargo.toml is kept");
        assert!(cargo.contains("publish = false"));
        assert!(adjust_for_no_license(
            "rust",
            ".github/workflows/release.yml",
            "jobs:".into(),
            None
        )
        .is_none());
        assert_eq!(
            adjust_for_no_license("rust", "src/main.rs", "fn main() {}".into(), None).as_deref(),
            Some("fn main() {}")
        );
        for lang in ["python-module", "python-fastapi"] {
            let out = adjust_for_no_license(
                lang,
                "pyproject.toml",
                "[project]\nlicense = \"MIT\"\n".into(),
                None,
            )
            .expect("pyproject.toml is kept");
            assert_eq!(out, "[project]\n");
        }
        assert_eq!(
            adjust_for_no_license("go", "go.mod", "module x\n".into(), None).as_deref(),
            Some("module x\n")
        );
    }

    #[test]
    fn adjust_for_no_license_with_chosen_license_is_identity() {
        assert_eq!(
            adjust_for_no_license("rust", "Cargo.toml", FIXTURE_CARGO_TOML.into(), Some("MIT"))
                .as_deref(),
            Some(FIXTURE_CARGO_TOML)
        );
        assert_eq!(
            adjust_for_no_license(
                "rust",
                ".github/workflows/release.yml",
                "jobs:\n".into(),
                Some("MIT")
            )
            .as_deref(),
            Some("jobs:\n")
        );
    }

    #[test]
    fn remote_template_apply_placeholders_replaces_license() {
        let (_dir, tmpl) = fixture("rust", &[]);
        assert_eq!(
            tmpl.apply_placeholders("license = \"{{license}}\"", "n", "d", "o", "MIT"),
            "license = \"MIT\""
        );
        assert_eq!(
            tmpl.apply_placeholders("license = \"{{license}}\"", "n", "d", "o", ""),
            "license = \"\""
        );
    }

    #[test]
    fn render_rust_fixture_with_mit_keeps_publish_files() {
        let (_dir, tmpl) = rust_fixture();
        let files = tmpl.boilerplate_files("myrepo", "My description", "myorg", Some("MIT"));
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&".github/workflows/release.yml"));

        let cargo = find(&files, "Cargo.toml");
        assert!(cargo.contains("license-file = \"LICENSE\""));
        assert!(!cargo.contains("publish = false"));
        assert!(cargo.contains("name = \"myrepo\""));
        assert!(cargo.contains("https://github.com/myorg/myrepo"));

        let ci = find(&files, ".github/workflows/ci.yml");
        assert!(!ci.contains("publish-check"));
        assert_eq!(ci, FIXTURE_CI_YML);
    }

    #[test]
    fn render_rust_fixture_without_license_disables_publish() {
        let (_dir, tmpl) = rust_fixture();
        let files = tmpl.boilerplate_files("myrepo", "My description", "myorg", None);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(!paths.contains(&".github/workflows/release.yml"));

        let cargo = find(&files, "Cargo.toml");
        assert!(cargo.contains("publish = false"));
        assert!(!cargo.contains("license-file"));

        assert!(find(&files, ".github/workflows/ci.yml")
            .contains("    with:\n      publish-check: false"));
        assert_eq!(find(&files, "README.md"), "# myrepo\n\nMy description\n");
        assert_eq!(find(&files, "src/main.rs"), "fn main() {}\n");
    }

    #[test]
    fn render_python_fixture_substitutes_license_placeholder() {
        let (_dir, tmpl) = fixture(
            "python-module",
            &[(
                "pyproject.toml",
                "[project]\nname = \"{{name}}\"\nlicense = \"{{license}}\"\n",
            )],
        );
        let files = tmpl.boilerplate_files("demo", "d", "org", Some("Apache-2.0"));
        assert_eq!(
            find(&files, "pyproject.toml"),
            "[project]\nname = \"demo\"\nlicense = \"Apache-2.0\"\n"
        );
    }

    #[test]
    fn render_python_fixture_without_license_has_no_license_line() {
        let (_dir, tmpl) = fixture(
            "python-module",
            &[(
                "pyproject.toml",
                "[project]\nname = \"{{name}}\"\nlicense = \"{{license}}\"\n",
            )],
        );
        let files = tmpl.boilerplate_files("demo", "d", "org", None);
        assert_eq!(
            find(&files, "pyproject.toml"),
            "[project]\nname = \"demo\"\n"
        );
        assert!(!find(&files, "pyproject.toml")
            .lines()
            .any(|l| l.starts_with("license")));
    }

    #[test]
    fn render_python_current_boilerplate_keeps_hardcoded_mit() {
        let (_dir, tmpl) = fixture(
            "python-module",
            &[(
                "pyproject.toml",
                "[project]\nname = \"{{name}}\"\nlicense = \"MIT\"\n",
            )],
        );
        let files = tmpl.boilerplate_files("demo", "d", "org", Some("Apache-2.0"));
        assert_eq!(
            find(&files, "pyproject.toml"),
            "[project]\nname = \"demo\"\nlicense = \"MIT\"\n"
        );
    }

    #[test]
    fn render_python_fastapi_fixture_strips_license() {
        let (_dir, tmpl) = fixture(
            "python-fastapi",
            &[(
                "pyproject.toml",
                "[project]\nlicense = \"{{license}}\"\nversion = \"0.1.0\"\n",
            )],
        );
        let files = tmpl.boilerplate_files("api", "d", "org", None);
        assert_eq!(
            find(&files, "pyproject.toml"),
            "[project]\nversion = \"0.1.0\"\n"
        );
    }

    #[test]
    fn module_name_converts_kebab_to_snake() {
        assert_eq!(module_name("astro-denoise"), "astro_denoise");
        assert_eq!(module_name("my-app"), "my_app");
        assert_eq!(module_name("UPPER-Case"), "upper_case");
        assert_eq!(module_name("already_snake"), "already_snake");
        assert_eq!(module_name("a-b-c"), "a_b_c");
    }

    #[test]
    fn remote_template_apply_placeholders_replaces_module() {
        let (_dir, tmpl) = fixture("python-module", &[]);
        assert_eq!(
            tmpl.apply_placeholders("from {{module}}.core import x", "astro-denoise", "", "", ""),
            "from astro_denoise.core import x"
        );
    }

    #[test]
    fn render_python_module_fixture_substitutes_path_and_entry_point() {
        let (_dir, tmpl) = fixture(
            "python-module",
            &[
                ("src/{{module}}/__init__.py", ""),
                (
                    "pyproject.toml",
                    "[project]\nname = \"{{name}}\"\n[project.scripts]\n{{name}} = \"{{module}}.cli:main\"\n",
                ),
            ],
        );
        let files = tmpl.boilerplate_files("astro-denoise", "Denoise spectra", "univerlab", None);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(
            paths.contains(&"src/astro_denoise/__init__.py"),
            "paths: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("{{module}}")),
            "no path should still contain a placeholder: {paths:?}"
        );
        let pyproject = find(&files, "pyproject.toml");
        assert!(pyproject.contains("\"astro_denoise.cli:main\""));
        assert!(!pyproject.contains("{{module}}"));
    }

    #[test]
    fn render_paths_without_placeholders_are_unchanged() {
        let (_dir, tmpl) = rust_fixture();
        let files = tmpl.boilerplate_files("my-cool-app", "desc", "org", Some("MIT"));
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            &[
                ".github/workflows/ci.yml",
                ".github/workflows/release.yml",
                "Cargo.toml",
                "README.md",
                "src/main.rs",
            ]
        );
    }

    /// FR5 evidence — prints the rendered file list and the license-relevant
    /// lines for MIT and for no license, from throwaway fixtures only. No
    /// repository is created and no network call is made.
    #[test]
    fn evidence_license_rendering_rust_and_python() {
        let (_d1, rust) = rust_fixture();
        let (_d2, python) = fixture(
            "python-module",
            &[(
                "pyproject.toml",
                "[project]\nname = \"{{name}}\"\nlicense = \"{{license}}\"\n",
            )],
        );

        let mit = rust.boilerplate_files("demo", "A demo", "univerlab", Some("MIT"));
        println!(
            "[evidence] rust/MIT  files: {}",
            mit.iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!(
            "[evidence] rust/MIT  Cargo.toml: {}",
            find(&mit, "Cargo.toml")
                .lines()
                .find(|l| l.starts_with("license") || l.starts_with("publish"))
                .unwrap_or("<none>")
        );
        println!(
            "[evidence] rust/MIT  ci.yml: {}; release.yml: present",
            if find(&mit, ".github/workflows/ci.yml").contains("publish-check") {
                "publish-check present"
            } else {
                "no publish-check"
            }
        );

        let none = rust.boilerplate_files("demo", "A demo", "univerlab", None);
        println!(
            "[evidence] rust/None files: {}",
            none.iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        println!(
            "[evidence] rust/None Cargo.toml: {}",
            find(&none, "Cargo.toml")
                .lines()
                .find(|l| l.starts_with("license") || l.starts_with("publish"))
                .unwrap_or("<none>")
        );
        println!(
            "[evidence] rust/None ci.yml: {}",
            find(&none, ".github/workflows/ci.yml")
                .lines()
                .filter(|l| l.trim() == "with:" || l.contains("publish-check"))
                .collect::<Vec<_>>()
                .join(" / ")
        );
        println!(
            "[evidence] rust/None release.yml: {}",
            if none
                .iter()
                .any(|f| f.path == ".github/workflows/release.yml")
            {
                "present"
            } else {
                "absent"
            }
        );

        for license in [Some("Apache-2.0"), None] {
            let label = license.unwrap_or("None");
            let files = python.boilerplate_files("demo", "A demo", "univerlab", license);
            println!(
                "[evidence] python/{label} pyproject.toml: {}",
                find(&files, "pyproject.toml")
                    .lines()
                    .find(|l| l.starts_with("license"))
                    .unwrap_or("no license line")
            );
        }

        // The printed lines above are also asserted, so the evidence cannot
        // drift from behaviour.
        assert!(mit
            .iter()
            .any(|f| f.path == ".github/workflows/release.yml"));
        assert!(find(&mit, "Cargo.toml").contains("license-file = \"LICENSE\""));
        assert!(!find(&mit, ".github/workflows/ci.yml").contains("publish-check"));
        assert!(!none
            .iter()
            .any(|f| f.path == ".github/workflows/release.yml"));
        assert!(find(&none, "Cargo.toml").contains("publish = false"));
        assert!(!find(&none, "Cargo.toml").contains("license-file"));
        assert!(find(&none, ".github/workflows/ci.yml")
            .contains("    with:\n      publish-check: false"));
    }

    // ── remove_markdown_section + doc trimming without a license ──

    /// Mirrors the real boilerplate README: `## License` is the last `## `
    /// section, preceded by a blank line.
    const FIXTURE_README_LICENSE: &str = "# {{name}}\n\n{{description}}\n\n## Getting started\n\nRun `cargo run`.\n\n## License\n\nThis project is licensed under the MIT License — see LICENSE for details.\n";

    /// Mirrors the real rust CONTRIBUTING outside the `{{...}}` placeholders:
    /// the crate-publish story in `## Release process`, again in the CI
    /// paragraph, and the `CARGO_REGISTRY_TOKEN` story in `### Required
    /// repository secrets` — all of which stop being true without a license.
    const FIXTURE_CONTRIBUTING_RELEASE: &str = concat!(
        "# Contributing to {{name}}\n\nThank you for your interest in contributing!\n",
        "\n## Development workflow\n\nFork, branch, PR.\n",
        "\n## CI/CD and required secrets\n",
        "\nThis project uses GitHub Actions for CI (`.github/workflows/ci.yml`)",
        " and automated releases (`.github/workflows/release.yml`).\n",
        "\n### Required repository secrets\n",
        "\n| Secret | Description | Where to get it |\n|---|---|---|\n",
        "| `CARGO_REGISTRY_TOKEN` | API token to publish crates to [crates.io](https://crates.io)",
        " | [crates.io/me](https://crates.io/me) → API Tokens → New Token |\n",
        "\n> **Tip:** If you use [ghscaff](https://github.com/UniverLab/ghscaff), you can run",
        " `ghscaff apply` to configure missing secrets interactively, or set the env var",
        " before running:\n> ```bash\n> export CARGO_REGISTRY_TOKEN=<your_token>\n",
        "> ghscaff apply\n> ```\n",
        "\n## Release process\n\nReleases are automated via the `release.yml` workflow.\n",
        "\nThe workflow builds binaries and publishes to crates.io.\n",
        "\n## Code style\n\nRun `cargo fmt`.\n"
    );

    #[test]
    fn remove_markdown_section_removes_middle_section() {
        // The single blank line above the heading leaves with the section, so
        // the next heading ends up glued to the preceding line (spec rule).
        assert_eq!(
            remove_markdown_section("top\n\n## A\nbody\nmore\n\n## B\ntail\n", "## A"),
            "top\n## B\ntail\n"
        );
    }

    #[test]
    fn remove_markdown_section_removes_end_of_file_section() {
        assert_eq!(
            remove_markdown_section("keep\n\n## License\n\nlicensed text\n", "## License"),
            "keep\n"
        );
        assert_eq!(
            remove_markdown_section("keep\n\n## A\nbody", "## A"),
            "keep"
        );
    }

    #[test]
    fn remove_markdown_section_heading_at_first_line_keeps_no_blank() {
        assert_eq!(remove_markdown_section("## A\nbody", "## A"), "");
    }

    #[test]
    fn remove_markdown_section_keeps_non_blank_line_above_heading() {
        assert_eq!(
            remove_markdown_section("keep\n## A\nbody\n", "## A"),
            "keep\n"
        );
    }

    #[test]
    fn remove_markdown_section_absent_heading_is_unchanged() {
        let text = "# Title\n\n## Other\n";
        assert_eq!(remove_markdown_section(text, "## License"), text);
    }

    #[test]
    fn remove_markdown_section_does_not_match_deeper_subsection() {
        // `### License` is neither the heading nor a section end, so nothing
        // is removed.
        let text = "## Foo\n\n### License\n\nx\n";
        assert_eq!(remove_markdown_section(text, "## License"), text);
        // Trailing whitespace on the heading line is trimmed before matching.
        assert_eq!(
            remove_markdown_section("keep\n\n## License  \nbody\n", "## License"),
            "keep\n"
        );
    }

    fn docs_fixture(language: &str) -> (tempfile::TempDir, RemoteTemplate) {
        fixture(
            language,
            &[
                ("README.md", FIXTURE_README_LICENSE),
                ("CONTRIBUTING.md", FIXTURE_CONTRIBUTING_RELEASE),
            ],
        )
    }

    #[test]
    fn render_rust_docs_fixture_without_license_drops_license_and_release_sections() {
        let (_dir, tmpl) = docs_fixture("rust");
        let files = tmpl.boilerplate_files("myrepo", "My description", "myorg", None);

        let readme = find(&files, "README.md");
        assert!(!readme.contains("## License"));
        assert!(!readme.contains("licensed under"));
        assert!(readme.contains("## Getting started"));

        let contributing = find(&files, "CONTRIBUTING.md");
        assert!(!contributing.contains("## Release process"));
        assert!(!contributing.contains("release.yml"));
        assert!(!contributing.contains("automated releases"));
        assert!(!contributing.contains("CARGO_REGISTRY_TOKEN"));
        assert!(!contributing.contains("Required repository secrets"));
        assert!(!contributing.contains("crates.io"));
        assert!(contributing.contains("## Development workflow"));
        assert!(contributing.contains("## CI/CD and required secrets"));
        assert!(contributing
            .contains("This project uses GitHub Actions for CI (`.github/workflows/ci.yml`)."));
        assert!(contributing.contains("## Code style"));
        assert!(contributing.contains("Fork, branch, PR."));
        // The blank line above each removed heading went with the section, so
        // what follows follows the preceding body line directly (spec rule).
        assert!(contributing.contains("Fork, branch, PR.\n\n## CI/CD and required secrets"));
        assert!(contributing.contains("ci.yml`).\n## Code style"));
    }

    #[test]
    fn render_rust_docs_fixture_with_mit_keeps_both_sections() {
        let (_dir, tmpl) = docs_fixture("rust");
        let files = tmpl.boilerplate_files("myrepo", "My description", "myorg", Some("MIT"));
        assert_eq!(
            find(&files, "README.md"),
            "# myrepo\n\nMy description\n\n## Getting started\n\nRun `cargo run`.\n\n## License\n\nThis project is licensed under the MIT License — see LICENSE for details.\n"
        );
        assert_eq!(
            find(&files, "CONTRIBUTING.md"),
            concat!(
                "# Contributing to myrepo\n\nThank you for your interest in contributing!\n",
                "\n## Development workflow\n\nFork, branch, PR.\n",
                "\n## CI/CD and required secrets\n",
                "\nThis project uses GitHub Actions for CI (`.github/workflows/ci.yml`)",
                " and automated releases (`.github/workflows/release.yml`).\n",
                "\n### Required repository secrets\n",
                "\n| Secret | Description | Where to get it |\n|---|---|---|\n",
                "| `CARGO_REGISTRY_TOKEN` | API token to publish crates to [crates.io](https://crates.io)",
                " | [crates.io/me](https://crates.io/me) → API Tokens → New Token |\n",
                "\n> **Tip:** If you use [ghscaff](https://github.com/UniverLab/ghscaff), you can run",
                " `ghscaff apply` to configure missing secrets interactively, or set the env var",
                " before running:\n> ```bash\n> export CARGO_REGISTRY_TOKEN=<your_token>\n",
                "> ghscaff apply\n> ```\n",
                "\n## Release process\n\nReleases are automated via the `release.yml` workflow.\n",
                "\nThe workflow builds binaries and publishes to crates.io.\n",
                "\n## Code style\n\nRun `cargo fmt`.\n"
            )
        );
    }

    #[test]
    fn render_python_docs_fixture_without_license_keeps_release_process() {
        let (_dir, tmpl) = docs_fixture("python-module");

        let none = tmpl.boilerplate_files("demo", "d", "org", None);
        assert!(!find(&none, "README.md").contains("## License"));
        assert!(find(&none, "CONTRIBUTING.md").contains("## Release process"));

        let mit = tmpl.boilerplate_files("demo", "d", "org", Some("MIT"));
        assert!(find(&mit, "README.md").contains("## License"));
        assert!(find(&mit, "CONTRIBUTING.md").contains("## Release process"));
    }

    // ── FR4 evidence: the REAL boilerplates, rendered with and without a license ──

    /// Recursively copy a boilerplate language tree into the scratch dir,
    /// keeping the language directory name (`RemoteTemplate::language()` reads
    /// it from `cache_dir`). The originals are only read, never written.
    fn copy_tree(src: &Path, dst: &Path) {
        std::fs::create_dir_all(dst).unwrap();
        for entry in std::fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let target = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    fn print_headings(prefix: &str, content: &str) {
        for line in content.lines().filter(|l| l.starts_with('#')) {
            eprintln!("{prefix}{line}");
        }
    }

    /// What the printed evidence must show, asserted so it cannot drift:
    /// `None` drops `## License` from every README and `## Release process`
    /// only from the rust CONTRIBUTING; a chosen license changes nothing.
    fn assert_license_headings(lang: &str, label: &str, readme: &str, contributing: &str) {
        let kept: &[&str] = if lang == "rust" {
            &["## Getting started", "## Development"]
        } else {
            &[
                "## Getting started",
                "## Development",
                "## Project structure",
            ]
        };
        for heading in kept {
            assert!(
                readme.contains(heading),
                "{lang}/{label} README must keep {heading}"
            );
        }
        if label == "MIT" {
            assert!(readme.contains("## License"), "{lang}/MIT README");
            assert!(
                contributing.contains("## Release process"),
                "{lang}/MIT CONTRIBUTING"
            );
            return;
        }
        assert!(!readme.contains("## License"), "{lang}/None README");
        assert!(
            !readme.contains("LICENSE"),
            "{lang}/None README mentions LICENSE"
        );
        if lang == "rust" {
            assert!(!contributing.contains("## Release process"));
            // Nothing about the dropped publish flow may survive: no
            // `release.yml` mention (the workflow does not exist), no
            // `CARGO_REGISTRY_TOKEN` story, no "automated releases" claim —
            // the CI paragraph keeps only its true ci.yml sentence.
            assert!(!contributing.contains("release.yml"));
            assert!(!contributing.contains("automated releases"));
            assert!(!contributing.contains("CARGO_REGISTRY_TOKEN"));
            for heading in [
                "## Development workflow",
                "## CI/CD and required secrets",
                "## Code style",
            ] {
                assert!(contributing.contains(heading), "rust/None keeps {heading}");
            }
            assert!(
                contributing.contains(
                    "This project uses GitHub Actions for CI (`.github/workflows/ci.yml`)."
                ),
                "rust/None CI paragraph must name only ci.yml"
            );
        } else {
            // python keeps its publish.yml story, so the section stays.
            assert!(contributing.contains("## Release process"));
        }
    }

    /// FR4 evidence — renders the REAL `ghscaff-boilerplate` trees (copied
    /// into a scratch dir under /tmp; the originals are untouched) with `MIT`
    /// and with no license, printing every `#` heading of README.md and
    /// CONTRIBUTING.md. Skips when the sibling checkout is absent. No network
    /// and no GitHub repository is created.
    #[test]
    fn evidence_real_boilerplate_docs_with_and_without_license() {
        let src = match dirs::home_dir() {
            Some(home) => home.join("Projects/UniverLab/ghscaff-boilerplate"),
            None => return,
        };
        if !src.join("rust").is_dir() || !src.join("python-module").is_dir() {
            eprintln!("[evidence-real] boilerplate checkout absent — skipped");
            return;
        }
        let scratch = tempfile::tempdir().unwrap();
        eprintln!("[evidence-real] scratch: {}", scratch.path().display());
        for lang in ["rust", "python-module"] {
            copy_tree(&src.join(lang), &scratch.path().join(lang));
            let tmpl = RemoteTemplate {
                cache_dir: scratch.path().join(lang),
            };
            for license in [Some("MIT"), None] {
                let label = license.unwrap_or("None");
                let files = tmpl.boilerplate_files("demo", "A demo", "univerlab", license);
                let readme = find(&files, "README.md");
                let contributing = find(&files, "CONTRIBUTING.md");
                eprintln!("[evidence-real] {lang}/{label} README.md:");
                print_headings("  ", readme);
                eprintln!("[evidence-real] {lang}/{label} CONTRIBUTING.md:");
                print_headings("  ", contributing);
                assert_license_headings(lang, label, readme, contributing);
            }
        }
    }
}
