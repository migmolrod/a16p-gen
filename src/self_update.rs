//! `a16p self-update`: upgrade a release-installed binary in place.
//!
//! Deliberately thin: this side only decides *which* release to install,
//! and the actual download/verify/replace is delegated to `install.sh`
//! (pinned with `--version`), so there's exactly one installer
//! implementation and no HTTP/TLS crates in the binary. Needs `curl` and
//! `sh` at runtime, same as installing.
//!
//! Two ways to find the target release:
//! - stable: the `releases/latest` redirect (no API, no rate limit).
//!   GitHub never points it at a prerelease.
//! - prerelease channel (`--prerelease`, or automatically when the running
//!   binary is itself a prerelease): the releases API, taking the highest
//!   *semver* across stable and prerelease tags -- so an rc user moves on
//!   to the final release once it ships, and a hotfix on an older line
//!   published later never wins just by being newer.

use anyhow::{Context, Result, bail};
use semver::Version;
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const REPO: &str = "migmolrod/a16p-gen";
const DEFAULT_API_URL: &str = "https://api.github.com";

/// `A16P_RELEASES_URL` / `A16P_API_URL` / `A16P_INSTALL_URL` are test
/// hooks (like `install.sh`'s `A16P_BASE_URL`): point them at a local
/// server to exercise the updater without GitHub.
fn releases_url() -> String {
    std::env::var("A16P_RELEASES_URL")
        .unwrap_or_else(|_| format!("https://github.com/{REPO}/releases"))
}

fn api_url() -> String {
    std::env::var("A16P_API_URL").unwrap_or_else(|_| DEFAULT_API_URL.to_string())
}

fn install_url() -> String {
    std::env::var("A16P_INSTALL_URL")
        .unwrap_or_else(|_| format!("https://raw.githubusercontent.com/{REPO}/master/install.sh"))
}

/// Semver of a release tag (`v1.2.3`, `v1.2.0-rc.1`); `None` for tags that
/// aren't versions.
fn parse_tag(tag: &str) -> Option<Version> {
    Version::parse(tag.strip_prefix('v').unwrap_or(tag)).ok()
}

/// `releases/latest` redirects to `.../releases/tag/<tag>`; with no
/// releases at all it lands back on `.../releases`, which has no tag.
fn tag_from_latest_url(url: &str) -> Option<&str> {
    let tag = url.trim_end_matches('/').rsplit_once("/tag/")?.1;
    (!tag.is_empty()).then_some(tag)
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
}

/// Highest-semver published release, stable or prerelease.
fn newest_release(releases: &[ApiRelease]) -> Option<(&str, Version)> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| Some((r.tag_name.as_str(), parse_tag(&r.tag_name)?)))
        .max_by(|a, b| a.1.cmp(&b.1))
}

/// Prerelease channel is sticky without storing anything: a binary that is
/// itself a prerelease keeps following prereleases until it lands on a
/// stable version.
fn wants_prerelease(flag: bool, current: &Version) -> bool {
    flag || !current.pre.is_empty()
}

fn latest_stable_tag() -> Result<String> {
    let url = format!("{}/latest", releases_url());
    let out = Command::new("curl")
        .args(["-fsSLI", "-o", "/dev/null", "-w", "%{url_effective}", &url])
        .output()
        .context("failed to run curl (is it installed?)")?;
    if !out.status.success() {
        bail!("could not reach {url} ({})", out.status);
    }
    let effective = String::from_utf8_lossy(&out.stdout);
    tag_from_latest_url(&effective)
        .map(str::to_string)
        .with_context(|| format!("no published release found at {url}"))
}

