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
                 (release asset: ghscaff-{current}-x86_64-pc-windows-msvc.zip)"
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
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;
    use std::sync::{Mutex, OnceLock};

    /// Tests that read or write process-wide environment (only the
    /// state-untouched test does) serialize on this lock; tests run in
    /// parallel by default.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn make_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            for (name, content) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o755);
                header.set_cksum();
                builder.append_data(&mut header, name, *content).unwrap();
            }
            builder.finish().unwrap();
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    /// A fake release tag guaranteed newer than this binary, whatever the
    /// current version is. Hardcoding "the next version" re-traps on every
    /// version bump.
    fn fake_newer_tag() -> String {
        let mut parts = env!("CARGO_PKG_VERSION").split('.');
        let (major, minor, patch) = (
            parts.next().expect("semver major"),
            parts.next().expect("semver minor"),
            parts
                .next()
                .expect("semver patch")
                .parse::<u64>()
                .expect("numeric patch"),
        );
        format!("v{major}.{minor}.{}", patch + 1)
    }

    fn release(tag: &str) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
            prerelease: false,
            draft: false,
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Deterministic release-list fake for the fetcher seam.
    struct FakeFetcher {
        body: std::result::Result<String, String>,
    }

    impl ReleaseFetcher for FakeFetcher {
        fn get(&self, _url: &str) -> Result<String> {
            self.body.clone().map_err(|error| anyhow!(error))
        }
    }

    /// Offline downloader: serves a canned asset archive and a canned
    /// `SHA256SUMS.txt`, recording every URL it is asked for.
    struct RecordingDownloader {
        asset: Vec<u8>,
        sums: std::result::Result<Vec<u8>, String>,
        calls: RefCell<Vec<String>>,
    }

    impl RecordingDownloader {
        fn new(asset: Vec<u8>, sums: std::result::Result<Vec<u8>, String>) -> Self {
            Self {
                asset,
                sums,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl BinaryDownloader for RecordingDownloader {
        fn download(&self, url: &str) -> Result<Vec<u8>> {
            self.calls.borrow_mut().push(url.to_string());
            if url.ends_with("SHA256SUMS.txt") {
                self.sums.clone().map_err(|error| anyhow!(error))
            } else {
                Ok(self.asset.clone())
            }
        }
    }

    fn make_exe(dir: &Path) -> PathBuf {
        let exe = dir.join("ghscaff");
        std::fs::write(&exe, b"old-binary").unwrap();
        exe
    }

    fn tmp_staging_for(exe: &Path) -> PathBuf {
        let name = exe.file_name().and_then(|n| n.to_str()).unwrap();
        exe.parent().unwrap().join(format!(".{name}.update"))
    }

    // ── ①② --check exit codes ───────────────────────────────────

    #[test]
    fn check_mode_exits_1_when_a_newer_release_exists() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&fake_newer_tag()), release(&current)]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| panic!("--check must not prompt"),
        };

        assert_eq!(run_update_with(true, false, &deps).unwrap(), 1);
        assert!(downloader.calls().is_empty(), "--check downloads nothing");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old-binary");
    }

    #[test]
    fn check_mode_exits_0_when_current() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&current)]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| panic!("up to date must not prompt"),
        };

        assert_eq!(run_update_with(true, false, &deps).unwrap(), 0);
        assert!(downloader.calls().is_empty());
    }

    // ── ③④ stable selection ─────────────────────────────────────

    #[test]
    fn select_ignores_drafts_prereleases_and_rc_tags() {
        let current = current_version();
        let newer = fake_newer_tag();
        let releases = vec![
            GitHubRelease {
                tag_name: "v99.0.0".into(),
                prerelease: false,
                draft: true,
            },
            GitHubRelease {
                tag_name: "v98.0.0".into(),
                prerelease: true,
                draft: false,
            },
            release(&format!("{newer}-rc1")),
            release("nightly"),
            release(&current),
        ];

        assert_eq!(select_latest_stable(&releases, &current), None);
    }

    #[test]
    fn select_ignores_higher_drafts_even_in_check_mode() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![
                GitHubRelease {
                    tag_name: "v99.0.0".into(),
                    prerelease: false,
                    draft: true,
                },
                GitHubRelease {
                    tag_name: "v98.0.0".into(),
                    prerelease: true,
                    draft: false,
                },
                release(&format!("{}-rc1", fake_newer_tag())),
                release(&current),
            ]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| panic!("must not prompt"),
        };

        assert_eq!(run_update_with(true, false, &deps).unwrap(), 0);
        assert!(downloader.calls().is_empty());
    }

    #[test]
    fn select_picks_max_stable_from_unsorted_list() {
        let releases = vec![
            release("v0.7.0"),
            release("v0.5.0"),
            release("v0.9.0"),
            release("v0.6.0"),
        ];
        assert_eq!(
            select_latest_stable(&releases, "v0.6.0"),
            Some("v0.9.0".to_string())
        );
    }

    // ── ⑤ version comparison matrix (port of main.rs::is_newer) ─

    /// Old `main.rs::is_newer(current, latest)` semantics, expressed on top
    /// of the single comparator the notice and the command now share.
    fn is_newer(current: &str, latest: &str) -> bool {
        compare_versions(latest, current).is_gt()
    }

    #[test]
    fn compare_versions_orders_by_significance() {
        assert!(is_newer("v0.5.0", "v1.0.0"));
        assert!(is_newer("v0.5.0", "v0.6.0"));
        assert!(is_newer("v0.5.0", "v0.5.1"));
        assert!(!is_newer("v0.5.0", "v0.5.0"));
        assert!(!is_newer("v1.0.0", "v0.9.0"));
        assert!(is_newer("v1.0.0", "v2.0.0"));
        assert!(!is_newer("v2.0.0", "v1.9.0"));
        assert!(!is_newer("v1.0.1", "v1.0.0"));
        assert!(!is_newer("v0.0.0", "v0.0.0"));
        assert!(is_newer("v0.0.1", "v99.99.99"));
    }

    #[test]
    fn compare_versions_treats_missing_components_as_zero() {
        assert!(!is_newer("v1.0.0", "v1.0"));
        assert!(!is_newer("v1.0", "v1.0.0"));
        assert!(!is_newer("v1", "v1"));
        assert!(is_newer("v1", "v2"));
        assert!(!is_newer("v2", "v1"));
        assert!(!is_newer("v1.0.0 ", "v1.0.0"));
        assert!(!is_newer("v1.0.0", " v1.0.0"));
        assert!(is_newer("v1.0.0.0", "v1.0.1.0"));
        assert!(!is_newer("v1..0", "v1..0"));
        assert!(!is_newer("v01.00.00", "v1.0.0"));
    }

    #[test]
    fn compare_versions_ignores_v_prefix_everywhere() {
        assert!(is_newer("0.5.0", "0.6.0"));
        assert!(is_newer("v0.5.0", "1.0.0"));
        assert!(is_newer("1.0.0", "2.0.0"));
        assert!(!is_newer("2.0.0", "1.0.0"));
        assert!(is_newer("v1.0.0", "2.0.0"));
        assert!(is_newer("1.0.0", "v2.0.0"));
    }

    #[test]
    fn compare_versions_handles_non_numeric_input() {
        assert!(!is_newer("abc", "xyz"));
        assert!(!is_newer("", ""));
        assert!(!is_newer("vabc", "vdef"));
        assert!(!is_newer("latest", "stable"));
    }

    #[test]
    fn compare_versions_component_precedence() {
        assert!(is_newer("v1.2.3", "v2.3.4"));
        assert!(!is_newer("v2.3.4", "v1.2.3"));
        assert!(is_newer("v1.1.0", "v1.1.1"));
        assert!(!is_newer("v1.1.1", "v1.1.0"));
        assert!(is_newer("v1.1.0", "v1.2.0"));
        assert!(!is_newer("v1.2.0", "v1.1.0"));
        assert!(is_newer("v1.9.9", "v2.0.0"));
        assert!(!is_newer("v2.0.0", "v1.9.9"));
        assert!(is_newer("v1.1.9", "v1.2.0"));
        assert!(!is_newer("v1.2.0", "v1.1.9"));
        assert!(is_newer("v9.0.0", "v10.0.0"));
        assert!(is_newer("v1.99.0", "v1.100.0"));
        assert!(is_newer("v1.0.999", "v1.0.1000"));
        assert!(is_newer("v999.999.999", "v1000.0.0"));
    }

    #[test]
    fn compare_versions_boundaries() {
        assert!(is_newer("v0.0.0", "v1.0.0"));
        assert!(!is_newer("v1.0.0", "v0.0.0"));
        assert!(is_newer("v0.0.0", "v0.1.0"));
        assert!(is_newer("v0.0.0", "v0.0.1"));
        assert!(!is_newer("v0.0.1", "v0.0.0"));
        assert!(!is_newer("v0.1.0", "v0.0.1"));
        assert!(!is_newer("v1.0.0", "v0.0.1"));
        assert!(!is_newer("v1.0.5", "v1.0.3"));
        assert!(!is_newer("v1.5.0", "v1.3.0"));
        assert!(!is_newer("v5.0.0", "v3.0.0"));
        assert!(!is_newer("v1.2.3", "v1.2.3"));
    }

    #[test]
    fn is_stable_version_rejects_prerelease_shapes() {
        assert!(is_stable_version("v1.2.3"));
        assert!(is_stable_version("1.2.3"));
        assert!(is_stable_version("v1.2"));
        assert!(!is_stable_version("v1.2.3-rc1"));
        assert!(!is_stable_version("v1.2.3-beta.2"));
        assert!(!is_stable_version(""));
        assert!(!is_stable_version("nightly"));
    }

    #[test]
    fn current_version_is_v_prefixed_and_fake_tag_is_strictly_newer() {
        let current = current_version();
        assert!(current.starts_with('v'));
        assert_eq!(current, format!("v{}", env!("CARGO_PKG_VERSION")));
        assert!(compare_versions(&fake_newer_tag(), &current).is_gt());
    }

    // ── ⑥ cargo guard ───────────────────────────────────────────

    #[test]
    fn cargo_managed_binary_is_refused_before_any_download() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let cargo_root = dir.path().join("cargo");
        let bin_dir = cargo_root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("ghscaff");
        std::fs::write(&exe, b"cargo-installed-binary").unwrap();

        let downloader =
            RecordingDownloader::new(make_tar_gz(&[("ghscaff", b"new")]), Err("x".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&fake_newer_tag())]),
            exe: &exe,
            cargo_bin: Some(&cargo_root),
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| panic!("cargo-managed install must not prompt"),
        };

        assert_eq!(run_update_with(false, false, &deps).unwrap(), 0);
        assert!(
            downloader.calls().is_empty(),
            "no download for cargo installs"
        );
        assert_eq!(std::fs::read(&exe).unwrap(), b"cargo-installed-binary");
        assert!(!tmp_staging_for(&exe).exists());
    }

    #[test]
    fn cargo_refusal_hint_is_the_exact_specified_command() {
        assert_eq!(CARGO_INSTALL_HINT, "cargo install --force ghscaff");
    }

    // ── ⑦ declining the prompt ──────────────────────────────────

    #[test]
    fn declining_the_prompt_leaves_the_binary_untouched() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let downloader =
            RecordingDownloader::new(make_tar_gz(&[("ghscaff", b"new")]), Err("x".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&fake_newer_tag())]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| false,
        };

        assert_eq!(run_update_with(false, false, &deps).unwrap(), 0);
        assert_eq!(std::fs::read(&exe).unwrap(), b"old-binary");
        assert!(downloader.calls().is_empty(), "no download before consent");
        assert!(!tmp_staging_for(&exe).exists());
    }

    // ── ⑧ accepted prompt replaces the binary ───────────────────

    #[test]
    fn accepting_replaces_the_binary_atomically_and_keeps_mode() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let tag = fake_newer_tag();
        let archive = make_tar_gz(&[("ghscaff", b"brand-new-binary")]);
        let sums = format!(
            "{}  {}\n",
            sha256_hex(&archive),
            asset_name(&tag, "x86_64", "unknown-linux-musl")
        );
        let downloader = RecordingDownloader::new(archive, Ok(sums.into_bytes()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&tag)]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| true,
        };

        assert_eq!(run_update_with(false, false, &deps).unwrap(), 0);
        assert_eq!(std::fs::read(&exe).unwrap(), b"brand-new-binary");
        assert!(!tmp_staging_for(&exe).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&exe).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }

        // Asset naming follows rust-release.yml: ghscaff-{tag}-{arch}-{os}.tar.gz
        let calls = downloader.calls();
        assert!(
            calls[0].ends_with(&format!(
                "/releases/download/{tag}/{}",
                asset_name(&tag, "x86_64", "unknown-linux-musl")
            )),
            "unexpected asset url: {}",
            calls[0]
        );
    }

    #[test]
    fn yes_flag_skips_the_prompt_entirely() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let archive = make_tar_gz(&[("ghscaff", b"yes-binary")]);
        let downloader = RecordingDownloader::new(archive, Err("no sums".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&fake_newer_tag())]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| panic!("--yes must not prompt"),
        };

        assert_eq!(run_update_with(false, true, &deps).unwrap(), 0);
        assert_eq!(std::fs::read(&exe).unwrap(), b"yes-binary");
    }

    // ── ⑨⑩ failure paths ───────────────────────────────────────

    #[test]
    fn target_error_fails_before_touching_the_downloader() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let downloader =
            RecordingDownloader::new(make_tar_gz(&[("ghscaff", b"new")]), Err("x".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&fake_newer_tag())]),
            exe: &exe,
            cargo_bin: None,
            target: Err("unsupported OS: windows".into()),
            downloader: &downloader,
            confirm: &|| true,
        };

        let err = run_update_with(false, false, &deps).unwrap_err();
        assert!(
            err.to_string().contains("target resolution failed"),
            "{err:#}"
        );
        assert!(downloader.calls().is_empty());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old-binary");
    }

    #[test]
    fn release_lookup_error_is_loud() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Err("HTTP 403: rate limit exceeded".into()),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| panic!("must not prompt"),
        };

        let err = run_update_with(false, false, &deps).unwrap_err();
        assert!(err.to_string().contains("release lookup failed"), "{err:#}");
        assert!(downloader.calls().is_empty());
    }

    // ── ⑪⑬⑭ checksum rules (install.sh semantics) ──────────────

    #[test]
    fn matching_checksum_replaces_the_binary() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let tag = fake_newer_tag();
        let archive = make_tar_gz(&[("ghscaff", b"verified-binary")]);
        let asset = asset_name(&tag, "x86_64", "unknown-linux-musl");
        let sums = format!("{}  {asset}\n0000000000000000000000000000000000000000000000000000000000000000  other-asset.tar.gz\n", sha256_hex(&archive));
        let downloader = RecordingDownloader::new(archive, Ok(sums.into_bytes()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&tag)]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| true,
        };

        assert_eq!(run_update_with(false, false, &deps).unwrap(), 0);
        assert_eq!(std::fs::read(&exe).unwrap(), b"verified-binary");
    }

    #[test]
    fn checksum_mismatch_aborts_with_the_binary_untouched() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let tag = fake_newer_tag();
        let archive = make_tar_gz(&[("ghscaff", b"tampered")]);
        let asset = asset_name(&tag, "x86_64", "unknown-linux-musl");
        let sums = format!("deadbeef  {asset}\n");
        let downloader = RecordingDownloader::new(archive, Ok(sums.into_bytes()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&tag)]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| true,
        };

        let err = run_update_with(false, false, &deps).unwrap_err();
        assert!(err.to_string().contains("SHA256 mismatch"), "{err:#}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old-binary");
        assert!(!tmp_staging_for(&exe).exists());
    }

    #[test]
    fn missing_sums_file_skips_verification_and_still_replaces() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let archive = make_tar_gz(&[("ghscaff", b"unverifiable-but-fine")]);
        let downloader = RecordingDownloader::new(archive, Err("HTTP 404".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&fake_newer_tag())]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| true,
        };

        assert_eq!(run_update_with(false, false, &deps).unwrap(), 0);
        assert_eq!(std::fs::read(&exe).unwrap(), b"unverifiable-but-fine");
        assert_eq!(
            downloader
                .calls()
                .iter()
                .filter(|url| url.ends_with("SHA256SUMS.txt"))
                .count(),
            1
        );
    }

    #[test]
    fn sums_file_without_a_line_for_this_asset_aborts() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let tag = fake_newer_tag();
        let archive = make_tar_gz(&[("ghscaff", b"new")]);
        let sums = "0000000000000000000000000000000000000000000000000000000000000000  some-other-asset.tar.gz\n";
        let downloader = RecordingDownloader::new(archive, Ok(sums.as_bytes().to_vec()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&tag)]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| true,
        };

        let err = run_update_with(false, false, &deps).unwrap_err();
        assert!(err.to_string().contains("no checksum listed"), "{err:#}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old-binary");
        assert!(!tmp_staging_for(&exe).exists());
    }

    // ── ⑮ archive without the ghscaff entry ─────────────────────

    #[test]
    fn archive_without_ghscaff_entry_fails_and_leaves_the_binary_untouched() {
        let current = current_version();
        let dir = tempfile::tempdir().unwrap();
        let exe = make_exe(dir.path());
        let archive = make_tar_gz(&[("README.md", b"not the binary")]);
        let downloader = RecordingDownloader::new(archive, Err("no sums".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&fake_newer_tag())]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| true,
        };

        let err = run_update_with(false, false, &deps).unwrap_err();
        assert!(err.to_string().contains("not found"), "{err:#}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old-binary");
        assert!(!tmp_staging_for(&exe).exists());
    }

    // ── ⑯ ~/.ghscaff state is never touched ─────────────────────

    #[test]
    fn update_never_touches_ghscaff_state() {
        let _guard = env_lock();
        let previous_home = std::env::var_os("HOME");

        let home = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", home.path());

        // Sentinels: the encrypted vault and the boilerplate cache.
        let state = home.path().join(".ghscaff");
        let boilerplate = state.join("boilerplate");
        std::fs::create_dir_all(&boilerplate).unwrap();
        std::fs::write(state.join("vault.enc"), b"encrypted-token-bytes").unwrap();
        std::fs::write(boilerplate.join("rust"), b"cached-boilerplate").unwrap();

        let snapshot = || -> Vec<(PathBuf, Vec<u8>)> {
            let mut entries = Vec::new();
            for path in walkdir::WalkDir::new(&state).sort_by_file_name() {
                let path = path.unwrap().path().to_path_buf();
                if path.is_file() {
                    entries.push((path.clone(), std::fs::read(&path).unwrap()));
                }
            }
            entries
        };
        let before = snapshot();

        // The binary lives somewhere else entirely — as in production.
        let exe_dir = tempfile::tempdir().unwrap();
        let exe = make_exe(exe_dir.path());
        let current = current_version();
        let archive = make_tar_gz(&[("ghscaff", b"state-safe-binary")]);
        let downloader = RecordingDownloader::new(archive, Err("no sums".into()));
        let deps = UpdateDeps {
            current: &current,
            releases: Ok(vec![release(&fake_newer_tag())]),
            exe: &exe,
            cargo_bin: None,
            target: Ok(("x86_64", "unknown-linux-musl")),
            downloader: &downloader,
            confirm: &|| true,
        };

        assert_eq!(run_update_with(false, false, &deps).unwrap(), 0);
        assert_eq!(std::fs::read(&exe).unwrap(), b"state-safe-binary");

        // State directory: same files, same bytes, nothing added or removed.
        let after = snapshot();
        assert_eq!(before, after, "update must not touch ~/.ghscaff state");

        match previous_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
    }

    // ── ⑰ asset naming pins (rust-release.yml) ──────────────────

    #[test]
    fn asset_naming_matches_the_release_workflow() {
        assert_eq!(
            asset_name("v0.7.0", "x86_64", "unknown-linux-musl"),
            "ghscaff-v0.7.0-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            asset_name("v0.7.0", "aarch64", "apple-darwin"),
            "ghscaff-v0.7.0-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(
            asset_url("v0.7.0", ("x86_64", "unknown-linux-musl")),
            "https://github.com/UniverLab/ghscaff/releases/download/v0.7.0/ghscaff-v0.7.0-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            sums_url("v0.7.0"),
            "https://github.com/UniverLab/ghscaff/releases/download/v0.7.0/SHA256SUMS.txt"
        );
        assert_eq!(
            releases_url(),
            "https://api.github.com/repos/UniverLab/ghscaff/releases?per_page=100"
        );
    }

    // ── fetcher seam (notice + command share the selection) ─────

    #[test]
    fn fetch_latest_stable_parses_the_release_list() {
        let current = current_version();
        let newer = fake_newer_tag();
        let body = format!(
            r#"[{{"tag_name":"{current}","prerelease":false,"draft":false}},
               {{"tag_name":"{newer}","prerelease":false,"draft":false}},
               {{"tag_name":"v99.0.0","prerelease":false,"draft":true}}]"#
        );
        let fetcher = FakeFetcher { body: Ok(body) };
        assert_eq!(
            fetch_latest_stable_with(&fetcher, &current).unwrap(),
            Some(newer)
        );
    }

    #[test]
    fn check_notice_is_silent_on_every_failure() {
        let failing = FakeFetcher {
            body: Err("connection refused".into()),
        };
        assert_eq!(check_notice(&failing, &current_version()), None);

        let unparsable = FakeFetcher {
            body: Ok("not json".into()),
        };
        assert_eq!(check_notice(&unparsable, &current_version()), None);

        let offline_shaped = FakeFetcher {
            body: Ok("[{\"tag_name\":\"v0.0.1\"}]".into()),
        };
        assert_eq!(check_notice(&offline_shaped, &current_version()), None);
    }

    // ── retained helpers: extraction, replacement, cargo detect ─

    #[test]
    fn extract_binary_finds_named_entry() {
        let archive = make_tar_gz(&[("ghscaff", b"fake-binary-contents")]);
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out");
        extract_binary(std::io::Cursor::new(archive), &output).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"fake-binary-contents");
    }

    #[test]
    fn extract_binary_rejects_missing_entry() {
        let archive = make_tar_gz(&[("other-file", b"contents")]);
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out");
        let err = extract_binary(std::io::Cursor::new(archive), &output).unwrap_err();
        assert!(err.to_string().contains("not found"));
        assert!(!output.exists());
    }

    #[test]
    fn extract_binary_rejects_empty_binary() {
        let archive = make_tar_gz(&[("ghscaff", b"")]);
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out");
        let err = extract_binary(std::io::Cursor::new(archive), &output).unwrap_err();
        assert!(err.to_string().contains("empty"));
        assert!(!output.exists());
    }

    #[test]
    fn extract_binary_rejects_corrupt_archive() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out");
        let result = extract_binary(std::io::Cursor::new(b"not a gzip stream".to_vec()), &output);
        assert!(result.is_err());
        assert!(!output.exists());
    }

    #[test]
    fn extract_binary_with_multiple_entries_finds_correct_one() {
        let archive = make_tar_gz(&[
            ("README.md", b"not the binary"),
            ("ghscaff", b"real-binary-data"),
            ("LICENSE", b"MIT"),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("out");
        extract_binary(std::io::Cursor::new(archive), &output).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"real-binary-data");
    }

    #[test]
    fn update_binary_at_replaces_target_and_sets_executable() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("ghscaff");
        std::fs::write(&target, b"old-binary").unwrap();
        let tmp_path = dir.path().join(".ghscaff.update");

        update_binary_at(&tmp_path, &target, |out| {
            std::fs::write(out, b"new-binary")?;
            Ok(())
        })
        .unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new-binary");
        assert!(!tmp_path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn update_binary_at_leaves_target_untouched_when_fetch_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("ghscaff");
        std::fs::write(&target, b"original-binary").unwrap();
        let tmp_path = dir.path().join(".ghscaff.update");

        let result = update_binary_at(&tmp_path, &target, |_out| {
            anyhow::bail!("simulated download failure")
        });

        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"original-binary");
        assert!(!tmp_path.exists());
    }

    #[test]
    fn update_binary_at_cleans_up_tmp_file_when_extraction_writes_then_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("ghscaff");
        std::fs::write(&target, b"original-binary").unwrap();
        let tmp_path = dir.path().join(".ghscaff.update");

        let result = update_binary_at(&tmp_path, &target, |out| {
            std::fs::write(out, b"partial-garbage")?;
            anyhow::bail!("corrupt archive")
        });

        assert!(result.is_err());
        assert!(!tmp_path.exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"original-binary");
    }

    #[test]
    fn update_binary_at_preserves_target_on_rename_failure() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("ghscaff");
        std::fs::write(&target, b"original").unwrap();
        let tmp = dir.path().join(".update");

        // Write to tmp, then try to rename to a non-existent directory
        let bad_target = dir.path().join("nonexistent").join("ghscaff");
        let result = update_binary_at(&tmp, &bad_target, |out| {
            std::fs::write(out, b"new-content")?;
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
    }

    #[test]
    fn resolve_cargo_root_prefers_install_root() {
        let root = resolve_cargo_root(
            Some("/opt/install-root".to_string()),
            Some("/opt/cargo-home".to_string()),
            Some(PathBuf::from("/home/user")),
        );
        assert_eq!(root, Some(PathBuf::from("/opt/install-root")));
    }

    #[test]
    fn resolve_cargo_root_falls_back_to_cargo_home() {
        let root = resolve_cargo_root(
            None,
            Some("/opt/cargo-home".to_string()),
            Some(PathBuf::from("/home/user")),
        );
        assert_eq!(root, Some(PathBuf::from("/opt/cargo-home")));
    }

    #[test]
    fn resolve_cargo_root_falls_back_to_home_dot_cargo() {
        let root = resolve_cargo_root(None, None, Some(PathBuf::from("/home/user")));
        assert_eq!(root, Some(PathBuf::from("/home/user/.cargo")));
    }

    #[test]
    fn resolve_cargo_root_ignores_empty_env_values() {
        let root = resolve_cargo_root(
            Some(String::new()),
            Some(String::new()),
            Some(PathBuf::from("/home/user")),
        );
        assert_eq!(root, Some(PathBuf::from("/home/user/.cargo")));
    }

    #[test]
    fn is_cargo_managed_detects_path_inside_root() {
        let dir = tempfile::tempdir().unwrap();
        let cargo_root = dir.path().join("cargo");
        let bin_dir = cargo_root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let exe = bin_dir.join("ghscaff");
        std::fs::write(&exe, b"binary").unwrap();

        assert!(is_cargo_managed_with_root(&exe, Some(cargo_root)));
    }

    #[test]
    fn is_cargo_managed_rejects_path_outside_root() {
        let dir = tempfile::tempdir().unwrap();
        let cargo_root = dir.path().join("cargo");
        std::fs::create_dir_all(cargo_root.join("bin")).unwrap();
        let other_dir = dir.path().join("elsewhere");
        std::fs::create_dir_all(&other_dir).unwrap();
        let exe = other_dir.join("ghscaff");
        std::fs::write(&exe, b"binary").unwrap();

        assert!(!is_cargo_managed_with_root(&exe, Some(cargo_root)));
    }

    #[test]
    fn is_cargo_managed_treats_uncanonicalizable_path_as_not_cargo() {
        let dir = tempfile::tempdir().unwrap();
        let cargo_root = dir.path().join("cargo");
        std::fs::create_dir_all(cargo_root.join("bin")).unwrap();
        let missing_exe = dir.path().join("does-not-exist");

        assert!(!is_cargo_managed_with_root(&missing_exe, Some(cargo_root)));
    }

    #[test]
    fn is_cargo_managed_with_no_resolvable_root_is_false() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("ghscaff");
        std::fs::write(&exe, b"binary").unwrap();

        assert!(!is_cargo_managed_with_root(&exe, None));
    }

    #[test]
    fn detect_platform_returns_supported_tuple_on_this_host() {
        let result = detect_platform();
        if (cfg!(target_os = "linux") || cfg!(target_os = "macos"))
            && (cfg!(target_arch = "x86_64") || cfg!(target_arch = "aarch64"))
        {
            assert!(result.is_ok());
        }
    }

    #[test]
    fn cargo_install_root_returns_something() {
        // cargo_install_root reads env vars; on any dev machine it should
        // resolve to at least Some(...) via CARGO_HOME or ~/.cargo.
        let root = cargo_install_root();
        assert!(root.is_some());
    }

    #[test]
    fn is_cargo_managed_delegates_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("ghscaff");
        std::fs::write(&exe, b"binary").unwrap();
        // Without a real cargo root, this should be false
        assert!(!is_cargo_managed(&exe));
    }
}
