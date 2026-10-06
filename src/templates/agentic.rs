//! The `.gitignore` block for AI coding agents, derived from the canopy
//! registry rather than from a hardcoded list, so a new harness (opencode,
//! codebuddy, devin, kilo, antigravity…) reaches every scaffolded repo as
//! soon as the registry lists it.
//!
//! Ported from gitkit's `src/ignore/agentic.rs` minus the merge half: ghscaff
//! always writes a fresh `.gitignore`, so nothing here strips an old block.
//! The header line is gitkit's own, so a later `gitkit ignore add agentic`
//! still recognises this block.

use std::collections::BTreeSet;

#[cfg(test)]
use super::agent_registry::snapshot_platforms;
use super::agent_registry::Platform;
#[cfg(not(test))]
use super::agent_registry::{cache_path, resolve, HttpFetcher, Resolution};

/// The header `assemble()` emits, byte for byte the one
/// `gitkit ignore add agentic` writes. Its presence is what marks the block.
pub(crate) const CANONICAL_HEADER: &str =
    "# AI coding agents (paths derived from the canopy registry)";
/// Marks the negation tail of the block; nothing but `!` lines may follow it.
pub(crate) const NEG_HEADER: &str = "# instruction files stay committable";

/// The complete agentic block for a new `.gitignore`. Never fails: when the
/// registry is unreachable the cached or embedded registry is used and one
/// warning line names the source.
pub(crate) fn block() -> String {
    let platforms = resolved_platforms();
    let (lines, warnings) = build(&platforms);
    for warning in warnings {
        eprintln!("{warning}");
    }
    assemble(&lines)
}

#[cfg(not(test))]
fn resolved_platforms() -> Vec<Platform> {
    let cache = cache_path();
    let Resolution { platforms, notice } = resolve(&HttpFetcher, cache.as_deref());
    if let Some(notice) = notice {
        eprintln!("{notice}");
    }
    platforms
}

/// Tests always build from the embedded snapshot: no network, and the
/// assertions below pin exact output.
#[cfg(test)]
fn resolved_platforms() -> Vec<Platform> {
    snapshot_platforms()
}

/// The registry-derived lines of an `agentic` block: ignore lines first, then
/// the negation lines, split the way the block is written.
pub(crate) fn build(platforms: &[Platform]) -> (Vec<String>, Vec<String>) {
    let protected = protected_set(platforms);
    let (mut ignore, warnings) = ignore_lines(platforms, &protected);
    // Registry-independent: every agent may write this lockfile.
    ignore.insert("skills-lock.json".to_string());
    let dirs: BTreeSet<String> = ignore
        .iter()
        .filter(|line| line.ends_with("/*"))
        .map(|line| line.trim_end_matches('*').to_string())
        .collect();
    let mut lines: Vec<String> = ignore.into_iter().collect();
    lines.extend(negations(&protected, &dirs));
    (lines, warnings)
}

#[cfg(test)]
pub(crate) fn render(platforms: &[Platform]) -> String {
    let (lines, _) = build(platforms);
    assemble(&lines)
}

/// Canonical header, the lines, and — as soon as the first negation appears —
/// a blank line, [`NEG_HEADER`] and the negations. No leading blank line: the
/// block is appended by the caller, which owns the separator.
fn assemble(lines: &[String]) -> String {
    let mut content = String::from(CANONICAL_HEADER);
    content.push('\n');
    let mut in_negations = false;
    for line in lines {
        if !in_negations && line.starts_with('!') {
            content.push('\n');
            content.push_str(NEG_HEADER);
            content.push('\n');
            in_negations = true;
        }
        content.push_str(line);
        content.push('\n');
    }
    content
}

/// Instruction files that stay committable: the two cross-agent names, each
/// platform's own instruction file, and every precedence entry that is not
/// also an ignored project path.
fn protected_set(platforms: &[Platform]) -> BTreeSet<String> {
    let project_paths: BTreeSet<&str> = platforms
        .iter()
        .flat_map(|platform| platform.project_paths.iter().map(String::as_str))
        .collect();
    let mut protected = BTreeSet::new();
    protected.insert("AGENTS.md".to_string());
    protected.insert("CLAUDE.md".to_string());
    for platform in platforms {
        if let Some(file) = &platform.instruction_file {
            protected.insert(file.clone());
        }
        for entry in &platform.instruction_precedence {
            if !project_paths.contains(entry.as_str()) {
                protected.insert(entry.clone());
            }
        }
    }
    protected
}