fn newest_tag_including_prereleases() -> Result<String> {
    let api = api_url();
    let url = format!("{api}/repos/{REPO}/releases?per_page=100");
    // Unauthenticated calls get 60/hour, plenty for this. A GITHUB_TOKEN is
    // only sent to the real API (never to an A16P_API_URL override), and
    // over stdin (`-H @-`) so it doesn't show up in `ps`.
    let token = std::env::var("GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty() && api == DEFAULT_API_URL);
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "-H", "Accept: application/vnd.github+json"]);
    if token.is_some() {
        cmd.args(["-H", "@-"]).stdin(Stdio::piped());
    }
    let mut child = cmd
        .arg(&url)
        .stdout(Stdio::piped())
        .spawn()
        .context("failed to run curl (is it installed?)")?;
    if let (Some(token), Some(mut stdin)) = (token, child.stdin.take()) {
        writeln!(stdin, "Authorization: Bearer {token}")?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("could not reach {url} ({})", out.status);
    }
    let releases: Vec<ApiRelease> = serde_json::from_slice(&out.stdout)
        .with_context(|| format!("unexpected response from {url}"))?;
    newest_release(&releases)
        .map(|(tag, _)| tag.to_string())
        .with_context(|| format!("no published release found at {url}"))
}

/// Binaries this command must not replace: ones cargo manages (a release
/// tarball would silently shadow `cargo install`'s bookkeeping) and
/// development builds under `target/`.
fn refuse_reason(exe_dir: &Path, cargo_home: &Path) -> Option<String> {
    if exe_dir == cargo_home.join("bin") {
        return Some(format!(
            "this a16p was installed with cargo; update it with:\n    \
             cargo install --locked --force --git https://github.com/{REPO}"
        ));
    }
    let profile = exe_dir.file_name()?.to_str()?;
    let parent = exe_dir.parent()?.file_name()?.to_str()?;
    if parent == "target" && (profile == "debug" || profile == "release") {
        return Some("this is a development build under target/; not updating it".to_string());
    }
    None
}

fn cargo_home() -> PathBuf {
    match std::env::var("CARGO_HOME") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => crate::xdg::expand_tilde("~/.cargo"),
    }
}

