#[cfg(test)]
use super::{LanguageTemplate, RepoFile};

#[cfg(test)]
pub struct RustTemplate;

#[cfg(test)]
impl RustTemplate {
    pub fn name(&self) -> &'static str {
        "rust"
    }
}

#[cfg(test)]
impl LanguageTemplate for RustTemplate {
    fn gitignore_name(&self) -> String {
        "Rust".into()
    }

    fn boilerplate_files(&self, repo_name: &str, description: &str, _owner: &str) -> Vec<RepoFile> {
        let cargo_toml = format!(
            "[package]\nname = \"{repo_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\ndescription = \"{description}\"\n\n[[bin]]\nname = \"{repo_name}\"\npath = \"src/main.rs\"\n\n[dependencies]\n"
        );
        let main_rs = "fn main() {\n    println!(\"Hello, world!\");\n}\n".to_string();
        let readme = format!("# {repo_name}\n\n{description}\n");

        vec![
            RepoFile {
                path: "Cargo.toml".into(),
                content: cargo_toml,
            },
            RepoFile {
                path: "src/main.rs".into(),
                content: main_rs,
            },
            RepoFile {
                path: "README.md".into(),
                content: readme,
            },
        ]
    }

    fn default_topics(&self) -> Vec<String> {
        vec!["rust".into(), "cli".into()]
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
    fn test_rust_template_boilerplate_files() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app", "myorg");
        assert_eq!(files.len(), 3, "Should generate 3 files");

        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Cargo.toml"));
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"README.md"));
    }

    #[test]
    fn test_rust_template_cargo_toml_has_name_placeholder() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app", "myorg");
        let cargo_toml = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        assert!(cargo_toml.content.contains("my-app"));
        assert!(cargo_toml.content.contains("A test app"));
    }

    #[test]
    fn test_rust_template_main_rs_is_valid() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app", "myorg");
        let main_rs = files.iter().find(|f| f.path == "src/main.rs").unwrap();
        assert!(main_rs.content.contains("fn main()"));
        assert!(main_rs.content.contains("println!"));
    }

    #[test]
    fn test_rust_template_readme_has_name() {
        let tmpl = RustTemplate;
        let files = tmpl.boilerplate_files("my-app", "A test app", "myorg");
        let readme = files.iter().find(|f| f.path == "README.md").unwrap();
        assert!(readme.content.contains("my-app"));
        assert!(readme.content.contains("A test app"));
    }

    #[test]
    fn test_rust_template_default_topics() {
        let tmpl = RustTemplate;
        let topics = tmpl.default_topics();
        assert_eq!(topics.len(), 2);
        assert!(topics.contains(&"rust".to_string()));
        assert!(topics.contains(&"cli".to_string()));
    }
}