/// Never-ignore roots: `.github` and `.config` hold configuration that must
/// stay visible, neither the bare root nor anything under it.
fn is_forbidden(path: &str) -> bool {
    path == ".github"
        || path == ".config"
        || path.starts_with(".github/")
        || path.starts_with(".config/")
}

fn ignore_lines(
    platforms: &[Platform],
    protected: &BTreeSet<String>,
) -> (BTreeSet<String>, Vec<String>) {
    let mut lines = BTreeSet::new();
    let mut warnings = Vec::new();
    for platform in platforms {
        for entry in &platform.project_paths {
            if is_forbidden(entry) {
                warnings.push(forbidden_warning(&platform.name, entry));
                continue;
            }
            if entry.ends_with('/') {
                // The directory itself stays trackable; its contents do not.
                lines.insert(format!("{entry}*"));
            } else if protected.contains(entry) {
                warnings.push(protected_warning(&platform.name, entry));
            } else {
                lines.insert(entry.clone());
            }
        }
    }
    (lines, warnings)
}

fn forbidden_warning(platform: &str, entry: &str) -> String {
    format!("agentic: {platform}: '{entry}' skipped — .github/.config are never ignored")
}

fn protected_warning(platform: &str, entry: &str) -> String {
    format!("agentic: {platform}: '{entry}' is a protected instruction file and stays committable")
}

/// One negation per protected file that lives under an ignored directory, plus
/// the negation of every directory between it and that ignored root: git must
/// be walked back down to the file, which `!file` alone cannot do.
fn negations(protected: &BTreeSet<String>, dirs: &BTreeSet<String>) -> Vec<String> {
    let mut out = BTreeSet::new();
    for path in protected {
        if !path.contains('/') {
            continue;
        }
        let Some(ignored) = dirs.iter().find(|dir| path.starts_with(dir.as_str())) else {
            continue;
        };
        for item in ancestor_chain(path, &ancestor_dir(path), ignored) {
            out.insert(format_negation(&item));
        }
    }
    out.into_iter().collect()
}

/// `path` plus its ancestors, walking up until `ignored` is reached.
///
/// The bound is `path.len()` steps, which the real chain never approaches
/// (every step shortens `current` and runs out first); it only stops a
/// non-productive [`ancestor_dir`] from spinning here.
fn ancestor_chain(path: &str, parent: &str, ignored: &str) -> Vec<String> {
    let mut current = parent.to_string();
    let mut chain = vec![path.to_string()];
    for _ in 0..=path.len() {
        if current == ignored {
            break;
        }
        chain.push(current.clone());
        current = ancestor_dir(&current);
    }
    chain
}

fn ancestor_dir(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(index) => trimmed[..=index].to_string(),
        None => String::new(),
    }
}

/// Files are negated as-is; a directory gets a trailing `/` so git treats the
/// negation as "this directory", not as a no-op file name.
fn format_negation(path: &str) -> String {
    if path.ends_with('/') {
        return format!("!{path}");
    }
    let last = path.rsplit('/').next().unwrap_or(path);
    if last.contains('.') {
        format!("!{path}")
    } else {
        format!("!{path}/")
    }
}

#[cfg(test)]
mod tests {
    use super::super::agent_registry::{parse_merged, snapshot_platforms, SNAPSHOT};
    use super::*;

    const FOO: &str = "[[platforms]]\nname = \"foo\"\nproject_paths = [\".foo/\"]\n";

    fn platforms_of(text: &str) -> Vec<Platform> {
        parse_merged(text).unwrap()
    }

    #[test]
    fn build_fixture_new_platform_foo() {
        let (lines, _) = build(&platforms_of(FOO));
        assert!(lines.contains(&".foo/*".to_string()));
    }

