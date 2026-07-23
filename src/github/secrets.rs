use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use blake2::{
    digest::{Update, VariableOutput},
    Blake2bVar,
};
use crypto_box::{
    aead::{generic_array::GenericArray, Aead, OsRng},
    PublicKey, SalsaBox, SecretKey,
};
use serde::{Deserialize, Serialize};

use super::client::GithubClient;

#[derive(Deserialize)]
struct PublicKeyResponse {
    key_id: String,
    key: String,
}

#[derive(Deserialize)]
struct SecretsListResponse {
    secrets: Vec<SecretEntry>,
}

#[derive(Deserialize)]
struct SecretEntry {
    name: String,
}

#[derive(Serialize)]
struct SetSecretBody {
    encrypted_value: String,
    key_id: String,
}

/// NaCl sealed box encryption — matches libsodium crypto_box_seal.
/// Format: ephemeral_pk (32 bytes) || box(msg, nonce, DH(eph_sk, recipient_pk))
/// where nonce = Blake2b-24(eph_pk || recipient_pk).
fn sealed_box_encrypt(recipient_pk: &PublicKey, plaintext: &[u8]) -> Vec<u8> {
    let ephemeral_sk = SecretKey::generate(&mut OsRng);
    let ephemeral_pk = ephemeral_sk.public_key();

    let mut nonce_bytes = [0u8; 24];
    let mut hasher = Blake2bVar::new(24).expect("valid output size");
    Update::update(&mut hasher, ephemeral_pk.as_bytes());
    Update::update(&mut hasher, recipient_pk.as_bytes());
    hasher
        .finalize_variable(&mut nonce_bytes)
        .expect("buffer matches output size");

    let nonce = GenericArray::from_slice(&nonce_bytes);
    let salsa = SalsaBox::new(recipient_pk, &ephemeral_sk);
    let ciphertext = salsa
        .encrypt(nonce, plaintext)
        .expect("encryption cannot fail");

    let mut result = Vec::with_capacity(32 + ciphertext.len());
    result.extend_from_slice(ephemeral_pk.as_bytes());
    result.extend_from_slice(&ciphertext);
    result
}

/// Returns the names of all secrets currently configured on the repo.
pub fn list_secret_names(client: &GithubClient, owner: &str, repo: &str) -> Result<Vec<String>> {
    let path = format!("/repos/{owner}/{repo}/actions/secrets?per_page=100");
    let resp: SecretsListResponse = client.get(&path)?;
    Ok(resp.secrets.into_iter().map(|s| s.name).collect())
}

