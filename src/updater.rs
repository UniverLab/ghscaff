//! Explicit self-update + read-only release notice (CM34): always asks,
//! never installs on its own. Never touches `~/.ghscaff` state — the vault
//! and the boilerplate cache are not the binary.
//!
//! Only [`run_update`] — the explicit `ghscaff update` command — downloads
//! and replaces the running binary, and only after consent (default **NO**,
//! `--yes` skips the prompt). The startup notice in `main` is a silent,
//! read-only lookup that prints one line and returns. Every external fact
//! (release list, archive bytes, prompt answer, executable path, target) is
//! injected through [`UpdateDeps`], so the unit tests below never touch the
//! network.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const GITHUB_REPO: &str = "UniverLab/ghscaff";

/// The exact remediation printed when the running binary lives below
/// `~/.cargo/bin`: cargo owns that file, so ghscaff refuses to replace it.
pub const CARGO_INSTALL_HINT: &str = "cargo install --force ghscaff";

// ── Release lookup seams ────────────────────────────────────────

/// The release fields needed to select a stable, published binary.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct GitHubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
}

/// Injectable release-JSON lookup used by the update core and the notice.
pub trait ReleaseFetcher {
    fn get(&self, url: &str) -> Result<String>;
}

/// Production release lookup. All network and HTTP-status handling lives
/// behind [`ReleaseFetcher`] so unit tests can use a deterministic fake.
///
/// The notice path carries a short timeout (it runs before every
/// subcommand); the explicit command uses [`RealFetcher::new`] with no
/// timeout so a slow API never masquerades as a failure — errors there are
/// loud anyway.
pub struct RealFetcher {
    timeout: Option<Duration>,
}

impl RealFetcher {
    /// No request timeout: used by the explicit `ghscaff update` command.
    pub fn new() -> Self {
        Self { timeout: None }
    }

    /// Bounded request timeout: used by the silent startup notice.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout: Some(timeout),
        }
    }
}

impl Default for RealFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ReleaseFetcher for RealFetcher {
    fn get(&self, url: &str) -> Result<String> {
        let client = match self.timeout {
            Some(timeout) => reqwest::blocking::Client::builder()
                .timeout(timeout)
                .build()
                .context("failed to build HTTP client")?,
            None => reqwest::blocking::Client::new(),
        };
        let response = client
            .get(url)
            .header("User-Agent", "ghscaff-update")
            .send()
            .context("failed to fetch GitHub releases")?;
        let status = response.status();
        if !status.is_success() {
            bail!("GitHub releases request failed: HTTP {status}");
        }
        response
            .text()
            .context("failed to read GitHub releases response")
    }
}

/// Injectable binary downloader. The archive (and `SHA256SUMS.txt`) are
/// decoded only after this seam returns, keeping the updater tests offline.
pub trait BinaryDownloader {
    fn download(&self, url: &str) -> Result<Vec<u8>>;
}

/// Production binary downloader. Deliberately timeout-free: a release
/// archive is a multi-MiB download and a slow link is not a dead link.
pub struct RealDownloader;

impl BinaryDownloader for RealDownloader {
    fn download(&self, url: &str) -> Result<Vec<u8>> {
        let response = reqwest::blocking::Client::new()
            .get(url)
            .header("User-Agent", "ghscaff-update")
            .send()
            .with_context(|| format!("failed to download {url}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("download failed: HTTP {status} for {url}");
        }
        Ok(response
            .bytes()
            .with_context(|| format!("failed to read {url}"))?
            .to_vec())
    }
}

// ── Hermetic dependencies ───────────────────────────────────────

/// Dependencies for the hermetic update command path. The real command uses
/// the same flow with production I/O; tests provide every fallible external
/// fact here and never contact GitHub or write outside the temp dirs they
/// own.
pub struct UpdateDeps<'a> {
    /// Current version tag, `v`-prefixed (e.g. `v0.6.0`).
    pub current: &'a str,
    pub releases: std::result::Result<Vec<GitHubRelease>, String>,
    pub exe: &'a Path,
    /// Cargo install root; `None` skips the cargo guard entirely.
    pub cargo_bin: Option<&'a Path>,
    pub target: std::result::Result<(&'static str, &'static str), String>,
    pub downloader: &'a dyn BinaryDownloader,
    pub confirm: &'a dyn Fn() -> bool,
}

// ── Public update entry points ───────────────────────────────────

