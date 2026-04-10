use anyhow::{Context, Result};
use blake2::digest::{Update, VariableOutput};
use blake2::Blake2bVar;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use xsalsa20poly1305::KeyInit;

const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const DOMAIN_SEPARATOR: &[u8] = b"|ghscaff-vault-v1";

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct VaultData {
    #[serde(default)]
    pub github_token: Option<String>,
    #[serde(default)]
    pub has_passphrase: bool,
    #[serde(default)]
    pub secrets: HashMap<String, String>,
}

/// Blake2b-256(username ‖ hostname ‖ binary_path ‖ passphrase ‖ domain)
fn derive_key(passphrase: &str) -> Result<[u8; KEY_LEN]> {
    let mut hasher = Blake2bVar::new(KEY_LEN).expect("valid output size");
    Update::update(&mut hasher, whoami::username().as_bytes());
    Update::update(&mut hasher, b"|");
    Update::update(
        &mut hasher,
        whoami::fallible::hostname().unwrap_or_default().as_bytes(),
    );
    Update::update(&mut hasher, b"|");
    Update::update(
        &mut hasher,
        std::env::current_exe()
            .unwrap_or_default()
            .to_string_lossy()
            .as_bytes(),
    );
    Update::update(&mut hasher, b"|");
    Update::update(&mut hasher, passphrase.as_bytes());
    Update::update(&mut hasher, DOMAIN_SEPARATOR);

    let mut key = [0u8; KEY_LEN];
    hasher
        .finalize_variable(&mut key)
        .expect("buffer matches output size");
    Ok(key)
}

fn vault_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Cannot resolve home directory")?;
    Ok(home.join(".ghscaff").join("vault.enc"))
}

/// File format: [nonce:24][ciphertext+poly1305_tag]
fn save_to_path(data: &VaultData, passphrase: &str, path: &Path) -> Result<()> {
    use crypto_box::aead::{generic_array::GenericArray, rand_core::RngCore, Aead, OsRng};

    let key_bytes = derive_key(passphrase)?;
    let key = GenericArray::from_slice(&key_bytes);
    let cipher = xsalsa20poly1305::XSalsa20Poly1305::new(key);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = GenericArray::from_slice(&nonce_bytes);

    let plaintext = serde_json::to_vec(data).context("Failed to serialize vault")?;
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_ref())
        .map_err(|_| anyhow::anyhow!("Encryption failed"))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }

    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);

    // Atomic write: write to temp file then rename
    let tmp_path = path.with_extension("enc.tmp");
    std::fs::write(&tmp_path, &blob)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))?;
    }

    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

fn load_from_path(passphrase: &str, path: &Path) -> Result<Option<VaultData>> {
    use crypto_box::aead::{generic_array::GenericArray, Aead};

    if !path.exists() {
        return Ok(None);
    }

    let blob = std::fs::read(path).context("Failed to read vault file")?;
    if blob.len() < NONCE_LEN {
        anyhow::bail!("Corrupt vault file");
    }

    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let key_bytes = derive_key(passphrase)?;
    let key = GenericArray::from_slice(&key_bytes);
    let cipher = xsalsa20poly1305::XSalsa20Poly1305::new(key);
    let nonce = GenericArray::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("Decryption failed — wrong passphrase or corrupt vault"))?;

    serde_json::from_slice(&plaintext)
        .map(Some)
        .context("Failed to parse vault contents")
}

pub fn save(data: &VaultData, passphrase: &str) -> Result<()> {
    save_to_path(data, passphrase, &vault_path()?)
}

pub fn load(passphrase: &str) -> Result<Option<VaultData>> {
    load_from_path(passphrase, &vault_path()?)
}

pub fn destroy() -> Result<bool> {
    let path = vault_path()?;
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path)?;
    Ok(true)
}

pub fn exists() -> bool {
    vault_path().map(|p| p.exists()).unwrap_or(false)
}

/// Try without passphrase first; prompt if vault has one.
pub fn load_interactive() -> Result<Option<(VaultData, String)>> {
    if !exists() {
        return Ok(None);
    }

    if let Ok(Some(data)) = load("") {
        if !data.has_passphrase {
            return Ok(Some((data, String::new())));
        }
    }

    let passphrase = inquire::Password::new("Vault passphrase:")
        .without_confirmation()
        .prompt()
        .context("Failed to read passphrase")?;

    let data = load(&passphrase)?
        .ok_or_else(|| anyhow::anyhow!("Failed to decrypt vault — wrong passphrase"))?;
    Ok(Some((data, passphrase)))
}