    #[test]
    fn build_fixture_github_path_warns_only() {
        let text = "[[platforms]]\nname = \"evil\"\nproject_paths = [\".github/prompts/\"]\n";
        let (lines, warnings) = build(&platforms_of(text));
        assert!(!lines.iter().any(|line| {
            let body = line.trim_start_matches('!');
            body == ".github" || body.starts_with(".github/") || body.starts_with(".github*")
        }));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("evil"));
        for entry in [
            ".github",
            ".config",
            ".github/",
            ".config/",
            ".config/x/",
            ".github/y",
        ] {
            let fixture = format!("[[platforms]]\nname = \"p\"\nproject_paths = [\"{entry}\"]\n");
            let (lines, warnings) = build(&platforms_of(&fixture));
            assert!(!lines.iter().any(|line| {
                let body = line.trim_start_matches('!');
                body.starts_with(".github") || body.starts_with(".config")
            }));
            assert_eq!(warnings.len(), 1, "{entry} must warn exactly once");
        }
    }

    #[test]
    fn build_fixture_protected_root_file_dropped_with_warning() {
        let text = "[[platforms]]\nname = \"p\"\nproject_paths = [\"AGENTS.md\"]\ninstruction_file = \"AGENTS.md\"\n";
        let (lines, warnings) = build(&platforms_of(text));
        assert!(!lines.contains(&"AGENTS.md".to_string()));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains('p'));
    }

    #[test]
    fn protected_set_rules() {
        let text = "[[platforms]]\nname = \"claude\"\nproject_paths = [\".claude/\", \"CLAUDE.local.md\"]\n\
            instruction_file = \"AGENTS.md\"\n\
            instruction_precedence = [\"CLAUDE.md\", \".claude/CLAUDE.md\", \"CLAUDE.local.md\"]\n";
        let protected = protected_set(&platforms_of(text));
        assert!(protected.contains("AGENTS.md"));
        assert!(protected.contains("CLAUDE.md"));
        assert!(protected.contains(".claude/CLAUDE.md"));
        assert!(!protected.contains("CLAUDE.local.md"));
    }

    /// `skills-lock.json` is not in any platform's paths, so a registry
    /// refresh that drops every platform would still keep it.
    #[test]
    fn build_always_includes_skills_lock() {
        let (_, warnings) = build(&platforms_of(
            "[[platforms]]\nname = \"p\"\nproject_paths = []\n",
        ));
        let (lines, _) = build(&platforms_of(
            "[[platforms]]\nname = \"p\"\nproject_paths = []\n",
        ));
        assert!(lines.contains(&"skills-lock.json".to_string()));
        assert!(warnings.is_empty());
    }

    #[test]
    fn render_snapshot_expected_lines() {
        let content = render(&snapshot_platforms());
        for expected in [
            ".kilo/*",
            "kilo.jsonc",
            "kilo.json",
            ".kilocode/*",
            ".opencode/*",
            "opencode.json",
            "opencode.jsonc",
            ".agents/*",
            ".claude/*",
            ".codebuddy/*",
            ".devin/*",
            "skills-lock.json",
            ".mcp.json",
            ".clineignore",
            "!.claude/CLAUDE.md",
            "!.continue/rules/",
            "!.cursor/rules/",
        ] {
            assert!(
                content.lines().any(|line| line == expected),
                "missing {expected}"
            );
        }
        assert_eq!(
            content.lines().filter(|line| *line == ".agents/*").count(),
            1,
            ".agents/* must be deduplicated"
        );
        assert!(!content.lines().any(|line| line == ".aider.chat.history.md"));
        assert!(!content
            .lines()
            .any(|line| line == ".claude/settings.local.json"));
        assert!(!content.lines().any(|line| line.trim() == ".claude/"));
        assert!(!content.lines().any(|line| line.trim() == ".windsurf/"));
        assert!(!content.lines().any(|line| line.trim() == ".zencoder/"));
        for line in content.lines() {
            if line.starts_with('!') || line.is_empty() || line.starts_with('#') {
                continue;
            }
            assert_ne!(line, "AGENTS.md");
            assert_ne!(line, "CLAUDE.md");
        }
    }

    #[test]
    fn render_negation_block_last() {
        let content = render(&snapshot_platforms());
        let mut seen_negation = false;
        for line in content.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('!') {
                seen_negation = true;
            } else if seen_negation {
                panic!("ignore line after negation block: {line}");
            }
        }
    }

    #[test]
    fn render_never_emits_github_or_config() {
        let (lines, _) = build(&snapshot_platforms());
        for line in &lines {
            let body = line.trim_start_matches('!');
            assert!(!is_forbidden(body), "forbidden line emitted: {line}");
        }
    }

    #[test]
    fn render_contains_only_expected_negations() {
        let (lines, _) = build(&snapshot_platforms());
        let negs: Vec<&String> = lines.iter().filter(|line| line.starts_with('!')).collect();
        assert_eq!(negs.len(), 3);
        assert!(negs
            .iter()
            .any(|line| line.as_str() == "!.claude/CLAUDE.md"));
        assert!(negs.iter().any(|line| line.as_str() == "!.continue/rules/"));
        assert!(negs.iter().any(|line| line.as_str() == "!.cursor/rules/"));
    }

    /// The whole point of the port: from the embedded snapshot ghscaff
    /// assembles the exact bytes `gitkit ignore add agentic` writes.
    #[test]
    fn block_matches_the_gitkit_snapshot_fixture() {
        assert_eq!(
            block(),
            include_str!("../../tests/fixtures/agentic-block.txt")
        );
    }

    #[test]
    fn block_contains_negation_section_and_no_forbidden_root() {
        let content = block();
        let lines: Vec<&str> = content.lines().collect();
        let header = lines.iter().position(|line| *line == CANONICAL_HEADER);
        assert_eq!(header, Some(0), "the block opens with gitkit's header");
        let neg = lines
            .iter()
            .position(|line| *line == NEG_HEADER)
            .expect("the negation section is part of the block");
        assert!(lines[neg + 1..].iter().all(|line| line.starts_with('!')));
        for line in &lines {
            let body = line.trim_start_matches('!');
            assert!(
                !body.starts_with(".github") && !body.starts_with(".config"),
                ".github/.config must never be ignored: {line}"
            );
        }
        assert!(!lines.iter().any(|line| line.trim() == "AGENTS.md"));
        assert!(!lines.iter().any(|line| line.trim() == "CLAUDE.md"));
    }

    /// The comment that marks the negation block is emitted exactly once, on
    /// the line before the first `!` entry — not before the first ignore line
    /// and not dropped when the input already starts with a negation.
    #[test]
    fn assemble_comment_marks_the_negation_block_in_place() {
        let content = assemble(&[
            ".foo/*".to_string(),
            "bar.json".to_string(),
            "!keep/one.md".to_string(),
            "!keep/two.md".to_string(),
        ]);
        let lines: Vec<&str> = content.lines().collect();
        let hits: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| **line == NEG_HEADER)
            .map(|(index, _)| index)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "the marker must appear exactly once: {content:?}"
        );
        let at = hits[0];
        assert!(
            lines.get(at + 1).is_some_and(|next| next.starts_with('!')),
            "the line after the marker must be the first negation: {content:?}"
        );
        assert!(
            lines[at + 1..].iter().all(|line| line.starts_with('!')),
            "nothing but negations may follow the marker: {content:?}"
        );
        assert!(
            lines[..at].contains(&".foo/*"),
            "ignore lines must precede the marker: {content:?}"
        );
    }

    /// An input that opens with a negation still gets the marker, once, and
    /// never an ignore line after it.
    #[test]
    fn assemble_comment_when_the_first_line_is_a_negation() {
        let content = assemble(&["!keep/one.md".to_string()]);
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.iter().filter(|line| **line == NEG_HEADER).count(), 1);
        assert!(
            lines.contains(&"!keep/one.md"),
            "the negation itself must be kept: {content:?}"
        );
        assert!(
            lines
                .iter()
                .skip_while(|line| **line != NEG_HEADER)
                .skip(1)
                .all(|line| line.starts_with('!')),
            "nothing but negations may follow the marker: {content:?}"
        );
    }

    /// The block is appended by the caller, which owns the separator line, so
    /// the block itself must not start with a blank line.
    #[test]
    fn assemble_has_no_leading_blank_line() {
        let content = assemble(&[".foo/*".to_string()]);
        assert!(content.starts_with(CANONICAL_HEADER), "{content:?}");
        assert!(content.ends_with('\n'));
    }

    /// `parse_merged` on the embedded snapshot is what every snapshot test
    /// above feeds through `build`; keep it in view so a snapshot change is
    /// obvious here.
    #[test]
    fn snapshot_is_the_source_of_the_rendered_block() {
        assert_eq!(render(&snapshot_platforms()), block());
        assert_eq!(parse_merged(SNAPSHOT).unwrap().len(), 21);
    }
}