/// Check for and, after consent, install the latest stable release.
///
/// The returned integer is the process exit code: `0` means no update was
/// installed (already current, a declined prompt, or a cargo-managed
/// install) and `1` means an update was available in `--check` mode.
/// Network and API errors surface as `Err` — the explicit command fails
/// loudly, unlike the silent startup notice.
pub fn run_update(check: bool, yes: bool) -> Result<i32> {
    let current = current_version();
    let releases = fetch_releases_with(&RealFetcher::new())?;

    // The first pass is limited to the network result. The hermetic core
    // below owns all output and consent, so `--check` cannot touch a local
    // path or the target before it returns.
    let latest = select_latest_stable(&releases, &current);
    if latest.is_none() || check {
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(releases),
            exe: Path::new("/tmp/ghscaff-update-test/ghscaff"),
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &RealDownloader,
            confirm: &|| false,
        };
        return run_update_with(check, yes, &deps);
    }

    // An actual install needs the executable and target facts. Resolve them
    // only after the read-only pass established that a newer release exists.
    let latest = latest.expect("newer release was established above");
    let exe = std::env::current_exe().context("failed to locate ghscaff executable")?;
    let cargo_bin = cargo_install_root();
    // Resolved eagerly but *carried* as a result: the hermetic core decides
    // when it surfaces, so the cargo guard still wins on a cargo-managed
    // install and an unsupported target fails loudly before any download.
    let target = detect_platform().map_err(|error| {
        if cfg!(target_os = "windows") {
            format!(
                "ghscaff update does not support windows — run scripts/install.ps1 instead \
                 (release asset: ghscaff-{latest}-x86_64-pc-windows-msvc.zip)"
            )
        } else {
            error.to_string()
        }
    });
    let deps = UpdateDeps {
        current: &current,
        releases: Ok(releases),
        exe: &exe,
        cargo_bin: cargo_bin.as_deref(),
        target,
        downloader: &RealDownloader,
        confirm: &|| {
            inquire::Confirm::new(&format!("Update to {latest}? [y/N]"))
                .with_default(false)
                .prompt()
                .unwrap_or(false)
        },
    };
    run_update_with(false, yes, &deps)
}

/// Hermetic update flow used by unit tests and embedders. It has no
/// network or filesystem setup step; callers provide those facts through
/// [`UpdateDeps`]. Exit codes follow the contract of [`run_update`].
pub fn run_update_with(check: bool, yes: bool, deps: &UpdateDeps<'_>) -> Result<i32> {
    let releases = deps
        .releases
        .as_ref()
        .map_err(|error| anyhow!("release lookup failed: {error}"))?;
    let Some(latest) = select_latest_stable(releases, deps.current) else {
        println!("ghscaff {} is up to date", deps.current);
        return Ok(0);
    };
    println!("ghscaff {} → {latest}", deps.current);

    // `--check` ends here: exit 1 = update available, 0 = already current.
    // Nothing below this line — cargo guard, target, prompt, download — may
    // run in read-only mode.
    if check {
        return Ok(1);
    }

    // Cargo owns `~/.cargo/bin`; refuse before anything is downloaded.
    if deps
        .cargo_bin
        .is_some_and(|root| is_cargo_managed_with_root(deps.exe, Some(root.to_path_buf())))
    {
        println!("{CARGO_INSTALL_HINT}");
        return Ok(0);
    }

    let (arch, os) = *deps
        .target
        .as_ref()
        .map_err(|error| anyhow!("target resolution failed: {error}"))?;

    if !yes && !(deps.confirm)() {
        println!("Aborted.");
        return Ok(0);
    }

    let exe = deps.exe;
    let dir = exe
        .parent()
        .context("executable path has no parent directory")?;
    let file_name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("ghscaff");
    let tmp_path = dir.join(format!(".{file_name}.update"));

    update_binary_at(&tmp_path, exe, |out| {
        download_verify_extract_with(deps.downloader, &latest, (arch, os), out)
    })?;

    println!("  ✓ Updated ghscaff to {latest} — restart your terminal to use it.");
    Ok(0)
}

// ── Version helpers ─────────────────────────────────────────────

/// The `v`-prefixed tag of the running binary, e.g. `v0.6.0`.
pub fn current_version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// A tag is "stable" when it carries nothing but digits and dots after an
/// optional `v` — anything prerelease-shaped (`v1.0.0-rc1`) is skipped.
pub fn is_stable_version(tag: &str) -> bool {
    let value = tag.trim_start_matches('v');
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Numeric, `v`-insensitive component-wise comparison; a missing component
/// counts as `0`, so `1.0` equals `1.0.0`.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u32> {
        s.trim_start_matches('v')
            .split('.')
            .filter_map(|part| part.parse().ok())
            .collect()
    };
    let (pa, pb) = (parse(a), parse(b));
    let len = pa.len().max(pb.len());
    for index in 0..len {
        let comparison = pa
            .get(index)
            .copied()
            .unwrap_or(0)
            .cmp(&pb.get(index).copied().unwrap_or(0));
        if comparison != std::cmp::Ordering::Equal {
            return comparison;
        }
    }
    std::cmp::Ordering::Equal
}

