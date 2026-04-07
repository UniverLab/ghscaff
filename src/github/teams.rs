use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

#[derive(Deserialize, Clone, Debug)]
pub struct Team {
    pub name: String,
    pub slug: String,
    #[allow(dead_code)]
    pub description: Option<String>,
}

#[derive(Serialize)]
struct AddTeamBody {
    permission: String,
}

pub fn list_teams(client: &GithubClient) -> Result<Vec<Team>> {
    client.get("/user/teams")
}

pub fn add_team_to_repo(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    team_slug: &str,
    permission: &str,
) -> Result<()> {
    let body = AddTeamBody {
        permission: permission.to_string(),
    };
    client.put_no_response(&format!("/repos/{owner}/{repo}/teams/{team_slug}"), &body)
}

#[derive(Clone, Debug)]
pub struct TeamAccess {
    pub team_slug: String,
    pub permission: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_team_struct_creation() {
        let team = Team {
            name: "backend".to_string(),
            slug: "backend".to_string(),
            description: Some("Backend team".to_string()),
        };

        assert_eq!(team.name, "backend");
        assert_eq!(team.slug, "backend");
        assert_eq!(team.description, Some("Backend team".to_string()));
    }

    #[test]
    fn test_team_struct_no_description() {
        let team = Team {
            name: "devops".to_string(),
            slug: "devops".to_string(),
            description: None,
        };

        assert_eq!(team.name, "devops");
        assert_eq!(team.slug, "devops");
        assert!(team.description.is_none());
    }

    #[test]
    fn test_team_access_struct() {
        let access = TeamAccess {
            team_slug: "backend".to_string(),
            permission: "push".to_string(),
        };

        assert_eq!(access.team_slug, "backend");
        assert_eq!(access.permission, "push");
    }

    #[test]
    fn test_team_access_all_permission_types() {
        let permissions = vec!["pull", "triage", "push", "admin"];

        for permission in permissions {
            let access = TeamAccess {
                team_slug: "test-team".to_string(),
                permission: permission.to_string(),
            };
            assert_eq!(access.permission, permission);
        }
    }

    #[test]
    fn test_add_team_body_serialization() {
        let body = AddTeamBody {
            permission: "push".to_string(),
        };

        let json = serde_json::to_string(&body).expect("Failed to serialize");
        assert!(json.contains("push"));
        assert!(json.contains("permission"));
    }

    #[test]
    fn test_team_clone() {
        let team = Team {
            name: "backend".to_string(),
            slug: "backend".to_string(),
            description: Some("Backend team".to_string()),
        };

        let cloned = team.clone();
        assert_eq!(team.name, cloned.name);
        assert_eq!(team.slug, cloned.slug);
        assert_eq!(team.description, cloned.description);
    }

    #[test]
    fn test_team_access_clone() {
        let access = TeamAccess {
            team_slug: "backend".to_string(),
            permission: "push".to_string(),
        };

        let cloned = access.clone();
        assert_eq!(access.team_slug, cloned.team_slug);
        assert_eq!(access.permission, cloned.permission);
    }
}
