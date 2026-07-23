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
    client.put_no_response(
        &format!("/orgs/{owner}/teams/{team_slug}/repos/{owner}/{repo}"),
        &body,
    )
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

    #[test]
    fn test_team_deserialize() {
        let json = r#"{"name":"frontend","slug":"frontend","description":"Frontend team"}"#;
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "frontend");
        assert_eq!(team.slug, "frontend");
        assert_eq!(team.description.as_deref(), Some("Frontend team"));
    }

    #[test]
    fn test_team_deserialize_no_description() {
        let json = r#"{"name":"devops","slug":"devops"}"#;
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "devops");
        assert!(team.description.is_none());
    }

    #[test]
    fn test_team_deserialize_real_github() {
        let json = r#"{
            "id": 1,
            "node_id": "MDQ6VGVhbTE=",
            "url": "https://api.github.com/orgs/octocat/teams/frontend",
            "name": "Frontend Team",
            "slug": "frontend",
            "description": "Frontend developers",
            "privacy": "closed",
            "permission": "admin",
            "members_url": "https://api.github.com/orgs/octocat/teams/frontend/members{/member}",
            "repositories_url": "https://api.github.com/orgs/octocat/teams/frontend/repos",
            "parent": null
        }"#;
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "Frontend Team");
        assert_eq!(team.slug, "frontend");
        assert_eq!(team.description.as_deref(), Some("Frontend developers"));
    }

    #[test]
    fn test_add_team_body_all_permissions() {
        for perm in &["pull", "triage", "push", "admin"] {
            let body = AddTeamBody {
                permission: perm.to_string(),
            };
            let json = serde_json::to_string(&body).unwrap();
            assert!(json.contains(&format!("\"permission\":\"{}\"", perm)));
        }
    }

    #[test]
    fn test_team_access_debug() {
        let access = TeamAccess {
            team_slug: "my-team".to_string(),
            permission: "admin".to_string(),
        };
        let dbg = format!("{:?}", access);
        assert!(dbg.contains("TeamAccess"));
        assert!(dbg.contains("my-team"));
    }

    #[test]
    fn test_team_debug() {
        let team = Team {
            name: "test".to_string(),
            slug: "test".to_string(),
            description: None,
        };
        let dbg = format!("{:?}", team);
        assert!(dbg.contains("Team"));
        assert!(dbg.contains("test"));
    }

    #[test]
    fn test_team_access_empty_strings() {
        let access = TeamAccess {
            team_slug: String::new(),
            permission: String::new(),
        };
        assert!(access.team_slug.is_empty());
        assert!(access.permission.is_empty());
    }

    #[test]
    fn test_team_empty_name() {
        let team = Team {
            name: String::new(),
            slug: String::new(),
            description: None,
        };
        assert!(team.name.is_empty());
        assert!(team.slug.is_empty());
    }

    #[test]
    fn test_list_teams_signature() {
        let _: fn(&GithubClient) -> Result<Vec<Team>> = list_teams;
    }

    #[test]
    fn test_add_team_to_repo_signature() {
        let _: fn(&GithubClient, &str, &str, &str, &str) -> Result<()> = add_team_to_repo;
    }

    #[test]
    fn test_team_deserialize_with_parent() {
        let json = r#"{
            "name": "frontend",
            "slug": "frontend",
            "description": "Frontend team",
            "parent": null
        }"#;
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "frontend");
        assert_eq!(team.slug, "frontend");
    }

    #[test]
    fn test_team_access_all_permission_types_debug() {
        for perm in &["pull", "triage", "push", "admin"] {
            let access = TeamAccess {
                team_slug: "team".to_string(),
                permission: perm.to_string(),
            };
            let dbg = format!("{:?}", access);
            assert!(dbg.contains(perm));
        }
    }

    #[test]
    fn test_add_team_body_serialization_admin() {
        let body = AddTeamBody {
            permission: "admin".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"admin\""));
    }

    #[test]
    fn test_team_access_clone_preserves_fields() {
        let access = TeamAccess {
            team_slug: "my-team".to_string(),
            permission: "triage".to_string(),
        };
        let cloned = access.clone();
        assert_eq!(access.team_slug, cloned.team_slug);
        assert_eq!(access.permission, cloned.permission);
    }

    #[test]
    fn test_team_clone_preserves_all() {
        let team = Team {
            name: "design".to_string(),
            slug: "design".to_string(),
            description: Some("Design team".to_string()),
        };
        let cloned = team.clone();
        assert_eq!(team.name, cloned.name);
        assert_eq!(team.slug, cloned.slug);
        assert_eq!(team.description, cloned.description);
    }

    #[test]
    fn test_team_deserialize_unicode() {
        let json = r#"{"name":"開発チーム","slug":"dev-team","description":"Development"}"#;
        let team: Team = serde_json::from_str(json).unwrap();
        assert_eq!(team.name, "開発チーム");
    }
}
