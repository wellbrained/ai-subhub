//! Self-update from GitHub Releases (ADR 0001, "Updates"). Pure decisions (versions, release JSON,
//! checksum lines, dev-build check, when it is safe to restart) are tested here; the two network
//! calls (`check_latest`, `download_verified`) are thin and blocking: call them from a background
//! thread. The file swap lives in `swap`, patch notes in `changelog`.
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::settings::UpdateMode;

/// Repo the releases come from.
pub const REPO: &str = "wellbrained/ai-subhub";
/// Asset names in a release (the checksum file is `<exe>.sha256`).
pub const EXE_ASSET: &str = "ai-subhub.exe";
pub const SHA_ASSET: &str = "ai-subhub.exe.sha256";
/// How often an update is looked for, and how soon after a failed try.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
pub const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(30 * 60);
/// Refuse downloads bigger than this (the exe is a few tens of MB).
const MAX_DOWNLOAD: u64 = 300 * 1024 * 1024;

fn latest_url() -> String {
    format!("https://api.github.com/repos/{REPO}/releases/latest")
}

// ---------- versions ----------

/// `major.minor.patch`; compares numerically (0.10.0 > 0.9.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// "1.2.3" or a tag "v1.2.3". Pre-release / build suffixes ("1.2.3-beta") are not versions
    /// we install, so they do not parse.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let mut it = s.split('.');
        let mut part = || -> Option<u32> {
            let p = it.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()
        };
        let v = Self {
            major: part()?,
            minor: part()?,
            patch: part()?,
        };
        it.next().is_none().then_some(v)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Only strictly newer versions are offered, so there are no downgrades.
pub fn is_newer(candidate: &Version, running: &Version) -> bool {
    candidate > running
}

// ---------- release JSON ----------

/// What the update needs from a GitHub release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub exe_url: String,
    pub sha_url: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseError {
    /// Not JSON, or not a release object.
    BadJson,
    /// `tag_name` is not `vX.Y.Z`.
    BadTag,
    /// A draft or pre-release: never offered.
    NotStable,
    /// The release lacks this asset.
    MissingAsset(&'static str),
    /// An asset URL that is not HTTPS.
    InsecureUrl,
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadJson => write!(f, "unexpected answer from GitHub"),
            Self::BadTag => write!(f, "the latest release has no usable version"),
            Self::NotStable => write!(f, "the latest release is not final"),
            Self::MissingAsset(n) => write!(f, "the latest release has no {n}"),
            Self::InsecureUrl => write!(f, "the release download is not HTTPS"),
        }
    }
}

#[derive(Deserialize)]
struct RawRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: Option<u64>,
}

/// Parses the `releases/latest` answer; the assets are found by name.
pub fn parse_release(json: &str) -> Result<Release, ReleaseError> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|_| ReleaseError::BadJson)?;
    if !value.is_object() {
        return Err(ReleaseError::BadJson);
    }
    let raw: RawRelease = serde_json::from_value(value).map_err(|_| ReleaseError::BadJson)?;
    if raw.draft || raw.prerelease {
        return Err(ReleaseError::NotStable);
    }
    let version = Version::parse(&raw.tag_name).ok_or(ReleaseError::BadTag)?;
    let find = |name: &'static str| {
        raw.assets
            .iter()
            .find(|a| a.name == name)
            .ok_or(ReleaseError::MissingAsset(name))
    };
    let exe = find(EXE_ASSET)?;
    let sha = find(SHA_ASSET)?;
    for a in [exe, sha] {
        if !a.browser_download_url.starts_with("https://") {
            return Err(ReleaseError::InsecureUrl);
        }
    }
    Ok(Release {
        version,
        exe_url: exe.browser_download_url.clone(),
        sha_url: sha.browser_download_url.clone(),
        size: exe.size,
    })
}

// ---------- checksums ----------