/// Select the newest published stable release strictly newer than `current`.
/// Drafts, prereleases, and non-semver tags never win — and neither notice
/// nor command can ever disagree, because both call this.
pub fn select_latest_stable(releases: &[GitHubRelease], current: &str) -> Option<String> {
    releases
        .iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter(|release| is_stable_version(&release.tag_name))
        .filter(|release| compare_versions(&release.tag_name, current).is_gt())
        .max_by(|a, b| compare_versions(&a.tag_name, &b.tag_name))
        .map(|release| release.tag_name.clone())
}

// ── Release and asset URLs ──────────────────────────────────────

/// Full release list (not `/releases/latest`) so drafts, prereleases, and
/// non-semver tags can be filtered in code; `per_page=100` keeps the default
/// page-30 cut-off from hiding a stable tag.
fn releases_url() -> String {
    format!("https://api.github.com/repos/{GITHUB_REPO}/releases?per_page=100")
}

/// The exact release asset name, matching `.github/workflows/release.yml`
/// (delegating to `rust-release.yml`) and `scripts/install.sh`:
/// `ghscaff-{tag}-{arch}-{os}.tar.gz`, tag keeping its leading `v`.
pub fn asset_name(tag: &str, arch: &str, os: &str) -> String {
    format!("ghscaff-{tag}-{arch}-{os}.tar.gz")
}

fn asset_url(tag: &str, (arch, os): (&str, &str)) -> String {
    format!(
        "https://github.com/{GITHUB_REPO}/releases/download/{tag}/{}",
        asset_name(tag, arch, os)
    )
}

/// `SHA256SUMS.txt` uploaded next to every release asset.
pub fn sums_url(tag: &str) -> String {
    format!("https://github.com/{GITHUB_REPO}/releases/download/{tag}/SHA256SUMS.txt")
}

/// Fetch the release list through an injected fetcher.
pub fn fetch_releases_with(fetcher: &dyn ReleaseFetcher) -> Result<Vec<GitHubRelease>> {
    let body = fetcher.get(&releases_url())?;
    serde_json::from_str(&body).context("failed to parse releases JSON")
}

/// Fetch and select the newest stable release strictly newer than `current`.
pub fn fetch_latest_stable_with(
    fetcher: &dyn ReleaseFetcher,
    current: &str,
) -> Result<Option<String>> {
    let releases = fetch_releases_with(fetcher)?;
    Ok(select_latest_stable(&releases, current))
}

/// Silent startup notice: any network or parse failure is a `None`, never an
/// error. The explicit command is the only loud path.
pub fn check_notice(fetcher: &dyn ReleaseFetcher, current: &str) -> Option<String> {
    fetch_latest_stable_with(fetcher, current).ok().flatten()
}

// ── Download, verification, extraction ──────────────────────────

/// Outcome of the `SHA256SUMS.txt` check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Check {
    /// The release shipped a checksum line for this asset and it matched.
    Verified,
    /// The release ships no `SHA256SUMS.txt` — verification skipped.
    Skipped,
}

/// Verify the downloaded archive against the release's `SHA256SUMS.txt`.
///
/// The rule mirrors `scripts/install.sh`: a missing sums file (fetch fails)
/// skips verification, a sums file present but with **no line for this
/// asset** aborts, and a mismatched digest aborts. The archive bytes are
/// hashed whole — this runs before anything is written next to the exe.
pub fn verify_checksum(
    downloader: &dyn BinaryDownloader,
    tag: &str,
    asset: &str,
    bytes: &[u8],
) -> Result<Check> {
    let Ok(sums) = downloader.download(&sums_url(tag)) else {
        return Ok(Check::Skipped);
    };

    let text = String::from_utf8_lossy(&sums);
    let line = text
        .lines()
        .find(|line| line.split_whitespace().nth(1) == Some(asset))
        .filter(|line| !line.split_whitespace().next().unwrap_or("").is_empty())
        .with_context(|| format!("no checksum listed for {asset}"))?;

    let expected = line.split_whitespace().next().unwrap_or("");

    use sha2::{Digest, Sha256};
    let actual: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !actual.eq_ignore_ascii_case(expected) {
        bail!("SHA256 mismatch for {asset}: expected {expected}, got {actual}");
    }
    Ok(Check::Verified)
}