#[cfg(test)]
mod check_ignore_tests {
    use super::super::agent_registry::{parse_merged, snapshot_platforms, SNAPSHOT};
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::process::Command;

    fn is_ignored(repo: &Path, path: &str) -> bool {
        let output = Command::new("git")
            .args(["check-ignore", "--no-index", "--", path])
            .current_dir(repo)
            .output()
            .expect("git binary must be available");
        match output.status.code() {
            Some(0) => true,
            Some(1) => false,
            other => panic!(
                "git check-ignore failed ({other:?}): {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        }
    }

    fn create_probe(repo: &Path, path: &str) {
        let full = repo.join(path);
        if path.ends_with('/') {
            std::fs::create_dir_all(&full).unwrap();
            std::fs::write(full.join("probe.txt"), "x").unwrap();
        } else if let Some(parent) = full.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, "x").unwrap();
        }
    }

    /// The block is only useful if git honours it: every registry path is
    /// ignored, `.github` and the instruction files are not. Local `git`, no
    /// network, no repository is created on GitHub.
    #[test]
    fn generated_block_via_git_check_ignore_is_behaved_exactly() {
        let platforms = snapshot_platforms();
        let content = render(&platforms);
        let dir = tempfile::TempDir::new().unwrap();
        let repo = dir.path();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(repo)
            .status()
            .unwrap()
            .success());
        std::fs::write(repo.join(".gitignore"), &content).unwrap();