/// The digest from a `.sha256` file: `<hex>`, `<hex>  <file>` or `<hex> *<file>`. Lowercased;
/// None when the first token is not 64 hex digits.
pub fn parse_sha256(text: &str) -> Option<String> {
    let text = text.trim_start_matches('\u{feff}').trim_start();
    let token = text.split_whitespace().next()?;
    (token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| token.to_ascii_lowercase())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    use fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ---------- when and where ----------

/// A build run from cargo's output folder (`target\debug`, `target\release`, also with a target
/// triple in between): checking is fine, installing never.
pub fn is_dev_build(exe: &Path) -> bool {
    let parts: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    parts.iter().enumerate().any(|(i, p)| {
        p == "target"
            && parts[i + 1..]
                .iter()
                .any(|q| q == "debug" || q == "release")
    })
}

/// The renamed running exe: `<exe>.old` (the file name with `.old` appended).
pub fn old_path(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

/// `%LOCALAPPDATA%\AI SubHub\updates`.
pub fn updates_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("AI SubHub").join("updates")
}

fn download_name(v: &Version) -> String {
    format!("ai-subhub-{v}.exe")
}

/// A file in the updates folder that is no longer needed: a partial download, or an exe that
/// is not newer than the running version.
pub fn is_stale_update_file(name: &str, running: &Version) -> bool {
    if name.ends_with(".part") {
        return true;
    }
    let Some(v) = name
        .strip_prefix("ai-subhub-")
        .and_then(|n| n.strip_suffix(".exe"))
    else {
        return false;
    };
    Version::parse(v).is_none_or(|v| v <= *running)
}

/// Removes stale files from the updates folder (best effort).
pub fn clean_updates_dir(dir: &Path, running: &Version) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if is_stale_update_file(&e.file_name().to_string_lossy(), running) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Is a check due? `since_last_try` is None before the first try in this run. After a failed
/// try it waits 30 minutes, otherwise 6 hours.
pub fn check_due(since_last_try: Option<Duration>, last_failed: bool) -> bool {
    match since_last_try {
        None => true,
        Some(d) => {
            d >= if last_failed {
                RETRY_AFTER_FAILURE
            } else {
                CHECK_EVERY
            }
        }
    }
}

/// What keeps an automatic restart waiting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Busy {
    pub popover_open: bool,
    pub settings_open: bool,
    pub stats_open: bool,
    /// The "Updated to …" dialog.
    pub notes_open: bool,
    /// A sign-in terminal is being watched.
    pub signing_in: bool,
    /// The popover's top edge is being dragged.
    pub popover_dragging: bool,
}

impl Busy {
    pub fn is_idle(&self) -> bool {
        *self == Self::default()
    }
}

/// Install by itself now? Auto mode, a real install (not a dev build), a verified download
/// ready and nothing in use. Manual installs run when the user clicks, regardless.
pub fn auto_install_due(mode: UpdateMode, dev_build: bool, downloaded: bool, busy: &Busy) -> bool {
    mode == UpdateMode::Auto && !dev_build && downloaded && busy.is_idle()
}

// ---------- network ----------

/// A problem with a safe, short message (no URLs, paths or tokens).
#[derive(Debug)]
pub enum UpdateError {
    Network(String),
    Release(ReleaseError),
    BadChecksumFile,
    ChecksumMismatch,
    TooLarge,
    Disk(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(m) => write!(f, "{m}"),
            Self::Release(e) => write!(f, "{e}"),
            Self::BadChecksumFile => write!(f, "the checksum file is not readable"),
            Self::ChecksumMismatch => {
                write!(
                    f,
                    "the download does not match its checksum and was deleted"
                )
            }
            Self::TooLarge => write!(f, "the download is unexpectedly large"),
            Self::Disk(m) => write!(f, "could not save the download ({m})"),
        }
    }
}

impl std::error::Error for UpdateError {}

fn net_error(e: &reqwest::Error) -> UpdateError {
    UpdateError::Network(if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "no connection".into()
    } else if let Some(s) = e.status() {
        format!("HTTP {}", s.as_u16())
    } else {
        "download failed".into()
    })
}

/// io errors by kind only: the message text can carry paths.
pub fn io_reason(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        PermissionDenied => "access is denied".into(),
        NotFound => "file not found".into(),
        StorageFull => "the disk is full".into(),
        ReadOnlyFilesystem => "the folder is read-only".into(),
        k => k.to_string(),
    }
}