pub fn resolve_github_token() -> Result<Option<(String, String)>> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        return Ok(Some((token, String::new())));
    }

    if let Some((data, passphrase)) = load_interactive()? {
        if let Some(token) = data.github_token {
            return Ok(Some((token, passphrase)));
        }
    }

    Ok(None)
}

pub fn resolve_secret(name: &str, passphrase: &str) -> Result<Option<String>> {
    if let Ok(val) = std::env::var(name) {
        return Ok(Some(val));
    }

    if let Some(data) = load(passphrase)? {
        if let Some(val) = data.secrets.get(name) {
            return Ok(Some(val.clone()));
        }
    }

    Ok(None)
}

pub fn prompt_and_save_github_token() -> Result<(String, String)> {
    let token = inquire::Password::new("GitHub token (ghp_...):")
        .with_help_message("Required scopes: repo, workflow — https://github.com/settings/tokens")
        .without_confirmation()
        .prompt()
        .context("Failed to read token")?;

    if token.is_empty() {
        anyhow::bail!("Token cannot be empty");
    }

    let passphrase = if exists() {
        load_interactive()?
            .ok_or_else(|| anyhow::anyhow!("Cannot read existing vault"))?
            .1
    } else {
        ask_optional_passphrase()?
    };

    let mut data = load(&passphrase)?.unwrap_or_default();
    data.github_token = Some(token.clone());
    data.has_passphrase = !passphrase.is_empty();
    save(&data, &passphrase)?;

    println!("  \x1b[32m✓\x1b[0m Token saved to encrypted vault (~/.ghscaff/vault.enc)");
    Ok((token, passphrase))
}

fn ask_optional_passphrase() -> Result<String> {
    let want = inquire::Confirm::new("Add an optional passphrase to protect the vault?")
        .with_default(false)
        .with_help_message("If set, you'll need to enter it each time ghscaff runs")
        .prompt()?;

    if !want {
        return Ok(String::new());
    }

    Ok(inquire::Password::new("Passphrase:")
        .prompt()
        .unwrap_or_default())
}

pub fn save_secret(name: &str, value: &str, passphrase: &str) -> Result<()> {
    let mut data = load(passphrase)?.unwrap_or_default();
    data.secrets.insert(name.to_string(), value.to_string());
    data.has_passphrase = !passphrase.is_empty();
    save(&data, passphrase)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_vault_data() -> VaultData {
        let mut secrets = HashMap::new();
        secrets.insert("CARGO_REGISTRY_TOKEN".into(), "crates_token_123".into());
        VaultData {
            github_token: Some("ghp_test_token_abc".into()),
            has_passphrase: false,
            secrets,
        }
    }

    #[test]
    fn save_load_roundtrip_no_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let data = test_vault_data();

        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();

        assert_eq!(loaded.github_token, data.github_token);
        assert_eq!(loaded.secrets, data.secrets);
        assert!(!loaded.has_passphrase);
    }

    #[test]
    fn save_load_roundtrip_with_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let mut data = test_vault_data();
        data.has_passphrase = true;

        save_to_path(&data, "my-secret-pass", &path).unwrap();
        let loaded = load_from_path("my-secret-pass", &path).unwrap().unwrap();

        assert_eq!(loaded.github_token, data.github_token);
        assert_eq!(loaded.secrets, data.secrets);
        assert!(loaded.has_passphrase);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");

        save_to_path(&test_vault_data(), "correct", &path).unwrap();
        let result = load_from_path("wrong", &path);

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Decryption failed"));
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.enc");

        let result = load_from_path("", &path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn corrupt_file_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        std::fs::write(&path, b"short").unwrap();

        let result = load_from_path("", &path);
        assert!(result.is_err());
    }

    #[test]
    fn derive_key_differs_with_passphrase() {
        let key_empty = derive_key("").unwrap();
        let key_pass = derive_key("secret").unwrap();
        assert_ne!(key_empty, key_pass);
    }

    #[test]
    fn derive_key_deterministic() {
        let key1 = derive_key("test").unwrap();
        let key2 = derive_key("test").unwrap();
        assert_eq!(key1, key2);
    }

    #[test]
    fn vault_data_default() {
        let data = VaultData::default();
        assert!(data.github_token.is_none());
        assert!(!data.has_passphrase);
        assert!(data.secrets.is_empty());
    }

    #[test]
    fn save_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");

        let mut data1 = VaultData::default();
        data1.github_token = Some("token_v1".into());
        save_to_path(&data1, "", &path).unwrap();

        let mut data2 = VaultData::default();
        data2.github_token = Some("token_v2".into());
        save_to_path(&data2, "", &path).unwrap();

        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.github_token.unwrap(), "token_v2");
    }
}