/// Encrypts `value` with the repo's public key and stores it as a secret.
pub fn set_secret(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    name: &str,
    value: &str,
) -> Result<()> {
    let pk: PublicKeyResponse = client
        .get(&format!("/repos/{owner}/{repo}/actions/secrets/public-key"))
        .context("Failed to fetch repo public key")?;

    let key_bytes = STANDARD
        .decode(&pk.key)
        .context("Failed to base64-decode repo public key")?;
    let key_arr: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("Repo public key must be 32 bytes"))?;

    let recipient_pk = PublicKey::from(key_arr);
    let ciphertext = sealed_box_encrypt(&recipient_pk, value.as_bytes());
    let encrypted_value = STANDARD.encode(ciphertext);

    let body = SetSecretBody {
        encrypted_value,
        key_id: pk.key_id,
    };
    client.put_no_response(
        &format!("/repos/{owner}/{repo}/actions/secrets/{name}"),
        &body,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_key_response_deserialize() {
        let json = r#"{"key_id":"123456","key":"I1cLzGPaF4jCwAN8kWxLOaAvGNp3MhPNMCLkSSkCJ3s="}"#;
        let resp: PublicKeyResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.key_id, "123456");
        assert_eq!(resp.key, "I1cLzGPaF4jCwAN8kWxLOaAvGNp3MhPNMCLkSSkCJ3s=");
    }

    #[test]
    fn test_secrets_list_response_deserialize() {
        let json = r#"{"secrets":[{"name":"SECRET1"},{"name":"SECRET2"}]}"#;
        let resp: SecretsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.secrets.len(), 2);
        assert_eq!(resp.secrets[0].name, "SECRET1");
        assert_eq!(resp.secrets[1].name, "SECRET2");
    }

    #[test]
    fn test_secrets_list_response_empty() {
        let json = r#"{"secrets":[]}"#;
        let resp: SecretsListResponse = serde_json::from_str(json).unwrap();
        assert!(resp.secrets.is_empty());
    }

    #[test]
    fn test_secret_entry_deserialize() {
        let json = r#"{"name":"MY_API_KEY","created_at":"2024-01-01T00:00:00Z","updated_at":"2024-01-01T00:00:00Z"}"#;
        let entry: SecretEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.name, "MY_API_KEY");
    }

    #[test]
    fn test_set_secret_body_serialization() {
        let body = SetSecretBody {
            encrypted_value: "base64encodedciphertext".to_string(),
            key_id: "key_id_123".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"encrypted_value\":\"base64encodedciphertext\""));
        assert!(json.contains("\"key_id\":\"key_id_123\""));
    }

    #[test]
    fn test_set_secret_body_empty_encrypted_value() {
        let body = SetSecretBody {
            encrypted_value: String::new(),
            key_id: "key_1".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("\"encrypted_value\":\"\""));
    }

    #[test]
    fn test_sealed_box_encrypt_output_length() {
        use crypto_box::PublicKey;
        let recipient_pk = PublicKey::from([1u8; 32]);
        let plaintext = b"hello world";
        let result = sealed_box_encrypt(&recipient_pk, plaintext);
        // sealed box = 32 bytes ephemeral pk + ciphertext (plaintext_len + 16 poly1305 tag)
        assert_eq!(result.len(), 32 + plaintext.len() + 16);
    }

    #[test]
    fn test_sealed_box_encrypt_different_each_time() {
        use crypto_box::PublicKey;
        let recipient_pk = PublicKey::from([2u8; 32]);
        let plaintext = b"same input";
        let result1 = sealed_box_encrypt(&recipient_pk, plaintext);
        let result2 = sealed_box_encrypt(&recipient_pk, plaintext);
        // Different ephemeral keys each time
        assert_ne!(result1, result2);
    }

    #[test]
    fn test_sealed_box_encrypt_ephemeral_pk_prefix() {
        use crypto_box::PublicKey;
        let recipient_pk = PublicKey::from([3u8; 32]);
        let plaintext = b"test";
        let result = sealed_box_encrypt(&recipient_pk, plaintext);
        // First 32 bytes are the ephemeral public key
        assert_eq!(result.len(), 32 + 4 + 16);
    }

    #[test]
    fn test_sealed_box_encrypt_empty_plaintext() {
        use crypto_box::PublicKey;
        let recipient_pk = PublicKey::from([4u8; 32]);
        let plaintext = b"";
        let result = sealed_box_encrypt(&recipient_pk, plaintext);
        // empty plaintext = 32 (eph pk) + 0 + 16 (tag) = 48
        assert_eq!(result.len(), 48);
    }

    #[test]
    fn test_sealed_box_encrypt_long_plaintext() {
        use crypto_box::PublicKey;
        let recipient_pk = PublicKey::from([5u8; 32]);
        let plaintext = b"a".repeat(10000);
        let result = sealed_box_encrypt(&recipient_pk, &plaintext);
        assert_eq!(result.len(), 32 + 10000 + 16);
    }

    #[test]
    fn test_sealed_box_encrypt_binary_plaintext() {
        use crypto_box::PublicKey;
        let recipient_pk = PublicKey::from([6u8; 32]);
        let plaintext: Vec<u8> = (0..=255).cycle().take(256).collect();
        let result = sealed_box_encrypt(&recipient_pk, &plaintext);
        assert_eq!(result.len(), 32 + 256 + 16);
    }

    #[test]
    fn test_list_secret_names_from_json() {
        let json =
            r#"{"total_count":3,"secrets":[{"name":"TOKEN"},{"name":"KEY"},{"name":"SECRET"}]}"#;
        let resp: SecretsListResponse = serde_json::from_str(json).unwrap();
        let names: Vec<String> = resp.secrets.into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["TOKEN", "KEY", "SECRET"]);
    }

    #[test]
    fn test_public_key_response_real_github_format() {
        let json = r#"{
            "key_id": "352845986",
            "key": "2Sg8iY4AOfKrV16708bPs5dJB9BvX7MA6P8BaQdNh58="
        }"#;
        let resp: PublicKeyResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.key_id, "352845986");
        assert_eq!(resp.key, "2Sg8iY4AOfKrV16708bPs5dJB9BvX7MA6P8BaQdNh58=");
    }

    #[test]
    fn test_secrets_list_real_github_format() {
        let json = r#"{
            "total_count": 2,
            "secrets": [
                {
                    "name": "DEPLOY_KEY",
                    "created_at": "2024-01-15T10:00:00Z",
                    "updated_at": "2024-01-15T10:00:00Z"
                },
                {
                    "name": "CODECOV_TOKEN",
                    "created_at": "2024-02-01T12:00:00Z",
                    "updated_at": "2024-02-01T12:00:00Z"
                }
            ]
        }"#;
        let resp: SecretsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.secrets.len(), 2);
        assert_eq!(resp.secrets[0].name, "DEPLOY_KEY");
        assert_eq!(resp.secrets[1].name, "CODECOV_TOKEN");
    }

    #[test]
    fn test_set_secret_body_with_long_key_id() {
        let body = SetSecretBody {
            encrypted_value: "encrypted_data_here".to_string(),
            key_id: "a".repeat(100),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains(&"a".repeat(100)));
    }

    #[test]
    fn test_public_key_response_deserialize_empty_key() {
        let json = r#"{"key_id":"0","key":""}"#;
        let resp: PublicKeyResponse = serde_json::from_str(json).unwrap();
        assert!(resp.key.is_empty());
        assert_eq!(resp.key_id, "0");
    }

    #[test]
    fn test_secrets_list_response_deserialize_single_secret() {
        let json = r#"{"secrets":[{"name":"ONLY_SECRET"}]}"#;
        let resp: SecretsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.secrets.len(), 1);
        assert_eq!(resp.secrets[0].name, "ONLY_SECRET");
    }

    #[test]
    fn test_secret_entry_deserialize_minimal() {
        let json = r#"{"name":"MY_SECRET"}"#;
        let entry: SecretEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.name, "MY_SECRET");
    }

    #[test]
    fn test_set_secret_body_json_roundtrip() {
        let body = SetSecretBody {
            encrypted_value: "enc_value_123".to_string(),
            key_id: "key_456".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["encrypted_value"], "enc_value_123");
        assert_eq!(parsed["key_id"], "key_456");
    }

    #[test]
    fn test_secrets_list_real_github_many_secrets() {
        let json = r#"{
            "total_count": 5,
            "secrets": [
                {"name": "SECRET_A", "created_at": "2024-01-01T00:00:00Z", "updated_at": "2024-01-01T00:00:00Z"},
                {"name": "SECRET_B", "created_at": "2024-01-02T00:00:00Z", "updated_at": "2024-01-02T00:00:00Z"},
                {"name": "SECRET_C", "created_at": "2024-01-03T00:00:00Z", "updated_at": "2024-01-03T00:00:00Z"},
                {"name": "SECRET_D", "created_at": "2024-01-04T00:00:00Z", "updated_at": "2024-01-04T00:00:00Z"},
                {"name": "SECRET_E", "created_at": "2024-01-05T00:00:00Z", "updated_at": "2024-01-05T00:00:00Z"}
            ]
        }"#;
        let resp: SecretsListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.secrets.len(), 5);
        let names: Vec<&str> = resp.secrets.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"SECRET_C"));
    }

    #[test]
    fn test_sealed_box_encrypt_unicode_plaintext() {
        use crypto_box::PublicKey;
        let recipient_pk = PublicKey::from([7u8; 32]);
        let plaintext = "日本語テキスト".as_bytes();
        let result = sealed_box_encrypt(&recipient_pk, plaintext);
        // 32 (eph pk) + plaintext_len + 16 (tag)
        assert_eq!(result.len(), 32 + plaintext.len() + 16);
    }

    #[test]
    fn test_set_secret_body_special_chars() {
        let body = SetSecretBody {
            encrypted_value: "base64+with/special=chars".to_string(),
            key_id: "key-with-dashes_123".to_string(),
        };
        let json = serde_json::to_string(&body).unwrap();
        assert!(json.contains("base64+with/special=chars"));
        assert!(json.contains("key-with-dashes_123"));
    }
}