/// HTTPS only, also across redirects (GitHub sends downloads to its CDN).
fn client(timeout: Duration) -> Result<reqwest::blocking::Client, UpdateError> {
    use reqwest::redirect::Policy;
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .user_agent(concat!("ai-subhub/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .redirect(Policy::custom(|a| {
            if a.url().scheme() == "https" && a.previous().len() < 5 {
                a.follow()
            } else {
                a.stop()
            }
        }))
        .build()
        .map_err(|e| net_error(&e))
}

/// Result of a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    UpToDate,
    Available(Release),
}

/// Asks GitHub for the latest release. A missing release (404) or one that is not newer than
/// `running` is "up to date". Blocking.
pub fn check_latest(running: &Version) -> Result<Checked, UpdateError> {
    let resp = client(Duration::from_secs(20))?
        .get(latest_url())
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| net_error(&e))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Checked::UpToDate);
    }
    if !resp.status().is_success() {
        return Err(UpdateError::Network(format!(
            "HTTP {}",
            resp.status().as_u16()
        )));
    }
    let mut body = String::new();
    resp.take(2 * 1024 * 1024)
        .read_to_string(&mut body)
        .map_err(|_| UpdateError::Network("download failed".into()))?;
    decide(&body, running)
}

/// Pure part of `check_latest`: JSON in, decision out. A draft / pre-release is "up to date".
fn decide(json: &str, running: &Version) -> Result<Checked, UpdateError> {
    match parse_release(json) {
        Ok(r) if is_newer(&r.version, running) => Ok(Checked::Available(r)),
        Ok(_) | Err(ReleaseError::NotStable) => Ok(Checked::UpToDate),
        Err(e) => Err(UpdateError::Release(e)),
    }
}

/// Downloads the release exe into `dir` and verifies its SHA-256. A matching file from an
/// earlier try is reused; on a mismatch the file is deleted. Returns the verified path.
/// Blocking.
pub fn download_verified(release: &Release, dir: &Path) -> Result<PathBuf, UpdateError> {
    let disk = |e: std::io::Error| UpdateError::Disk(io_reason(&e));
    std::fs::create_dir_all(dir).map_err(disk)?;
    let target = dir.join(download_name(&release.version));

    // The expected digest first: it is tiny and decides whether a stored copy is reusable.
    let mut text = String::new();
    client(Duration::from_secs(30))?
        .get(&release.sha_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| net_error(&e))?
        .take(4096)
        .read_to_string(&mut text)
        .map_err(|_| UpdateError::BadChecksumFile)?;
    let expected = parse_sha256(&text).ok_or(UpdateError::BadChecksumFile)?;

    if target.exists() {
        if sha256_file(&target).is_ok_and(|h| h == expected) {
            return Ok(target);
        }
        let _ = std::fs::remove_file(&target);
    }

    let resp = client(Duration::from_secs(600))?
        .get(&release.exe_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| net_error(&e))?;
    if !resp.status().is_success() {
        return Err(UpdateError::Network(format!(
            "HTTP {}",
            resp.status().as_u16()
        )));
    }
    if resp.content_length().is_some_and(|n| n > MAX_DOWNLOAD) {
        return Err(UpdateError::TooLarge);
    }
    let part = dir.join(format!("{}.part", download_name(&release.version)));
    let written = {
        let mut f = std::fs::File::create(&part).map_err(disk)?;
        // One byte over the cap tells a runaway stream from an exact fit.
        let n = std::io::copy(&mut resp.take(MAX_DOWNLOAD + 1), &mut f).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            UpdateError::Network(format!("download failed ({})", io_reason(&e)))
        })?;
        f.flush().map_err(disk)?;
        n
    };
    if written > MAX_DOWNLOAD {
        let _ = std::fs::remove_file(&part);
        return Err(UpdateError::TooLarge);
    }
    match sha256_file(&part) {
        Ok(h) if h == expected => {}
        Ok(_) => {
            let _ = std::fs::remove_file(&part);
            return Err(UpdateError::ChecksumMismatch);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(disk(e));
        }
    }
    std::fs::rename(&part, &target).map_err(disk)?;
    Ok(target)
}

// ---------- start arguments ----------

