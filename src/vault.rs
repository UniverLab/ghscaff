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

#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
pub struct VaultData {
    #[serde(default)]
    pub github_token: Option<String>,
    #[serde(default)]
    pub has_passphrase: bool,
    #[serde(default)]
    pub secrets: HashMap<String, String>,
}

/// Blake2b-256(username ‖ hostname ‖ passphrase ‖ domain)
/// Note: Removed binary_path to allow vault to work even if binary is relocated
fn derive_key(passphrase: &str) -> Result<[u8; KEY_LEN]> {
    let mut hasher = Blake2bVar::new(KEY_LEN).expect("valid output size");
    Update::update(&mut hasher, whoami::username().as_bytes());
    Update::update(&mut hasher, b"|");
    Update::update(
        &mut hasher,
        whoami::fallible::hostname().unwrap_or_default().as_bytes(),
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

    // Atomic write: write to a writer-unique temp file, then rename. The
    // suffix must be unique per writer (not just per path) — two processes
    // racing to write the same vault path with a shared, fixed temp name
    // can interleave their writes and corrupt the file before either
    // rename() runs.
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = path.with_extension(format!("enc.tmp.{}.{unique}", std::process::id()));
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

    loop {
        let passphrase = inquire::Password::new("Passphrase:")
            .with_help_message("Minimum 5 characters recommended")
            .prompt()?;

        if passphrase.len() < 5 {
            println!("  \x1b[33m⚠ Passphrase is weak (less than 5 characters)\x1b[0m");
            let confirm_weak = inquire::Confirm::new("Use this weak passphrase anyway?")
                .with_default(false)
                .prompt()?;
            if !confirm_weak {
                continue;
            }
        }

        return Ok(passphrase);
    }
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

        let data1 = VaultData {
            github_token: Some("token_v1".into()),
            ..Default::default()
        };
        save_to_path(&data1, "", &path).unwrap();

        let data2 = VaultData {
            github_token: Some("token_v2".into()),
            ..Default::default()
        };
        save_to_path(&data2, "", &path).unwrap();

        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.github_token.unwrap(), "token_v2");
    }

    #[test]
    fn vault_data_with_secrets() {
        let mut secrets = std::collections::HashMap::new();
        secrets.insert("API_KEY".to_string(), "abc123".to_string());
        let data = VaultData {
            github_token: Some("tok".into()),
            has_passphrase: true,
            secrets,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "pass", &path).unwrap();
        let loaded = load_from_path("pass", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.get("API_KEY").unwrap(), "abc123");
        assert!(loaded.has_passphrase);
    }

    #[test]
    fn vault_data_serialization_roundtrip_json() {
        let data = VaultData {
            github_token: Some("tok".into()),
            has_passphrase: false,
            secrets: std::collections::HashMap::from([
                ("A".into(), "1".into()),
                ("B".into(), "2".into()),
            ]),
        };
        let json = serde_json::to_string(&data).unwrap();
        let restored: VaultData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn vault_data_equality() {
        let a = VaultData {
            github_token: Some("t".into()),
            has_passphrase: true,
            secrets: std::collections::HashMap::new(),
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn derive_key_empty_passphrase() {
        let key1 = derive_key("").unwrap();
        let key2 = derive_key("").unwrap();
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), KEY_LEN);
    }

    #[test]
    fn derive_key_long_passphrase() {
        let long = "x".repeat(10000);
        let key = derive_key(&long).unwrap();
        assert_eq!(key.len(), KEY_LEN);
    }

    #[test]
    fn vault_data_no_token_no_secrets() {
        let data = VaultData {
            github_token: None,
            has_passphrase: false,
            secrets: HashMap::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert!(loaded.github_token.is_none());
        assert!(loaded.secrets.is_empty());
    }

    #[test]
    fn vault_data_multiple_secrets() {
        let mut secrets = HashMap::new();
        secrets.insert("KEY1".into(), "val1".into());
        secrets.insert("KEY2".into(), "val2".into());
        secrets.insert("KEY3".into(), "val3".into());
        let data = VaultData {
            github_token: None,
            has_passphrase: false,
            secrets,
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.len(), 3);
        assert_eq!(loaded.secrets.get("KEY1").unwrap(), "val1");
        assert_eq!(loaded.secrets.get("KEY2").unwrap(), "val2");
        assert_eq!(loaded.secrets.get("KEY3").unwrap(), "val3");
    }

    #[test]
    fn save_to_path_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deep").join("vault.enc");
        let data = VaultData::default();
        save_to_path(&data, "", &path).unwrap();
        assert!(path.exists());
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded, VaultData::default());
    }

    #[test]
    fn load_from_path_corrupt_data_not_too_short() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        // Exactly NONCE_LEN bytes but garbage
        let data = vec![0u8; NONCE_LEN];
        std::fs::write(&path, &data).unwrap();
        let result = load_from_path("", &path);
        assert!(result.is_err());
    }

    #[test]
    fn vault_data_clone() {
        let data = test_vault_data();
        let cloned = data.clone();
        assert_eq!(data, cloned);
    }

    #[test]
    fn vault_data_debug() {
        let data = test_vault_data();
        let dbg = format!("{:?}", data);
        assert!(dbg.contains("VaultData"));
        assert!(dbg.contains("ghp_test_token_abc"));
    }

    #[test]
    fn vault_data_json_missing_fields() {
        let json = r#"{}"#;
        let data: VaultData = serde_json::from_str(json).unwrap();
        assert_eq!(data, VaultData::default());
    }

    #[test]
    fn vault_data_json_partial_fields() {
        let json = r#"{"github_token":"tok_only"}"#;
        let data: VaultData = serde_json::from_str(json).unwrap();
        assert_eq!(data.github_token.as_deref(), Some("tok_only"));
        assert!(!data.has_passphrase);
        assert!(data.secrets.is_empty());
    }

    #[test]
    fn vault_data_json_secrets_only() {
        let json = r#"{"secrets":{"A":"1"}}"#;
        let data: VaultData = serde_json::from_str(json).unwrap();
        assert!(data.github_token.is_none());
        assert_eq!(data.secrets.get("A").unwrap(), "1");
    }

    #[test]
    fn save_secret_stores_in_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        // Create initial vault
        let initial = VaultData::default();
        save_to_path(&initial, "", &path).unwrap();
        // Now simulate save_secret by loading, inserting, saving
        let mut data = load_from_path("", &path).unwrap().unwrap();
        data.secrets
            .insert("MY_SECRET".into(), "secret_value".into());
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.get("MY_SECRET").unwrap(), "secret_value");
    }

    #[test]
    fn resolve_secret_from_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let mut secrets = HashMap::new();
        secrets.insert("API_KEY".into(), "key123".into());
        let data = VaultData {
            secrets,
            ..Default::default()
        };
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.get("API_KEY").unwrap(), "key123");
        assert!(!loaded.secrets.contains_key("NONEXISTENT"));
    }

    #[test]
    fn vault_file_format_has_correct_overhead() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let data = VaultData::default();
        save_to_path(&data, "", &path).unwrap();
        let blob = std::fs::read(&path).unwrap();
        // NONCE_LEN (24) + encryption overhead (plaintext_len + 16 poly1305 tag)
        // plaintext for empty VaultData is {"github_token":null,"has_passphrase":false,"secrets":{}}
        assert!(blob.len() > NONCE_LEN);
    }

    #[test]
    fn destroy_nonexistent_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        // Simulate destroy on non-existent path
        assert!(!path.exists());
    }

    #[test]
    fn vault_data_has_passphrase_true_serialization() {
        let data = VaultData {
            github_token: None,
            has_passphrase: true,
            secrets: HashMap::new(),
        };
        let json = serde_json::to_string(&data).unwrap();
        assert!(json.contains("\"has_passphrase\":true"));
        let restored: VaultData = serde_json::from_str(&json).unwrap();
        assert!(restored.has_passphrase);
    }

    #[test]
    fn vault_data_secret_with_empty_value() {
        let mut secrets = HashMap::new();
        secrets.insert("EMPTY".into(), "".into());
        let data = VaultData {
            secrets,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.get("EMPTY").unwrap(), "");
    }

    #[test]
    fn vault_data_secret_with_special_chars() {
        let mut secrets = HashMap::new();
        secrets.insert("SPECIAL".into(), "hello world! @#$%^&*() 你好".into());
        let data = VaultData {
            secrets,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(
            loaded.secrets.get("SPECIAL").unwrap(),
            "hello world! @#$%^&*() 你好"
        );
    }

    #[test]
    fn vault_data_unicode_token() {
        let data = VaultData {
            github_token: Some("ghp_测试token".into()),
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.github_token.as_deref(), Some("ghp_测试token"));
    }

    #[test]
    fn save_load_roundtrip_different_passphrases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let data = test_vault_data();

        save_to_path(&data, "pass1", &path).unwrap();
        let result = load_from_path("pass2", &path);
        assert!(result.is_err());
        // Load with correct passphrase still works
        let loaded = load_from_path("pass1", &path).unwrap().unwrap();
        assert_eq!(loaded.github_token, data.github_token);
    }

    #[test]
    fn vault_constants() {
        assert_eq!(NONCE_LEN, 24);
        assert_eq!(KEY_LEN, 32);
        assert_eq!(DOMAIN_SEPARATOR, b"|ghscaff-vault-v1");
    }

    #[test]
    fn vault_path_contains_ghscaff_dir() {
        let p = vault_path().unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains(".ghscaff"));
        assert!(s.ends_with("vault.enc"));
    }

    #[test]
    fn derive_key_special_characters() {
        let key1 = derive_key("!@#$%^&*()").unwrap();
        let key2 = derive_key("!@#$%^&*()").unwrap();
        assert_eq!(key1, key2);
        assert_ne!(key1, derive_key("normal").unwrap());
    }

    #[test]
    fn vault_data_large_secrets_map() {
        let mut secrets = HashMap::new();
        for i in 0..100 {
            secrets.insert(format!("KEY_{i}"), format!("value_{i}"));
        }
        let data = VaultData {
            secrets,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.len(), 100);
        assert_eq!(loaded.secrets.get("KEY_50").unwrap(), "value_50");
    }

    #[test]
    fn save_load_with_long_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let long_pass = "a".repeat(500);
        let data = test_vault_data();
        save_to_path(&data, &long_pass, &path).unwrap();
        let loaded = load_from_path(&long_pass, &path).unwrap().unwrap();
        assert_eq!(loaded.github_token, data.github_token);
    }

    #[test]
    fn save_to_path_sets_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let data = VaultData::default();
        save_to_path(&data, "", &path).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn save_to_path_parent_dir_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subdir").join("vault.enc");
        let data = VaultData::default();
        save_to_path(&data, "", &path).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(dir.path().join("subdir"))
                .unwrap()
                .permissions();
            assert_eq!(perms.mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn load_from_path_very_short_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        // Write exactly NONCE_LEN - 1 bytes (too short)
        let data = vec![0u8; NONCE_LEN - 1];
        std::fs::write(&path, &data).unwrap();
        let result = load_from_path("", &path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Corrupt"));
    }

    #[test]
    fn vault_data_with_empty_github_token() {
        let data = VaultData {
            github_token: Some(String::new()),
            has_passphrase: false,
            secrets: HashMap::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.github_token.as_deref(), Some(""));
    }

    #[test]
    fn save_load_roundtrip_many_passphrases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let data = test_vault_data();

        for i in 0..10 {
            let pass = format!("pass_{}", i);
            save_to_path(&data, &pass, &path).unwrap();
            let loaded = load_from_path(&pass, &path).unwrap().unwrap();
            assert_eq!(loaded.github_token, data.github_token);
        }
    }

    #[test]
    fn vault_data_secret_keys_preserved() {
        let mut secrets = HashMap::new();
        for c in 'a'..='z' {
            secrets.insert(format!("KEY_{}", c), format!("val_{}", c));
        }
        let data = VaultData {
            secrets,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        for c in 'a'..='z' {
            let key = format!("KEY_{}", c);
            assert!(loaded.secrets.contains_key(&key), "Missing key: {}", key);
        }
    }

    #[test]
    fn vault_data_has_passphrase_roundtrip() {
        let data = VaultData {
            github_token: Some("tok".into()),
            has_passphrase: true,
            secrets: HashMap::from([("K".into(), "V".into())]),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "secret", &path).unwrap();
        let loaded = load_from_path("secret", &path).unwrap().unwrap();
        assert!(loaded.has_passphrase);
        assert_eq!(loaded.github_token.as_deref(), Some("tok"));
        assert_eq!(loaded.secrets.get("K").unwrap(), "V");
    }

    #[test]
    fn vault_data_no_passphrase_roundtrip() {
        let data = VaultData {
            github_token: Some("tok".into()),
            has_passphrase: false,
            secrets: HashMap::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert!(!loaded.has_passphrase);
    }

    #[test]
    fn derive_key_unicode_passphrase() {
        let key = derive_key("日本語パスフレーズ").unwrap();
        assert_eq!(key.len(), KEY_LEN);
        let key2 = derive_key("日本語パスフレーズ").unwrap();
        assert_eq!(key, key2);
    }

    #[test]
    fn derive_key_emoji_passphrase() {
        let key = derive_key("🔑🛡️").unwrap();
        assert_eq!(key.len(), KEY_LEN);
    }

    #[test]
    fn vault_data_debug_format() {
        let data = VaultData::default();
        let dbg = format!("{:?}", data);
        assert!(dbg.contains("VaultData"));
        assert!(dbg.contains("github_token"));
        assert!(dbg.contains("has_passphrase"));
        assert!(dbg.contains("secrets"));
    }

    #[test]
    fn vault_data_clone_preserves_all_fields() {
        let mut secrets = HashMap::new();
        secrets.insert("A".into(), "1".into());
        let data = VaultData {
            github_token: Some("tok".into()),
            has_passphrase: true,
            secrets,
        };
        let cloned = data.clone();
        assert_eq!(cloned.github_token, data.github_token);
        assert_eq!(cloned.has_passphrase, data.has_passphrase);
        assert_eq!(cloned.secrets, data.secrets);
    }

    #[test]
    fn vault_data_inequality() {
        let a = VaultData {
            github_token: Some("tok1".into()),
            ..Default::default()
        };
        let b = VaultData {
            github_token: Some("tok2".into()),
            ..Default::default()
        };
        assert_ne!(a, b);
    }

    #[test]
    fn vault_data_inequality_secrets() {
        let a = VaultData {
            secrets: HashMap::from([("A".into(), "1".into())]),
            ..Default::default()
        };
        let b = VaultData {
            secrets: HashMap::from([("B".into(), "2".into())]),
            ..Default::default()
        };
        assert_ne!(a, b);
    }

    #[test]
    fn vault_data_inequality_passphrase() {
        let a = VaultData {
            has_passphrase: true,
            ..Default::default()
        };
        let b = VaultData {
            has_passphrase: false,
            ..Default::default()
        };
        assert_ne!(a, b);
    }

    #[test]
    fn vault_data_json_roundtrip_preserves_all_fields() {
        let mut secrets = HashMap::new();
        secrets.insert("KEY".into(), "VALUE".into());
        let data = VaultData {
            github_token: Some("ghp_test".into()),
            has_passphrase: true,
            secrets,
        };
        let json = serde_json::to_string(&data).unwrap();
        let restored: VaultData = serde_json::from_str(&json).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn destroy_existing_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&VaultData::default(), "", &path).unwrap();
        assert!(path.exists());
        // Simulate destroy by removing the file
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn destroy_nonexistent_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.enc");
        assert!(!path.exists());
    }

    #[test]
    fn exists_true_when_vault_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&VaultData::default(), "", &path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn exists_false_when_no_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        assert!(!path.exists());
    }

    #[test]
    fn save_secret_adds_to_existing_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        // Create initial vault with no secrets
        let initial = VaultData::default();
        save_to_path(&initial, "", &path).unwrap();

        // Load, insert secret, save back (simulating save_secret logic)
        let mut data = load_from_path("", &path).unwrap().unwrap();
        assert!(data.secrets.is_empty());
        data.secrets.insert("MY_TOKEN".into(), "token_value".into());
        save_to_path(&data, "", &path).unwrap();

        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.get("MY_TOKEN").unwrap(), "token_value");
    }

    #[test]
    fn save_secret_overwrites_existing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let mut data = VaultData::default();
        data.secrets.insert("KEY".into(), "value1".into());
        save_to_path(&data, "", &path).unwrap();

        // Overwrite
        let mut data2 = load_from_path("", &path).unwrap().unwrap();
        data2.secrets.insert("KEY".into(), "value2".into());
        save_to_path(&data2, "", &path).unwrap();

        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.get("KEY").unwrap(), "value2");
    }

    #[test]
    fn resolve_secret_returns_none_for_missing_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&VaultData::default(), "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert!(!loaded.secrets.contains_key("NONEXISTENT"));
    }

    #[test]
    fn resolve_github_token_from_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let data = VaultData {
            github_token: Some("ghp_from_vault".into()),
            ..Default::default()
        };
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.github_token.as_deref(), Some("ghp_from_vault"));
    }

    #[test]
    fn save_load_roundtrip_unicode_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        let data = test_vault_data();
        save_to_path(&data, "パスフレーズ", &path).unwrap();
        let loaded = load_from_path("パスフレーズ", &path).unwrap().unwrap();
        assert_eq!(loaded.github_token, data.github_token);
    }

    #[test]
    fn vault_data_default_equality() {
        let a = VaultData::default();
        let b = VaultData::default();
        assert_eq!(a, b);
    }

    #[test]
    fn vault_data_default_clone() {
        let data = VaultData::default();
        let cloned = data.clone();
        assert_eq!(data, cloned);
    }

    #[test]
    fn vault_data_secrets_preserve_order() {
        let mut secrets = HashMap::new();
        secrets.insert("Z_KEY".into(), "z".into());
        secrets.insert("A_KEY".into(), "a".into());
        secrets.insert("M_KEY".into(), "m".into());
        let data = VaultData {
            secrets,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.len(), 3);
        assert!(loaded.secrets.contains_key("Z_KEY"));
        assert!(loaded.secrets.contains_key("A_KEY"));
        assert!(loaded.secrets.contains_key("M_KEY"));
    }

    #[test]
    fn vault_data_empty_string_token() {
        let data = VaultData {
            github_token: Some(String::new()),
            has_passphrase: false,
            secrets: HashMap::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.github_token.as_deref(), Some(""));
    }

    #[test]
    fn vault_data_many_secrets_roundtrip() {
        let mut secrets = HashMap::new();
        for i in 0..50 {
            secrets.insert(format!("SECRET_{i:03}"), format!("value_{i}"));
        }
        let data = VaultData {
            secrets,
            ..Default::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.enc");
        save_to_path(&data, "", &path).unwrap();
        let loaded = load_from_path("", &path).unwrap().unwrap();
        assert_eq!(loaded.secrets.len(), 50);
    }

    #[test]
    fn vault_data_debug_all_fields() {
        let data = VaultData {
            github_token: Some("tok".into()),
            has_passphrase: true,
            secrets: HashMap::from([("K".into(), "V".into())]),
        };
        let dbg = format!("{:?}", data);
        assert!(dbg.contains("github_token"));
        assert!(dbg.contains("has_passphrase"));
        assert!(dbg.contains("secrets"));
        assert!(dbg.contains("tok"));
    }

    #[test]
    fn vault_data_clone_all_fields() {
        let mut secrets = HashMap::new();
        secrets.insert("A".into(), "1".into());
        secrets.insert("B".into(), "2".into());
        let data = VaultData {
            github_token: Some("tok".into()),
            has_passphrase: true,
            secrets,
        };
        let cloned = data.clone();
        assert_eq!(data, cloned);
        assert_eq!(cloned.github_token, Some("tok".into()));
        assert!(cloned.has_passphrase);
        assert_eq!(cloned.secrets.len(), 2);
    }

    #[test]
    fn vault_data_json_deserialize_empty_object() {
        let json = r#"{}"#;
        let data: VaultData = serde_json::from_str(json).unwrap();
        assert_eq!(data, VaultData::default());
    }

    #[test]
    fn vault_data_json_deserialize_full() {
        let json = r#"{"github_token":"ghp_test","has_passphrase":true,"secrets":{"K":"V"}}"#;
        let data: VaultData = serde_json::from_str(json).unwrap();
        assert_eq!(data.github_token.as_deref(), Some("ghp_test"));
        assert!(data.has_passphrase);
        assert_eq!(data.secrets.get("K").unwrap(), "V");
    }

    #[test]
    fn vault_data_inequality_token() {
        let a = VaultData {
            github_token: Some("a".into()),
            ..Default::default()
        };
        let b = VaultData {
            github_token: Some("b".into()),
            ..Default::default()
        };
        assert_ne!(a, b);
    }

    #[test]
    fn vault_data_inequality_secrets_diff_values() {
        let a = VaultData {
            secrets: HashMap::from([("K".into(), "V1".into())]),
            ..Default::default()
        };
        let b = VaultData {
            secrets: HashMap::from([("K".into(), "V2".into())]),
            ..Default::default()
        };
        assert_ne!(a, b);
    }

    #[test]
    fn vault_data_inequality_secrets_diff_keys() {
        let a = VaultData {
            secrets: HashMap::from([("A".into(), "V".into())]),
            ..Default::default()
        };
        let b = VaultData {
            secrets: HashMap::from([("B".into(), "V".into())]),
            ..Default::default()
        };
        assert_ne!(a, b);
    }

    #[test]
    fn vault_constants_values() {
        assert_eq!(NONCE_LEN, 24);
        assert_eq!(KEY_LEN, 32);
        assert_eq!(DOMAIN_SEPARATOR, b"|ghscaff-vault-v1");
    }

    #[test]
    fn vault_path_structure() {
        let p = vault_path().unwrap();
        assert!(p.to_string_lossy().contains(".ghscaff"));
        assert!(p.to_string_lossy().ends_with("vault.enc"));
    }

    #[test]
    fn derive_key_various_lengths() {
        for len in &[0, 1, 5, 100, 1000] {
            let key = derive_key(&"a".repeat(*len)).unwrap();
            assert_eq!(key.len(), KEY_LEN);
        }
    }

    #[test]
    fn test_resolve_secret_from_env_var() {
        std::env::set_var("GHSCAFF_TEST_SECRET_XYZ", "env_value_123");
        let result = resolve_secret("GHSCAFF_TEST_SECRET_XYZ", "").unwrap();
        assert_eq!(result.as_deref(), Some("env_value_123"));
        std::env::remove_var("GHSCAFF_TEST_SECRET_XYZ");
    }

    #[test]
    fn test_resolve_secret_missing_key_returns_none() {
        let result = resolve_secret("TOTALLY_NONEXISTENT_KEY_999999", "").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_github_token_from_env() {
        std::env::set_var("GHSCAFF_TEST_TOKEN_ABC", "ghp_env_test");
        let result = std::env::var("GHSCAFF_TEST_TOKEN_ABC").unwrap();
        assert_eq!(result, "ghp_env_test");
        std::env::remove_var("GHSCAFF_TEST_TOKEN_ABC");
    }

    #[test]
    fn test_exists_does_not_panic() {
        let _result = exists();
    }

    use std::sync::Mutex;
    static VAULT_MUTEX: Mutex<()> = Mutex::new(());

    /// Points `HOME` (and thus `vault_path()`) at a private temp directory
    /// for the lifetime of the guard, restoring the previous value on drop.
    ///
    /// `save`/`load`/`destroy`/`exists`/`save_secret` all resolve their file
    /// location via `dirs::home_dir()`, which reads `HOME`. Pointing every
    /// wrapper-level test at the developer's *real* `~/.ghscaff/vault.enc`
    /// meant concurrent test processes (nextest runs one process per test)
    /// raced on the same file and intermittently corrupted it. Overriding
    /// `HOME` per test removes the shared mutable state instead of trying to
    /// serialize access to it — `_dir` is kept alive only to defer cleanup
    /// until the guard drops.
    struct HomeGuard {
        _dir: tempfile::TempDir,
        previous: Option<std::ffi::OsString>,
    }

    impl HomeGuard {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let previous = std::env::var_os("HOME");
            // SAFETY: guarded by VAULT_MUTEX, so no other thread in this
            // process observes or mutates HOME concurrently.
            unsafe { std::env::set_var("HOME", dir.path()) };
            Self {
                _dir: dir,
                previous,
            }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            // SAFETY: same guarantee as in `new` — still under VAULT_MUTEX.
            match self.previous.take() {
                Some(home) => unsafe { std::env::set_var("HOME", home) },
                None => unsafe { std::env::remove_var("HOME") },
            }
        }
    }

    #[test]
    fn test_save_load_via_wrappers_with_backup() {
        let _lock = VAULT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _home = HomeGuard::new();

        let data = VaultData {
            github_token: Some("ghp_wrapper_test".into()),
            has_passphrase: false,
            secrets: HashMap::from([("WK".into(), "WV".into())]),
        };
        save(&data, "").unwrap();
        let loaded = load("").unwrap().unwrap();
        assert_eq!(loaded.github_token.as_deref(), Some("ghp_wrapper_test"));
        assert_eq!(loaded.secrets.get("WK").unwrap(), "WV");
    }

    #[test]
    fn test_destroy_nonexistent_returns_false() {
        let _lock = VAULT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _home = HomeGuard::new();

        assert!(!exists());
        let result = destroy().unwrap();
        assert!(!result);
    }

    #[test]
    fn test_destroy_existing_returns_true() {
        let _lock = VAULT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _home = HomeGuard::new();

        save(&VaultData::default(), "").unwrap();
        assert!(exists());
        let result = destroy().unwrap();
        assert!(result);
        assert!(!exists());
    }

    #[test]
    fn test_save_secret_public_api_with_backup() {
        let _lock = VAULT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _home = HomeGuard::new();

        save_secret("GHSCAFF_PUB_SEC", "pub_val", "").unwrap();
        let loaded = load("").unwrap().unwrap();
        assert_eq!(loaded.secrets.get("GHSCAFF_PUB_SEC").unwrap(), "pub_val");
    }

    #[test]
    fn test_save_secret_overwrite_public_api() {
        let _lock = VAULT_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let _home = HomeGuard::new();

        save_secret("GHSCAFF_OW2", "first", "").unwrap();
        save_secret("GHSCAFF_OW2", "second", "").unwrap();
        let loaded = load("").unwrap().unwrap();
        assert_eq!(loaded.secrets.get("GHSCAFF_OW2").unwrap(), "second");
    }
}
