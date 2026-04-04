use super::{LanguageTemplate, RepoFile};

pub struct RustTemplate;

impl LanguageTemplate for RustTemplate {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn gitignore_name(&self) -> &'static str {
        "Rust"
    }

    fn ci_workflow(&self) -> &'static str {
        r#"name: CI
on:
  pull_request:
    branches: [main, develop]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo fmt --check
      - run: cargo clippy -- -D warnings
      - run: cargo test
"#
    }

    fn boilerplate_files(&self, repo_name: &str, description: &str) -> Vec<RepoFile> {
        let cargo_toml = format!(
            "[package]\nname = \"{repo_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\ndescription = \"{description}\"\n\n[[bin]]\nname = \"{repo_name}\"\npath = \"src/main.rs\"\n\n[dependencies]\n"
        );
        let main_rs = "fn main() {\n    println!(\"Hello, world!\");\n}\n".to_string();
        let readme = format!("# {repo_name}\n\n{description}\n");

        vec![
            RepoFile {
                path: "Cargo.toml".into(),
                content: cargo_toml,
                commit_message: "chore: init Cargo.toml".into(),
            },
            RepoFile {
                path: "src/main.rs".into(),
                content: main_rs,
                commit_message: "chore: init src/main.rs".into(),
            },
            RepoFile {
                path: "README.md".into(),
                content: readme,
                commit_message: "docs: init README".into(),
            },
        ]
    }

    fn default_topics(&self) -> Vec<&'static str> {
        vec!["rust", "cli"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_template_name() {
        let tmpl = RustTemplate;
        assert_eq!(tmpl.name(), "rust");
    }

    #[test]
    fn test_rust_template_gitignore_name() {
        let tmpl = RustTemplate;
        assert_eq!(tmpl.gitignore_name(), "Rust");
    }

    #[test]
    fn test_rust_template_ci_workflow_has_correct_name() {
        let tmpl = RustTemplate;
        let workflow = tmpl.ci_workflow();
        assert!(
            workflow.contains("name: CI"),
            "Workflow should have name: CI"
        );
        assert!(
            workflow.contains("branches: [main, develop]"),
            "Workflow should trigger on main and develop"
        );
    }

    #[test]
    fn test_rust_template_ci_workflow_has_required_steps() {
        let tmpl = RustTemplate;
        let workflow = tmpl.ci_workflow();
        assert!(workflow.contains("cargo fmt --check"));
        assert!(workflow.contains("cargo clippy -- -D warnings"));
        assert!(workflow.contains("cargo test"));
    }

    #[test]
    fn test_rust_template_boilerplate_files() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app");
        assert_eq!(files.len(), 3, "Should generate 3 files");

        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"README.md"));
    }

    #[test]
    fn test_rust_template_cargo_toml_has_name_placeholder() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app");
        let cargo_toml = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        assert!(cargo_toml.content.contains("my-app"));
        assert!(cargo_toml.content.contains("A test app"));
    }

    #[test]
    fn test_rust_template_main_rs_is_valid() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app");
        let main_rs = files.iter().find(|f| f.path == "src/main.rs").unwrap();
        assert!(main_rs.content.contains("fn main()"));
        assert!(main_rs.content.contains("println!"));
    }

    #[test]
    fn test_rust_template_readme_has_name() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app");
        let readme = files.iter().find(|f| f.path == "README.md").unwrap();
        assert!(readme.content.contains("my-app"));
        assert!(readme.content.contains("A test app"));
    }

    #[test]
    fn test_rust_template_default_topics() {
        let tmpl = RustTemplate;
        let topics = tmpl.default_topics();
        assert_eq!(topics.len(), 2);
        assert!(topics.contains(&"rust"));
        assert!(topics.contains(&"cli"));
    }

    #[test]
    fn test_rust_template_file_commit_messages() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("test", "description");
        for file in files {
            assert!(
                !file.commit_message.is_empty(),
                "Commit message should not be empty"
            );
            assert!(
                file.commit_message.contains(':'),
                "Commit message should follow conventional commits"
            );
        }
    }
}