pub fn run(check: bool, prerelease: bool) -> Result<()> {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).context("unparseable own version")?;
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("could not locate the running a16p binary")?;
    let exe_dir = exe
        .parent()
        .context("a16p binary has no parent directory")?;
    // Checked before any network access, so a refused update fails fast;
    // `--check` still reports, since knowing a release exists is useful
    // whichever way the binary was installed.
    let refused = refuse_reason(exe_dir, &cargo_home());
    if let Some(reason) = &refused
        && !check
    {
        bail!("{reason}");
    }

    let include_pre = wants_prerelease(prerelease, &current);
    let tag = if include_pre {
        newest_tag_including_prereleases()?
    } else {
        latest_stable_tag()?
    };
    let latest = parse_tag(&tag).with_context(|| format!("release tag {tag} is not semver"))?;
    // Strictly newer only: an rc build never "updates" back down to an
    // older stable release.
    if latest <= current {
        let channel = if include_pre {
            "newest release, prereleases included"
        } else {
            "newest stable release"
        };
        println!("a16p {current} is up to date ({channel}: {latest})");
        return Ok(());
    }
    if check {
        println!("a16p {latest} is available (installed: {current})");
        match refused {
            Some(reason) => println!("{reason}"),
            // Sticky prerelease binaries don't need the flag repeated.
            None if prerelease && current.pre.is_empty() => {
                println!("run `a16p self-update --prerelease` to install it")
            }
            None => println!("run `a16p self-update` to install it"),
        }
        return Ok(());
    }

    println!(
        "updating a16p {current} -> {latest} in {}",
        exe_dir.display()
    );
    // Paths/URLs go in as positional args rather than being spliced into
    // the script string, so no quoting issues with odd directory names.
    let status = Command::new("sh")
        .arg("-c")
        .arg(r#"curl -fsSL "$1" | sh -s -- --version "$2" --bin-dir "$3""#)
        .arg("sh")
        .arg(install_url())
        .arg(&tag)
        .arg(exe_dir)
        .status()
        .context("failed to run sh")?;
    if !status.success() {
        bail!("install.sh failed ({status})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> ApiRelease {
        ApiRelease {
            tag_name: tag.to_string(),
            draft: false,
        }
    }

    #[test]
    fn parse_tag_accepts_stable_and_prerelease_with_or_without_v() {
        assert_eq!(parse_tag("v0.1.1"), Some(Version::new(0, 1, 1)));
        assert_eq!(parse_tag("10.20.30"), Some(Version::new(10, 20, 30)));
        assert!(parse_tag("v1.2.0-rc.1").is_some());
        assert_eq!(parse_tag("v1.2"), None);
        assert_eq!(parse_tag("latest"), None);
    }

    #[test]
    fn semver_ordering_handles_prereleases() {
        let v = |s| parse_tag(s).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("0.2.0-rc.10") > v("0.2.0-rc.2"));
        assert!(v("0.2.0-rc.1") > v("0.2.0-beta.3"));
        assert!(v("0.2.0") > v("0.2.0-rc.10"));
        assert!(v("0.2.0-rc.1") > v("0.1.9"));
    }

    #[test]
    fn newest_release_is_highest_semver_not_most_recent() {
        // API order is newest-published first: the 0.1.5 hotfix came out
        // after 0.2.0-rc.1 but must not win.
        let releases = [release("v0.1.5"), release("v0.2.0-rc.1"), release("v0.1.4")];
        assert_eq!(newest_release(&releases).unwrap().0, "v0.2.0-rc.1");
    }

    #[test]
    fn newest_release_prefers_final_over_its_rcs() {
        let releases = [
            release("v0.2.0-rc.2"),
            release("v0.2.0"),
            release("v0.2.0-rc.1"),
        ];
        assert_eq!(newest_release(&releases).unwrap().0, "v0.2.0");
    }

    #[test]
    fn newest_release_skips_drafts_and_non_version_tags() {
        let mut draft = release("v9.0.0");
        draft.draft = true;
        let releases = [draft, release("nightly"), release("v0.1.2")];
        assert_eq!(newest_release(&releases).unwrap().0, "v0.1.2");
        assert!(newest_release(&[]).is_none());
    }

    #[test]
    fn prerelease_channel_is_sticky_for_prerelease_builds() {
        assert!(!wants_prerelease(false, &Version::parse("0.1.3").unwrap()));
        assert!(wants_prerelease(true, &Version::parse("0.1.3").unwrap()));
        assert!(wants_prerelease(
            false,
            &Version::parse("0.2.0-rc.1").unwrap()
        ));
    }

    #[test]
    fn tag_from_latest_url_extracts_tag() {
        let url = "https://github.com/migmolrod/a16p-gen/releases/tag/v0.1.1";
        assert_eq!(tag_from_latest_url(url), Some("v0.1.1"));
        assert_eq!(tag_from_latest_url(&format!("{url}/")), Some("v0.1.1"));
    }

    #[test]
    fn tag_from_latest_url_none_without_releases() {
        let url = "https://github.com/migmolrod/a16p-gen/releases";
        assert_eq!(tag_from_latest_url(url), None);
    }

    #[test]
    fn refuses_cargo_installed_binary() {
        let home = Path::new("/home/u/.cargo");
        assert!(refuse_reason(&home.join("bin"), home).is_some());
    }

    #[test]
    fn refuses_development_builds() {
        let home = Path::new("/home/u/.cargo");
        assert!(refuse_reason(Path::new("/src/a16p-gen/target/debug"), home).is_some());
        assert!(refuse_reason(Path::new("/src/a16p-gen/target/release"), home).is_some());
    }

    #[test]
    fn allows_release_installs() {
        let home = Path::new("/home/u/.cargo");
        assert!(refuse_reason(Path::new("/home/u/.local/bin"), home).is_none());
        assert!(refuse_reason(Path::new("/usr/local/bin"), home).is_none());
    }
}