        let protected = protected_set(&platforms);
        let mut project_entries: BTreeSet<String> = BTreeSet::new();
        for platform in &platforms {
            for entry in &platform.project_paths {
                project_entries.insert(entry.clone());
            }
        }
        for entry in &project_entries {
            if is_forbidden(entry) || protected.contains(entry) {
                continue;
            }
            if entry.ends_with('/') {
                create_probe(repo, entry);
                assert!(
                    is_ignored(repo, &format!("{entry}probe.txt")),
                    "{entry}probe.txt should be ignored"
                );
            } else {
                create_probe(repo, entry);
                assert!(is_ignored(repo, entry), "{entry} should be ignored");
            }
        }

        for path in &protected {
            let last = path.rsplit('/').next().unwrap_or(path);
            let dir_like = path.ends_with('/') || (path.contains('/') && !last.contains('.'));
            if dir_like {
                let dir = if path.ends_with('/') {
                    path.clone()
                } else {
                    format!("{path}/")
                };
                create_probe(repo, &dir);
                assert!(
                    !is_ignored(repo, &format!("{dir}probe.mdc")),
                    "{path} should stay committable"
                );
            } else {
                create_probe(repo, path);
                assert!(!is_ignored(repo, path), "{path} should stay committable");
            }
        }
        create_probe(repo, ".github/copilot-instructions.md");
        assert!(!is_ignored(repo, ".github/copilot-instructions.md"));

        for ignored in [
            "CLAUDE.local.md",
            "skills-lock.json",
            ".mcp.json",
            ".clineignore",
            "kilo.jsonc",
            "opencode.json",
        ] {
            create_probe(repo, ignored);
            assert!(is_ignored(repo, ignored), "{ignored} should be ignored");
        }
        for kept in [
            "AGENTS.md",
            "CLAUDE.md",
            "GEMINI.md",
            "knowledge.md",
            ".clinerules",
            "AGENTS.override.md",
        ] {
            create_probe(repo, kept);
            assert!(!is_ignored(repo, kept), "{kept} should stay committable");
        }
    }

    /// The platform list the assertions above walk is the embedded snapshot,
    /// not a test fixture that could drift from it.
    #[test]
    fn check_ignore_fixture_is_the_embedded_snapshot() {
        assert_eq!(parse_merged(SNAPSHOT).unwrap(), snapshot_platforms());
    }
}
