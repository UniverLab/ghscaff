//! GitHub Sponsors capability: enable the Sponsor button on a repository.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

const GRAPHQL_PATH: &str = "/graphql";

const REPOSITORY_LOOKUP_QUERY: &str = "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id hasSponsorshipsEnabled } }";

const ENABLE_SPONSORSHIPS_MUTATION: &str = "mutation($repositoryId: ID!) { updateRepository(input: { repositoryId: $repositoryId, hasSponsorshipsEnabled: true }) { repository { hasSponsorshipsEnabled } } }";

/// Outcome of a successful [`enable_sponsorships`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SponsorshipStatus {
    /// The Sponsor button was off and this call turned it on.
    Enabled,
    /// The Sponsor button was already on; nothing was changed.
    AlreadyEnabled,
}

#[derive(Serialize)]
struct GraphqlRequest {
    query: &'static str,
    variables: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct GraphqlResponse<T> {
    #[serde(default)]
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

#[derive(Deserialize)]
struct RepositoryLookupData {
    repository: Option<RepositoryFields>,
}

#[derive(Debug, Deserialize)]
struct RepositoryFields {
    id: String,
    #[serde(rename = "hasSponsorshipsEnabled")]
    has_sponsorships_enabled: bool,
}

#[derive(Deserialize)]
struct UpdateRepositoryData {
    #[serde(rename = "updateRepository")]
    update_repository: Option<UpdateRepositoryPayload>,
}

#[derive(Deserialize)]
struct UpdateRepositoryPayload {
    repository: UpdatedRepositoryFields,
}

#[derive(Deserialize)]
struct UpdatedRepositoryFields {
    #[serde(rename = "hasSponsorshipsEnabled")]
    has_sponsorships_enabled: bool,
}

fn join_graphql_errors(errors: &[GraphqlError]) -> String {
    errors
        .iter()
        .map(|e| e.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Turn a parsed lookup response into the repository's id + current
/// sponsorship state, or a clear error naming the lookup call.
fn interpret_repository_response(
    resp: GraphqlResponse<RepositoryLookupData>,
    owner: &str,
    name: &str,
) -> Result<RepositoryFields> {
    if let Some(errors) = resp.errors.filter(|e| !e.is_empty()) {
        bail!(
            "Repository lookup call failed: {}",
            join_graphql_errors(&errors)
        );
    }

    match resp.data.and_then(|d| d.repository) {
        Some(repo) => Ok(repo),
        None => bail!("Repository lookup call failed: repository '{owner}/{name}' not found"),
    }
}

/// Turn a parsed mutation response into the resulting `hasSponsorshipsEnabled`
/// value, or a clear error naming the enable call.
fn interpret_update_response(resp: GraphqlResponse<UpdateRepositoryData>) -> Result<bool> {
    if let Some(errors) = resp.errors.filter(|e| !e.is_empty()) {
        bail!(
            "Enable-sponsorships call failed: {}",
            join_graphql_errors(&errors)
        );
    }

    let payload = resp.data.and_then(|d| d.update_repository).ok_or_else(|| {
        anyhow::anyhow!(
            "Enable-sponsorships call failed: GitHub returned no repository in the response"
        )
    })?;

    Ok(payload.repository.has_sponsorships_enabled)
}

fn fetch_repository(client: &GithubClient, owner: &str, name: &str) -> Result<RepositoryFields> {
    let req = GraphqlRequest {
        query: REPOSITORY_LOOKUP_QUERY,
        variables: serde_json::json!({ "owner": owner, "name": name }),
    };
    let resp: GraphqlResponse<RepositoryLookupData> = client
        .post(GRAPHQL_PATH, &req)
        .context("Repository lookup call failed")?;
    interpret_repository_response(resp, owner, name)
}

fn set_has_sponsorships_enabled(client: &GithubClient, repository_id: &str) -> Result<bool> {
    let req = GraphqlRequest {
        query: ENABLE_SPONSORSHIPS_MUTATION,
        variables: serde_json::json!({ "repositoryId": repository_id }),
    };
    let resp: GraphqlResponse<UpdateRepositoryData> = client
        .post(GRAPHQL_PATH, &req)
        .context("Enable-sponsorships call failed")?;
    interpret_update_response(resp)
}

/// Enable the GitHub Sponsor button on `owner/name`.
///
/// Resolves the repository's GraphQL node id first, then — only if the
/// Sponsor button is currently off — flips `hasSponsorshipsEnabled` on. If
/// it is already on, no mutation call is made and this reports
/// [`SponsorshipStatus::AlreadyEnabled`].
pub fn enable_sponsorships(
    client: &GithubClient,
    owner: &str,
    name: &str,
) -> Result<SponsorshipStatus> {
    let repo = fetch_repository(client, owner, name)?;

    if repo.has_sponsorships_enabled {
        return Ok(SponsorshipStatus::AlreadyEnabled);
    }

    let enabled = set_has_sponsorships_enabled(client, &repo.id)?;
    if !enabled {
        bail!(
            "Enable-sponsorships call failed: GitHub reported hasSponsorshipsEnabled=false after the mutation"
        );
    }

    Ok(SponsorshipStatus::Enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_response(json: &str) -> GraphqlResponse<RepositoryLookupData> {
        serde_json::from_str(json).unwrap()
    }

    fn update_response(json: &str) -> GraphqlResponse<UpdateRepositoryData> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn test_interpret_repository_response_success_disabled() {
        let resp = lookup_response(
            r#"{"data":{"repository":{"id":"R_kgDORv6zWQ","hasSponsorshipsEnabled":false}}}"#,
        );
        let repo = interpret_repository_response(resp, "UniverLab", "repo").unwrap();
        assert_eq!(repo.id, "R_kgDORv6zWQ");
        assert!(!repo.has_sponsorships_enabled);
    }

    #[test]
    fn test_interpret_repository_response_success_already_enabled() {
        let resp = lookup_response(
            r#"{"data":{"repository":{"id":"R_kgDORv6zWQ","hasSponsorshipsEnabled":true}}}"#,
        );
        let repo = interpret_repository_response(resp, "UniverLab", "repo").unwrap();
        assert!(repo.has_sponsorships_enabled);
    }

    #[test]
    fn test_interpret_repository_response_null_repository() {
        let resp = lookup_response(r#"{"data":{"repository":null}}"#);
        let err = interpret_repository_response(resp, "UniverLab", "ghost")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Repository lookup call failed"));
        assert!(err.contains("UniverLab/ghost"));
        assert!(err.contains("not found"));
    }

    #[test]
    fn test_interpret_repository_response_missing_data() {
        let resp = lookup_response(r#"{"data":null}"#);
        let err = interpret_repository_response(resp, "UniverLab", "ghost")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Repository lookup call failed"));
        assert!(err.contains("not found"));
    }

    #[test]
    fn test_interpret_repository_response_not_found_graphql_error() {
        let resp = lookup_response(
            r#"{"data":{"repository":null},"errors":[{"type":"NOT_FOUND","message":"Could not resolve to a Repository with the name 'ghost/repo'."}]}"#,
        );
        let err = interpret_repository_response(resp, "ghost", "repo")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Repository lookup call failed"));
        assert!(err.contains("Could not resolve to a Repository"));
    }

    #[test]
    fn test_interpret_repository_response_insufficient_scope() {
        let resp = lookup_response(
            r#"{"data":null,"errors":[{"type":"INSUFFICIENT_SCOPES","message":"Your token has not been granted the required scopes to execute this query."}]}"#,
        );
        let err = interpret_repository_response(resp, "UniverLab", "repo")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Repository lookup call failed"));
        assert!(err.contains("required scopes"));
    }

    #[test]
    fn test_interpret_repository_response_multiple_errors_joined() {
        let resp = lookup_response(
            r#"{"data":null,"errors":[{"message":"first problem"},{"message":"second problem"}]}"#,
        );
        let err = interpret_repository_response(resp, "o", "r")
            .unwrap_err()
            .to_string();
        assert!(err.contains("first problem; second problem"));
    }

    #[test]
    fn test_interpret_update_response_success_true() {
        let resp = update_response(
            r#"{"data":{"updateRepository":{"repository":{"hasSponsorshipsEnabled":true}}}}"#,
        );
        assert!(interpret_update_response(resp).unwrap());
    }

    #[test]
    fn test_interpret_update_response_success_false() {
        let resp = update_response(
            r#"{"data":{"updateRepository":{"repository":{"hasSponsorshipsEnabled":false}}}}"#,
        );
        assert!(!interpret_update_response(resp).unwrap());
    }

    #[test]
    fn test_interpret_update_response_insufficient_scope() {
        let resp = update_response(
            r#"{"data":null,"errors":[{"type":"INSUFFICIENT_SCOPES","message":"Your token has not been granted the required scopes to execute this query."}]}"#,
        );
        let err = interpret_update_response(resp).unwrap_err().to_string();
        assert!(err.contains("Enable-sponsorships call failed"));
        assert!(err.contains("required scopes"));
    }

    #[test]
    fn test_interpret_update_response_missing_payload() {
        let resp = update_response(r#"{"data":{"updateRepository":null}}"#);
        let err = interpret_update_response(resp).unwrap_err().to_string();
        assert!(err.contains("Enable-sponsorships call failed"));
    }

    #[test]
    fn test_interpret_update_response_null_data() {
        let resp = update_response(r#"{"data":null}"#);
        let err = interpret_update_response(resp).unwrap_err().to_string();
        assert!(err.contains("Enable-sponsorships call failed"));
    }

    #[test]
    fn test_error_message_names_lookup_call_not_mutation() {
        let resp = lookup_response(r#"{"data":null,"errors":[{"message":"some failure"}]}"#);
        let err = interpret_repository_response(resp, "o", "r")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Repository lookup"));
        assert!(!err.contains("Enable-sponsorships"));
    }

    #[test]
    fn test_error_message_names_mutation_call_not_lookup() {
        let resp = update_response(r#"{"data":null,"errors":[{"message":"some failure"}]}"#);
        let err = interpret_update_response(resp).unwrap_err().to_string();
        assert!(err.contains("Enable-sponsorships"));
        assert!(!err.contains("Repository lookup"));
    }

    #[test]
    fn test_join_graphql_errors_single() {
        let errors = vec![GraphqlError {
            message: "boom".to_string(),
        }];
        assert_eq!(join_graphql_errors(&errors), "boom");
    }

    #[test]
    fn test_join_graphql_errors_multiple() {
        let errors = vec![
            GraphqlError {
                message: "first".to_string(),
            },
            GraphqlError {
                message: "second".to_string(),
            },
        ];
        assert_eq!(join_graphql_errors(&errors), "first; second");
    }

    #[test]
    fn test_join_graphql_errors_empty() {
        let errors: Vec<GraphqlError> = vec![];
        assert_eq!(join_graphql_errors(&errors), "");
    }

    #[test]
    fn test_repository_lookup_query_shape() {
        assert!(REPOSITORY_LOOKUP_QUERY.contains("repository(owner: $owner, name: $name)"));
        assert!(REPOSITORY_LOOKUP_QUERY.contains("id"));
        assert!(REPOSITORY_LOOKUP_QUERY.contains("hasSponsorshipsEnabled"));
    }

    #[test]
    fn test_enable_sponsorships_mutation_shape() {
        assert!(ENABLE_SPONSORSHIPS_MUTATION.contains("updateRepository"));
        assert!(ENABLE_SPONSORSHIPS_MUTATION.contains("hasSponsorshipsEnabled: true"));
        assert!(!ENABLE_SPONSORSHIPS_MUTATION.contains("private"));
        assert!(!ENABLE_SPONSORSHIPS_MUTATION.contains("description"));
    }

    #[test]
    fn test_graphql_request_serialization() {
        let req = GraphqlRequest {
            query: "query { viewer { login } }",
            variables: serde_json::json!({ "owner": "o", "name": "r" }),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"query\""));
        assert!(json.contains("\"variables\""));
        assert!(json.contains("\"owner\":\"o\""));
    }

    #[test]
    fn test_graphql_request_mutation_variables_serialization() {
        let req = GraphqlRequest {
            query: ENABLE_SPONSORSHIPS_MUTATION,
            variables: serde_json::json!({ "repositoryId": "R_kgDORv6zWQ" }),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"repositoryId\":\"R_kgDORv6zWQ\""));
    }

    #[test]
    fn test_sponsorship_status_equality() {
        assert_eq!(SponsorshipStatus::Enabled, SponsorshipStatus::Enabled);
        assert_eq!(
            SponsorshipStatus::AlreadyEnabled,
            SponsorshipStatus::AlreadyEnabled
        );
        assert_ne!(
            SponsorshipStatus::Enabled,
            SponsorshipStatus::AlreadyEnabled
        );
    }

    #[test]
    fn test_sponsorship_status_debug() {
        assert_eq!(format!("{:?}", SponsorshipStatus::Enabled), "Enabled");
        assert_eq!(
            format!("{:?}", SponsorshipStatus::AlreadyEnabled),
            "AlreadyEnabled"
        );
    }
}
