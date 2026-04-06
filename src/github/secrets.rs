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
