use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

#[derive(Serialize, Deserialize, Clone)]
pub struct Label {
    pub name: String,
    pub color: String,
    pub description: String,
}

pub fn list_labels(client: &GithubClient, owner: &str, repo: &str) -> Result<Vec<Label>> {
    client.get(&format!("/repos/{owner}/{repo}/labels?per_page=100"))
}

pub fn create_label(client: &GithubClient, owner: &str, repo: &str, label: &Label) -> Result<()> {
    let _: serde_json::Value = client.post(&format!("/repos/{owner}/{repo}/labels"), label)?;
    Ok(())
}

pub fn update_label(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    name: &str,
    label: &Label,
) -> Result<()> {
    let _: serde_json::Value =
        client.patch(&format!("/repos/{owner}/{repo}/labels/{name}"), label)?;
    Ok(())
}

pub fn standard_labels() -> Vec<Label> {
    vec![
        Label {
            name: "bug".into(),
            color: "d73a4a".into(),
            description: "Something isn't working".into(),
        },
        Label {
            name: "enhancement".into(),
            color: "a2eeef".into(),
            description: "New feature or request".into(),
        },
        Label {
            name: "documentation".into(),
            color: "0075ca".into(),
            description: "Improvements to docs".into(),
        },
        Label {
            name: "breaking-change".into(),
            color: "e4e669".into(),
            description: "Introduces breaking changes".into(),
        },
        Label {
            name: "good first issue".into(),
            color: "7057ff".into(),
            description: "Good for newcomers".into(),
        },
        Label {
            name: "help wanted".into(),
            color: "008672".into(),
            description: "Extra attention needed".into(),
        },
        Label {
            name: "wontfix".into(),
            color: "ffffff".into(),
            description: "This will not be worked on".into(),
        },
        Label {
            name: "duplicate".into(),
            color: "cfd3d7".into(),
            description: "This issue already exists".into(),
        },
        Label {
            name: "question".into(),
            color: "d876e3".into(),
            description: "Further information requested".into(),
        },
        Label {
            name: "dependencies".into(),
            color: "0366d6".into(),
            description: "Dependency updates".into(),
        },
        Label {
            name: "ci/cd".into(),
            color: "f9d0c4".into(),
            description: "CI/CD related changes".into(),
        },
        Label {
            name: "refactor".into(),
            color: "e99695".into(),
            description: "Code refactor, no behavior change".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_labels_count() {
        let labels = standard_labels();
        assert_eq!(labels.len(), 12, "Should have exactly 12 standard labels");
    }

    #[test]
    fn test_standard_labels_have_unique_names() {
        let labels = standard_labels();
        let mut names: Vec<_> = labels.iter().map(|l| &l.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(
            names.len(),
            labels.len(),
            "All label names should be unique"
        );
    }

    #[test]
    fn test_standard_labels_have_valid_colors() {
        let labels = standard_labels();
        for label in labels {
            assert_eq!(
                label.color.len(),
                6,
                "Color {} should be 6 hex characters",
                label.color
            );
            assert!(
                label.color.chars().all(|c| c.is_ascii_hexdigit()),
                "Color {} should contain only hex digits",
                label.color
            );
        }
    }

    #[test]
    fn test_standard_labels_have_descriptions() {
        let labels = standard_labels();
        for label in labels {
            assert!(
                !label.description.is_empty(),
                "Label {} should have a description",
                label.name
            );
        }
    }

    #[test]
    fn test_bug_label_exists() {
        let labels = standard_labels();
        let bug = labels.iter().find(|l| l.name == "bug");
        assert!(bug.is_some(), "Bug label should exist");
        assert_eq!(bug.unwrap().color, "d73a4a");
    }

    #[test]
    fn test_label_serialization() {
        let label = Label {
            name: "test".into(),
            color: "000000".into(),
            description: "Test label".into(),
        };
        let json = serde_json::to_string(&label).unwrap();
        let deserialized: Label = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, label.name);
        assert_eq!(deserialized.color, label.color);
        assert_eq!(deserialized.description, label.description);
    }
}
