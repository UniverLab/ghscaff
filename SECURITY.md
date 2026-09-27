# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| 0.5.x   | :white_check_mark: |
| < 0.5   | :x:                |

## Reporting a Vulnerability

If you discover a security vulnerability in ghscaff, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, email: **jheison.mb@univerlab.org**

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

You should receive an initial response within **72 hours**. We will work with you to understand and address the issue before any public disclosure.

## Security Architecture

### Encrypted Vault

ghscaff stores GitHub tokens and template secrets in an encrypted local vault at `~/.ghscaff/vault.enc`.

| Property | Detail |
|----------|--------|
| **Cipher** | XSalsa20-Poly1305 (authenticated encryption) |
| **Key derivation** | Blake2b-256(username \|\| hostname \|\| passphrase \|\| domain) |
| **Nonce** | 24 bytes, cryptographically random per write |
| **File permissions** | `0600` (owner read/write only) |
| **Directory permissions** | `0700` (owner access only) |
| **Write method** | Atomic (temp file + rename) |

#### Key derivation factors

The encryption key is derived from four factors:

| Factor | Purpose |
|--------|---------|
| OS username | Only your user account can decrypt |
| Hostname | Vault cannot be decrypted on a different machine |
| Passphrase (optional) | Extra protection if the attacker has local access |
| Domain separator | Prevents cross-application key reuse |

**Note:** The binary path was intentionally removed from key derivation to allow the vault to survive binary relocation (e.g., after `cargo install` or self-update).

#### What this protects against

- **Theft of `~/.ghscaff/vault.enc`** — ciphertext is useless without the key factors
- **Copying vault to another machine** — hostname mismatch prevents decryption
- **Other users on the same machine** — file permissions restrict access
- **Nonce reuse** — fresh random nonce generated on every write
- **Tampering** — Poly1305 authentication tag detects any modification

#### What this does NOT protect against

- **Root access** — root can read any file regardless of permissions
- **Keyloggers / memory dumps** — if the passphrase is captured at input time
- **Compromised OS** — if the attacker controls the machine, they control the vault
- **Weak passphrases** — the tool warns but allows passphrases under 5 characters

### GitHub Actions Secrets

When configuring repository secrets, ghscaff uses NaCl sealed box encryption (libsodium-compatible):

| Property | Detail |
|----------|--------|
| **Algorithm** | crypto_box_seal (X25519 + XSalsa20-Poly1305) |
| **Ephemeral key** | Generated per-secret, never stored |
| **Nonce** | Blake2b-24(ephemeral_pk \|\| recipient_pk) |
| **Recipient key** | Fetched from GitHub API per-repo |

### Token Resolution Order

1. `GITHUB_TOKEN` environment variable (for CI/CD)
2. Encrypted vault (`~/.ghscaff/vault.enc`)
3. Interactive prompt (first run only, saves to vault)

The tool never prints, logs, or transmits tokens. Token input uses `inquire::Password` which disables terminal echo.

### Network Communications

- All GitHub API calls use HTTPS (via `rustls-tls`)
- User-Agent is `ghscaff` for GitHub API calls and `ghscaff-update` for update paths (no identifying information beyond the tool name)
- The startup notice fetches the release list (`GET /repos/UniverLab/ghscaff/releases?per_page=100`) and silently does nothing on any failure; it can be disabled with `GHSCAFF_NO_UPDATE_CHECK=1` (which never affects the explicit `ghscaff update` command). Only the explicit command downloads release assets (`https://github.com/UniverLab/ghscaff/releases/download/{tag}/{asset}` plus `SHA256SUMS.txt`).

### Self-Update Mechanism

Updating is always explicit: only `ghscaff update` downloads anything, and only after consent (prompt defaults to NO; `--yes` skips it). It selects the newest stable release strictly newer than the running binary (drafts, prereleases and non-semver tags excluded), downloads the `ghscaff-{tag}-{arch}-{os}.tar.gz` asset for the current target, verifies its SHA256 against the release's `SHA256SUMS.txt` when present (missing file skips verification; missing entry or mismatch aborts), then atomically replaces the running binary (`std::env::current_exe()`, same-directory temp file plus rename). Binaries under `~/.cargo/bin` are refused with `cargo install --force ghscaff`. The update never touches `~/.ghscaff` state. `ghscaff update --check` only reports (exit 1 = available, 0 = current) and downloads nothing.

**Mitigation:** The update only runs via the explicit `ghscaff update` command with user consent (default NO); the startup notice never prompts and never installs.

## Best Practices for Users

1. **Use a scoped token** — grant only `repo`, `workflow`, and optionally `read:org`. Do not use a token with `admin:org` or `delete_repo` unless necessary.

2. **Enable the optional passphrase** — it adds a factor that protects the vault even if someone gains read access to `~/.ghscaff/`.

3. **Use a strong passphrase** — the tool warns about passphrases under 5 characters. Use a unique, random passphrase for maximum protection.

4. **Restrict vault directory permissions** — the tool sets `0700` on the directory, but verify after manual operations:
   ```bash
   chmod 700 ~/.ghscaff
   chmod 600 ~/.ghscaff/vault.enc
   ```

5. **Rotate tokens periodically** — if you suspect compromise, run `ghscaff config` to wipe the vault and start fresh with a new token.

6. **Disable update checks in CI** — set `GHSCAFF_NO_UPDATE_CHECK=1` in automated environments.

7. **Audit token scopes** — review your token at https://github.com/settings/tokens regularly and revoke unused tokens.

## Cryptographic Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `xsalsa20poly1305` | 0.9 | Vault encryption (XSalsa20-Poly1305) |
| `crypto_box` | 0.9 | GitHub secrets encryption (NaCl sealed boxes) |
| `blake2` | 0.10 | Key derivation (Blake2b-256) |

All three are well-audited crates from the RustCrypto project.

## Scope

This security policy applies to the `ghscaff` CLI tool published on [crates.io](https://crates.io/crates/ghscaff) and the [GitHub releases](https://github.com/UniverLab/ghscaff/releases). It does not apply to third-party forks or modifications.
