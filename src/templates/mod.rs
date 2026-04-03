pub mod rust;

use anyhow::Result;

pub struct RepoFile {
    pub path: String,
    pub content: String,
    pub commit_message: String,
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

pub fn resolve(language: &str) -> Result<Box<dyn LanguageTemplate>> {
    match language {
        "rust" => Ok(Box::new(rust::RustTemplate)),
        other => anyhow::bail!("Unknown language template: {other}"),
    }
}

pub const AVAILABLE: &[&str] = &["rust"];