/// Download the release asset for `tag`/`target`, verify it when the release
/// ships checksums, and unpack the `ghscaff` entry into `output` — which is
/// the same-directory staging file, never the running binary itself.
pub fn download_verify_extract_with(
    downloader: &dyn BinaryDownloader,
    tag: &str,
    target: (&str, &str),
    output: &Path,
) -> Result<()> {
    let asset = asset_name(tag, target.0, target.1);
    let bytes = downloader.download(&asset_url(tag, target))?;
    if verify_checksum(downloader, tag, &asset, &bytes)? == Check::Skipped {
        println!("  ℹ checksum skipped (release ships no SHA256SUMS.txt)");
    }
    extract_binary(std::io::Cursor::new(bytes), output)
}

// ── Cargo-managed detection ─────────────────────────────────────

fn resolve_cargo_root(
    install_root_env: Option<String>,
    cargo_home_env: Option<String>,
    home_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(root) = install_root_env.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(root));
    }
    if let Some(home) = cargo_home_env.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(home));
    }
    home_dir.map(|h| h.join(".cargo"))
}

fn cargo_install_root() -> Option<PathBuf> {
    resolve_cargo_root(
        std::env::var("CARGO_INSTALL_ROOT").ok(),
        std::env::var("CARGO_HOME").ok(),
        dirs::home_dir(),
    )
}

/// Both paths are canonicalized; an uncanonicalizable path (missing exe, an
/// unresolvable root) is *not* cargo-managed rather than guessed.
fn is_cargo_managed_with_root(exe_path: &Path, root: Option<PathBuf>) -> bool {
    let Some(root) = root else {
        return false;
    };
    let bin_dir = root.join("bin");
    match (exe_path.canonicalize(), bin_dir.canonicalize()) {
        (Ok(exe), Ok(bin)) => exe.starts_with(bin),
        _ => false,
    }
}

#[cfg(test)]
fn is_cargo_managed(exe_path: &Path) -> bool {
    is_cargo_managed_with_root(exe_path, cargo_install_root())
}

// ── Download + replace ──────────────────────────────────────────

/// Fetches into `tmp_path` via `fetch`, makes it executable, and atomically
/// replaces `target` with it. On any failure, `tmp_path` is removed and
/// `target` is left untouched.
fn update_binary_at(
    tmp_path: &Path,
    target: &Path,
    fetch: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let result = (|| -> Result<()> {
        fetch(tmp_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(tmp_path, std::fs::Permissions::from_mode(0o755))
                .with_context(|| {
                    format!(
                        "failed to set executable permission on {}",
                        tmp_path.display()
                    )
                })?;
        }

        replace_binary(tmp_path, target)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(tmp_path);
    }

    result
}

/// Atomic same-directory swap: the staging file is a sibling of the target
/// (`.{name}.update`), so the rename never crosses a filesystem.
fn replace_binary(tmp_path: &Path, target: &Path) -> Result<()> {
    std::fs::rename(tmp_path, target).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            anyhow::anyhow!(
                "permission denied replacing {} — check that the containing directory is writable by the current user",
                target.display()
            )
        } else {
            anyhow::Error::new(e).context(format!("failed to replace {}", target.display()))
        }
    })
}

fn extract_binary(reader: impl std::io::Read, output: &Path) -> Result<()> {
    let decoder = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);

    let mut found = false;
    for entry in archive
        .entries()
        .context("corrupt archive: failed to read entries")?
    {
        let mut entry = entry.context("corrupt archive: failed to read entry")?;
        let path = entry
            .path()
            .context("corrupt archive: invalid entry path")?;
        if path.file_name().is_some_and(|n| n == "ghscaff") {
            entry
                .unpack(output)
                .context("failed to extract binary from archive")?;
            found = true;
            break;
        }
    }

    if !found {
        anyhow::bail!("binary not found in archive");
    }

    let meta = std::fs::metadata(output)
        .with_context(|| format!("failed to stat extracted binary at {}", output.display()))?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(output);
        anyhow::bail!("extracted binary is empty");
    }

    Ok(())
}

/// `(arch, os)` pair for the running binary, matching the release matrix
/// (`rust-release.yml`). Windows ships as a `.zip`, which this updater does
/// not handle: refuse loudly before anything is downloaded.
fn detect_platform() -> Result<(&'static str, &'static str)> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => anyhow::bail!("unsupported architecture: {other}"),
    };
    let os = match std::env::consts::OS {
        "linux" => "unknown-linux-musl",
        "macos" => "apple-darwin",
        other => anyhow::bail!("unsupported OS: {other}"),
    };
    Ok((arch, os))
}

#[cfg(test)]
mod tests;