/// What the previous process tells the new one: `--updated-from <version> --wait-pid <pid>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartArgs {
    pub updated_from: Option<String>,
    pub wait_pid: Option<u32>,
}

pub fn parse_start_args(args: &[String]) -> StartArgs {
    let after = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    StartArgs {
        // Only a real version is kept; anything else is ignored.
        updated_from: after("--updated-from")
            .and_then(|v| Version::parse(v))
            .map(|v| v.to_string()),
        wait_pid: after("--wait-pid").and_then(|p| p.parse().ok()),
    }
}

// ---------- what's new ----------

/// After an update: the notes to show and whether `last_seen_version` should move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhatsNew {
    pub sections: Vec<crate::changelog::Section>,
    /// `last_seen_version` should be set to the running version and saved.
    pub mark_seen: bool,
}

/// Decides the "Updated to vX.Y.Z" dialog. `last_seen` is the saved text (empty on a fresh
/// install: no dialog, just remember). `updated_from` is the `--updated-from` argument; it
/// stands in when nothing is saved. No dialog without notes for the versions in between.
pub fn whats_new(
    last_seen: &str,
    updated_from: Option<&str>,
    running: &Version,
    changelog: &str,
) -> WhatsNew {
    let saved = Version::parse(last_seen);
    let baseline = saved.or_else(|| updated_from.and_then(Version::parse));
    let mark_seen = saved != Some(*running);
    let sections = match baseline {
        Some(b) if b < *running => crate::changelog::since(changelog, Some(&b), running),
        _ => vec![],
    };
    WhatsNew {
        sections,
        mark_seen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_parse_and_compare_numerically() {
        assert_eq!(v("1.2.3").to_string(), "1.2.3");
        assert_eq!(
            v("v0.10.0"),
            Version {
                major: 0,
                minor: 10,
                patch: 0
            }
        );
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1.2.3-beta",
            "a.b.c",
            "1..3",
            "-1.2.3",
            "+1.2.3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert_eq!(Version::parse(" v1.2.3 "), Some(v("1.2.3")));
    }

    #[test]
    fn only_newer_versions_are_offered() {
        assert!(is_newer(&v("0.1.1"), &v("0.1.0")));
        assert!(!is_newer(&v("0.1.0"), &v("0.1.0")), "same version");
        assert!(!is_newer(&v("0.0.9"), &v("0.1.0")), "no downgrades");
    }

    fn release_json(tag: &str, extra: &str, assets: &str) -> String {
        format!(r#"{{"tag_name":"{tag}",{extra}"assets":[{assets}]}}"#)
    }
    const EXE: &str = r#"{"name":"ai-subhub.exe","size":1234,"browser_download_url":"https://github.com/x/y/releases/download/v0.2.0/ai-subhub.exe"}"#;
    const SHA: &str = r#"{"name":"ai-subhub.exe.sha256","browser_download_url":"https://github.com/x/y/releases/download/v0.2.0/ai-subhub.exe.sha256"}"#;
    const OTHER: &str =
        r#"{"name":"notes.txt","browser_download_url":"https://example.com/notes.txt"}"#;

    #[test]
    fn release_assets_are_found_by_name() {
        let json = release_json(
            "v0.2.0",
            r#""draft":false,"prerelease":false,"body":"hi","#,
            &format!("{OTHER},{SHA},{EXE}"),
        );
        let r = parse_release(&json).unwrap();
        assert_eq!(r.version, v("0.2.0"));
        assert!(r.exe_url.ends_with("/ai-subhub.exe"));
        assert!(r.sha_url.ends_with("/ai-subhub.exe.sha256"));
        assert_eq!(r.size, Some(1234));
    }

    #[test]
    fn release_problems() {
        let ok_assets = format!("{EXE},{SHA}");
        assert_eq!(parse_release("nope"), Err(ReleaseError::BadJson));
        assert_eq!(parse_release("[]"), Err(ReleaseError::BadJson));
        assert_eq!(
            parse_release(&release_json("nightly", "", &ok_assets)),
            Err(ReleaseError::BadTag)
        );
        assert_eq!(
            parse_release(&release_json("v0.2.0", r#""prerelease":true,"#, &ok_assets)),
            Err(ReleaseError::NotStable)
        );
        assert_eq!(
            parse_release(&release_json("v0.2.0", r#""draft":true,"#, &ok_assets)),
            Err(ReleaseError::NotStable)
        );
        assert_eq!(
            parse_release(&release_json("v0.2.0", "", EXE)),
            Err(ReleaseError::MissingAsset(SHA_ASSET))
        );
        assert_eq!(
            parse_release(&release_json("v0.2.0", "", SHA)),
            Err(ReleaseError::MissingAsset(EXE_ASSET))
        );
        let http = EXE.replace("https://", "http://");
        assert_eq!(
            parse_release(&release_json("v0.2.0", "", &format!("{http},{SHA}"))),
            Err(ReleaseError::InsecureUrl)
        );
    }

    #[test]
    fn deciding_from_a_release_answer() {
        let json = release_json("v0.2.0", "", &format!("{EXE},{SHA}"));
        assert!(
            matches!(decide(&json, &v("0.1.0")), Ok(Checked::Available(r)) if r.version == v("0.2.0"))
        );
        assert!(matches!(decide(&json, &v("0.2.0")), Ok(Checked::UpToDate)));
        assert!(
            matches!(decide(&json, &v("0.3.0")), Ok(Checked::UpToDate)),
            "no downgrade"
        );
        let pre = release_json("v0.9.0", r#""prerelease":true,"#, "");
        assert!(matches!(decide(&pre, &v("0.1.0")), Ok(Checked::UpToDate)));
        // GitHub's "Not Found" body (no release yet) is not a release either.
        assert!(decide(r#"{"message":"Not Found"}"#, &v("0.1.0")).is_err());
    }

    #[test]
    fn sha256_lines() {
        let h = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(parse_sha256(h).as_deref(), Some(h));
        assert_eq!(parse_sha256(&format!("{h}\n")).as_deref(), Some(h));
        assert_eq!(
            parse_sha256(&format!("{h}  ai-subhub.exe\r\n")).as_deref(),
            Some(h)
        );
        assert_eq!(
            parse_sha256(&format!("{h} *ai-subhub.exe")).as_deref(),
            Some(h)
        );
        assert_eq!(
            parse_sha256(&h.to_uppercase()).as_deref(),
            Some(h),
            "lowercased"
        );
        assert_eq!(
            parse_sha256(&format!("\u{feff}{h}")).as_deref(),
            Some(h),
            "BOM"
        );
        assert_eq!(parse_sha256(""), None);
        assert_eq!(parse_sha256("deadbeef"), None);
        assert_eq!(parse_sha256(&format!("{}zz", &h[..62])), None);
        assert_eq!(parse_sha256(&format!("{h}00")), None, "too long");
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn dev_builds_are_found_by_path() {
        for p in [
            r"D:\Github\ai-subhub\target\debug\ai-subhub.exe",
            r"D:\Github\ai-subhub\target\release\ai-subhub.exe",
            r"D:\Github\ai-subhub\Target\Release\ai-subhub.exe",
            r"D:\x\target\x86_64-pc-windows-msvc\release\ai-subhub.exe",
        ] {
            assert!(is_dev_build(Path::new(p)), "{p}");
        }
        for p in [
            r"C:\Users\me\AppData\Local\Programs\AI SubHub\ai-subhub.exe",
            r"D:\Tools\ai-subhub.exe",
            r"D:\debug\ai-subhub.exe",
            r"D:\target\ai-subhub.exe",
            r"D:\release\target\ai-subhub.exe",
        ] {
            assert!(!is_dev_build(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn old_exe_name() {
        assert_eq!(
            old_path(Path::new(r"C:\Apps\ai-subhub.exe")),
            PathBuf::from(r"C:\Apps\ai-subhub.exe.old")
        );
    }

    #[test]
    fn stale_update_files() {
        let run = v("0.2.0");
        assert!(is_stale_update_file("ai-subhub-0.3.0.exe.part", &run));
        assert!(is_stale_update_file("ai-subhub-0.1.0.exe", &run));
        assert!(is_stale_update_file("ai-subhub-0.2.0.exe", &run));
        assert!(
            !is_stale_update_file("ai-subhub-0.2.1.exe", &run),
            "a newer download is kept"
        );
        assert!(
            !is_stale_update_file("readme.txt", &run),
            "other files are left alone"
        );
    }

    #[test]
    fn checks_are_spaced() {
        assert!(check_due(None, false), "first try of a run");
        assert!(!check_due(Some(Duration::from_secs(3600)), false));
        assert!(check_due(Some(CHECK_EVERY), false));
        assert!(!check_due(Some(Duration::from_secs(600)), true));
        assert!(check_due(Some(RETRY_AFTER_FAILURE), true));
    }

    #[test]
    fn restart_waits_until_idle() {
        let idle = Busy::default();
        assert!(idle.is_idle());
        assert!(auto_install_due(UpdateMode::Auto, false, true, &idle));
        // Each thing in use keeps it waiting.
        for busy in [
            Busy {
                popover_open: true,
                ..idle
            },
            Busy {
                settings_open: true,
                ..idle
            },
            Busy {
                stats_open: true,
                ..idle
            },
            Busy {
                notes_open: true,
                ..idle
            },
            Busy {
                signing_in: true,
                ..idle
            },
            Busy {
                popover_dragging: true,
                ..idle
            },
        ] {
            assert!(!busy.is_idle());
            assert!(
                !auto_install_due(UpdateMode::Auto, false, true, &busy),
                "{busy:?}"
            );
        }
        // Not without a verified download, never in Manual mode or a dev build.
        assert!(!auto_install_due(UpdateMode::Auto, false, false, &idle));
        assert!(!auto_install_due(UpdateMode::Manual, false, true, &idle));
        assert!(!auto_install_due(UpdateMode::Auto, true, true, &idle));
    }

    #[test]
    fn start_arguments() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let a = parse_start_args(&args(&[
            "app.exe",
            "--updated-from",
            "0.1.0",
            "--wait-pid",
            "4242",
        ]));
        assert_eq!(a.updated_from.as_deref(), Some("0.1.0"));
        assert_eq!(a.wait_pid, Some(4242));
        assert_eq!(parse_start_args(&args(&["app.exe"])), StartArgs::default());
        let bad = parse_start_args(&args(&[
            "app.exe",
            "--updated-from",
            "soon",
            "--wait-pid",
            "x",
        ]));
        assert_eq!(bad, StartArgs::default());
        // A flag with no value at the end.
        assert_eq!(
            parse_start_args(&args(&["app.exe", "--wait-pid"])),
            StartArgs::default()
        );
    }

    const LOG: &str =
        "## 0.1.2 – 2026-11-20\n- c\n## 0.1.1 – 2026-11-10\n- b\n## 0.1.0 – 2026-11-07\n- a\n";

    #[test]
    fn whats_new_after_an_update() {
        // Skipped versions are included, newest first.
        let w = whats_new("0.1.0", Some("0.1.0"), &v("0.1.2"), LOG);
        assert_eq!(w.sections.len(), 2);
        assert_eq!(w.sections[0].version, v("0.1.2"));
        assert!(w.mark_seen);

        // Fresh install: nothing saved, no dialog, just remember.
        let w = whats_new("", None, &v("0.1.0"), LOG);
        assert!(w.sections.is_empty() && w.mark_seen);

        // Nothing saved but started by an update: the argument is the baseline.
        let w = whats_new("", Some("0.1.0"), &v("0.1.1"), LOG);
        assert_eq!(w.sections.len(), 1);

        // Already seen: nothing to do.
        let w = whats_new("0.1.2", None, &v("0.1.2"), LOG);
        assert!(w.sections.is_empty() && !w.mark_seen);

        // A downgrade or garbage is remembered without a dialog.
        let w = whats_new("0.9.0", None, &v("0.1.2"), LOG);
        assert!(w.sections.is_empty() && w.mark_seen);
        let w = whats_new("garbage", None, &v("0.1.2"), LOG);
        assert!(w.sections.is_empty() && w.mark_seen);

        // No notes for the versions in between: no dialog.
        let w = whats_new("0.1.2", Some("0.1.2"), &v("0.1.3"), LOG);
        assert!(w.sections.is_empty() && w.mark_seen);
    }
}
