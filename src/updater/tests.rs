use super::*;
use std::cell::RefCell;
use std::io::Write;
use std::sync::MutexGuard;

/// Tests that read or write process-wide environment serialize on the one
/// crate-wide lock, so a `HOME` swap here can never overlap with the `vault`,
/// `apply` or `wizard` tests that swap the same variable; tests run in
/// parallel by default.
fn env_lock() -> MutexGuard<'static, ()> {
    crate::github::test_utils::env_lock()
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

/// Run the injected-fetcher path end to end the way `run_update` wires it:
/// fetch → one-line error string → hermetic core. Returns the exit code.
fn exit_when_release_lookup_fails(check: bool, body: std::result::Result<String, String>) -> i32 {
    let current = current_version();
    let dir = tempfile::tempdir().unwrap();
    let exe = make_exe(dir.path());
    let fetcher = FakeFetcher { body };
    let releases = fetch_releases_with(&fetcher).map_err(|error| format!("{error:#}"));
    assert!(releases.is_err(), "test premise: the lookup must fail");
    let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
    let deps = UpdateDeps {
        current: &current,
        releases,
        exe: &exe,
        cargo_bin: None,
        target: Ok(("x86_64", "unknown-linux-musl")),
        downloader: &downloader,
        confirm: &|| panic!("must not prompt"),
    };
    let code = run_update_with(check, false, &deps).unwrap();
    assert!(
        downloader.calls().is_empty(),
        "a failed check downloads nothing"
    );
    assert_eq!(
        std::fs::read(&exe).unwrap(),
        b"old-binary",
        "state untouched"
    );
    code
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

    let downloader = RecordingDownloader::new(make_tar_gz(&[("ghscaff", b"new")]), Err("x".into()));
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
    let downloader = RecordingDownloader::new(make_tar_gz(&[("ghscaff", b"new")]), Err("x".into()));
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
    let downloader = RecordingDownloader::new(make_tar_gz(&[("ghscaff", b"new")]), Err("x".into()));
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
fn check_exits_2_when_the_release_lookup_fails() {
    assert_eq!(
        exit_when_release_lookup_fails(true, Err("dns failure".into())),
        2
    );
}

#[test]
fn check_exits_2_on_an_unparsable_response() {
    assert_eq!(
        exit_when_release_lookup_fails(true, Ok("not json".into())),
        2
    );
}

#[test]
fn plain_update_exits_2_when_the_release_lookup_fails() {
    assert_eq!(
        exit_when_release_lookup_fails(false, Err("HTTP 403: rate limit exceeded".into())),
        2
    );
}

// ── exit-2 wording: one stderr line, cause named ─────────────

/// The check-error string is what lands on stderr: it must be a single
/// line that names the cause.
#[test]
fn check_failure_cause_is_a_single_stderr_line() {
    for cause in [
        "failed to fetch GitHub releases: dns error: no such host",
        "failed to fetch GitHub releases: error sending request (TLS handshake)",
        "GitHub releases request failed: HTTP 403 Forbidden",
        "failed to parse releases JSON: expected value at line 1 column 1",
    ] {
        let error = anyhow!(cause);
        let line = format!("update check failed: {error:#}");
        assert!(!line.contains('\n'), "one line only: {line:?}");
        assert!(line.contains(cause), "names the cause: {line}");
    }
}

// ── output wording: bare versions, no restart advice ─────────

#[test]
fn display_version_strips_only_a_leading_v() {
    assert_eq!(display_version("v0.6.0"), "0.6.0");
    assert_eq!(display_version("0.6.0"), "0.6.0");
    assert_eq!(display_version(""), "");
}

#[test]
fn success_line_has_bare_version_and_no_restart_advice() {
    assert_eq!(success_line("v0.6.0"), "✓ updated to 0.6.0");
}

#[test]
fn arrow_line_drops_the_v_prefix() {
    assert_eq!(arrow_line("v0.0.1", "v0.6.0"), "ghscaff 0.0.1 → 0.6.0");
}

#[test]
fn up_to_date_line_drops_the_v_prefix() {
    assert_eq!(up_to_date_line("v0.6.0"), "ghscaff 0.6.0 is up to date");
}

#[test]
fn cargo_refusal_is_a_full_sentence() {
    assert_eq!(
        cargo_refusal_line(),
        "installed with cargo — run: cargo install --force ghscaff"
    );
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

// ── run_update exit codes through the injected seams ────────────

fn release_list_body(tags: &[String]) -> String {
    let entries: Vec<String> = tags
        .iter()
        .map(|tag| format!(r#"{{"tag_name":"{tag}","prerelease":false,"draft":false}}"#))
        .collect();
    format!("[{}]", entries.join(","))
}

#[test]
fn check_notice_returns_some_for_newer_stable() {
    let current = current_version();
    let newer = fake_newer_tag();
    let fetcher = FakeFetcher {
        body: Ok(release_list_body(&[current.clone(), newer.clone()])),
    };
    assert_eq!(check_notice(&fetcher, &current), Some(newer));
}

#[test]
fn run_update_up_to_date_exits_0() {
    let current = current_version();
    let fetcher = FakeFetcher {
        body: Ok(release_list_body(std::slice::from_ref(&current))),
    };
    let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
    assert_eq!(run_update(false, false, &fetcher, &downloader).unwrap(), 0);
    assert!(
        downloader.calls().is_empty(),
        "up to date downloads nothing"
    );
}

#[test]
fn run_update_check_mode_exits_1_when_newer() {
    let current = current_version();
    let fetcher = FakeFetcher {
        body: Ok(release_list_body(&[current, fake_newer_tag()])),
    };
    let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
    assert_eq!(run_update(true, false, &fetcher, &downloader).unwrap(), 1);
    assert!(downloader.calls().is_empty(), "--check downloads nothing");
}

#[test]
fn run_update_check_failure_exits_2() {
    let fetcher = FakeFetcher {
        body: Err("dns failure".into()),
    };
    let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
    assert_eq!(run_update(true, false, &fetcher, &downloader).unwrap(), 2);
    assert!(downloader.calls().is_empty());
}

#[test]
fn run_update_plain_mode_check_failure_exits_2() {
    let fetcher = FakeFetcher {
        body: Err("HTTP 403: rate limit exceeded".into()),
    };
    let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
    assert_eq!(run_update(false, false, &fetcher, &downloader).unwrap(), 2);
    assert!(downloader.calls().is_empty());
}

#[test]
fn run_update_unparsable_response_exits_2() {
    let fetcher = FakeFetcher {
        body: Ok("not json".into()),
    };
    let downloader = RecordingDownloader::new(Vec::new(), Err("unused".into()));
    assert_eq!(run_update(false, false, &fetcher, &downloader).unwrap(), 2);
    assert!(downloader.calls().is_empty());
}

// ── dry-run path (main maps `update --dry-run` to check = true) ───

#[test]
fn dry_run_reports_available_update_without_downloading() {
    let current = current_version();
    let fetcher = FakeFetcher {
        body: Ok(release_list_body(&[current, fake_newer_tag()])),
    };
    let downloader = RecordingDownloader::new(make_tar_gz(&[("ghscaff", b"new")]), Err("x".into()));
    // `--yes` is ignored on the dry-run path: still a read-only report.
    assert_eq!(run_update(true, true, &fetcher, &downloader).unwrap(), 1);
    assert!(
        downloader.calls().is_empty(),
        "dry run never downloads or installs"
    );
}

#[test]
fn dry_run_reports_up_to_date_without_downloading() {
    let current = current_version();
    let fetcher = FakeFetcher {
        body: Ok(release_list_body(std::slice::from_ref(&current))),
    };
    let downloader = RecordingDownloader::new(make_tar_gz(&[("ghscaff", b"new")]), Err("x".into()));
    assert_eq!(run_update(true, true, &fetcher, &downloader).unwrap(), 0);
    assert!(
        downloader.calls().is_empty(),
        "dry run never downloads or installs"
    );
}
