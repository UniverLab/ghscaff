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
