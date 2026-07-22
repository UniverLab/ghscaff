use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

#[derive(Serialize, Deserialize, Clone, Debug)]
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

pub fn delete_label(client: &GithubClient, owner: &str, repo: &str, name: &str) -> Result<()> {
    let encoded = urlencoding::encode(name);
    client.delete(&format!("/repos/{owner}/{repo}/labels/{encoded}"))
}

pub fn standard_labels() -> Vec<Label> {
    vec![
        Label {
            name: "bug".into(),
            color: "d73a4a".into(),
            description: "Something isn't working".into(),
        },
        Label {
            name: "feature".into(),
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
            name: "target:main".into(),
            color: "1d76db".into(),
            description: "Targets the main branch".into(),
        },
        Label {
            name: "target:develop".into(),
            color: "0e8a16".into(),
            description: "Targets the develop branch".into(),
        },
        Label {
            name: "help wanted".into(),
            color: "008672".into(),
            description: "Extra attention needed".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_labels_count() {
        let labels = standard_labels();
        assert_eq!(labels.len(), 7, "Should have exactly 7 standard labels");
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

    #[test]
    fn test_target_labels_exist() {
        let labels = standard_labels();
        assert!(labels.iter().any(|l| l.name == "target:main"));
        assert!(labels.iter().any(|l| l.name == "target:develop"));
    }

    #[test]
    fn test_standard_labels_names() {
        let labels = standard_labels();
        let names: Vec<&str> = labels.iter().map(|l| l.name.as_str()).collect();
        assert!(names.contains(&"bug"));
        assert!(names.contains(&"feature"));
        assert!(names.contains(&"documentation"));
        assert!(names.contains(&"breaking-change"));
        assert!(names.contains(&"target:main"));
        assert!(names.contains(&"target:develop"));
        assert!(names.contains(&"help wanted"));
    }

    #[test]
    fn test_label_deserialize_minimal() {
        let json = r#"{"name":"x","color":"ff0000","description":"X label"}"#;
        let label: Label = serde_json::from_str(json).unwrap();
        assert_eq!(label.name, "x");
        assert_eq!(label.color, "ff0000");
        assert_eq!(label.description, "X label");
    }

    #[test]
    fn test_label_deserialize_real_github() {
        let json = r#"{
            "id": 1,
            "node_id": "MDU6TGFiZWwx",
            "url": "https://api.github.com/repos/owner/repo/labels/bug",
            "name": "bug",
            "description": "Something isn't working",
            "color": "d73a4a",
            "default": true
        }"#;
        let label: Label = serde_json::from_str(json).unwrap();
        assert_eq!(label.name, "bug");
        assert_eq!(label.color, "d73a4a");
    }

    #[test]
    fn test_label_clone() {
        let label = Label {
            name: "clone-test".into(),
            color: "abcdef".into(),
            description: "Clone me".into(),
        };
        let cloned = label.clone();
        assert_eq!(label.name, cloned.name);
        assert_eq!(label.color, cloned.color);
        assert_eq!(label.description, cloned.description);
    }

    #[test]
    fn test_label_debug() {
        let label = Label {
            name: "debug-test".into(),
            color: "123456".into(),
            description: "Debug test".into(),
        };
        let dbg = format!("{:?}", label);
        assert!(dbg.contains("Label"));
        assert!(dbg.contains("debug-test"));
    }

    #[test]
    fn test_standard_labels_colors_are_lowercase_hex() {
        let labels = standard_labels();
        for label in &labels {
            assert!(
                label.color.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "Color {} should be lowercase hex digits only",
                label.color
            );
        }
    }

    #[test]
    fn test_feature_label_exists() {
        let labels = standard_labels();
        let feature = labels.iter().find(|l| l.name == "feature");
        assert!(feature.is_some());
        assert_eq!(feature.unwrap().color, "a2eeef");
    }

    #[test]
    fn test_documentation_label_exists() {
        let labels = standard_labels();
        let doc = labels.iter().find(|l| l.name == "documentation");
        assert!(doc.is_some());
        assert_eq!(doc.unwrap().color, "0075ca");
    }

    #[test]
    fn test_breaking_change_label_exists() {
        let labels = standard_labels();
        let bc = labels.iter().find(|l| l.name == "breaking-change");
        assert!(bc.is_some());
        assert_eq!(bc.unwrap().color, "e4e669");
    }

    #[test]
    fn test_help_wanted_label_exists() {
        let labels = standard_labels();
        let hw = labels.iter().find(|l| l.name == "help wanted");
        assert!(hw.is_some());
        assert_eq!(hw.unwrap().color, "008672");
    }

    #[test]
    fn test_standard_labels_sorted_by_category() {
        let labels = standard_labels();
        // Verify labels are in expected order (as returned by the function)
        assert_eq!(labels[0].name, "bug");
        assert_eq!(labels[1].name, "feature");
        assert_eq!(labels[2].name, "documentation");
        assert_eq!(labels[3].name, "breaking-change");
        assert_eq!(labels[4].name, "target:main");
        assert_eq!(labels[5].name, "target:develop");
        assert_eq!(labels[6].name, "help wanted");
    }

    #[test]
    fn test_label_color_not_empty() {
        let labels = standard_labels();
        for label in &labels {
            assert!(!label.color.is_empty(), "Color should not be empty");
        }
    }

    #[test]
    fn test_label_name_not_empty() {
        let labels = standard_labels();
        for label in &labels {
            assert!(!label.name.is_empty(), "Name should not be empty");
        }
    }
}
