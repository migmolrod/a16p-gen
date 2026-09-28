//! `a16p self-update`: upgrade a release-installed binary in place.
//!
//! Deliberately thin: finding the latest version is one `curl` redirect,
//! and the actual download/verify/replace is delegated to `install.sh`,
//! so there's exactly one updater implementation and no HTTP/TLS crates
//! in the binary. Needs `curl` and `sh` at runtime, same as installing.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const REPO: &str = "migmolrod/a16p-gen";

/// `A16P_RELEASES_URL` / `A16P_INSTALL_URL` are test hooks (like
/// `install.sh`'s `A16P_BASE_URL`): point them at a local server to
/// exercise the updater without GitHub.
fn releases_url() -> String {
    std::env::var("A16P_RELEASES_URL")
        .unwrap_or_else(|_| format!("https://github.com/{REPO}/releases"))
}

fn install_url() -> String {
    std::env::var("A16P_INSTALL_URL")
        .unwrap_or_else(|_| format!("https://raw.githubusercontent.com/{REPO}/master/install.sh"))
}

/// `major.minor.patch` of a plain release version. Pre-release/build
/// suffixes return `None` -- `releases/latest` never points at a
/// pre-release, so they can't show up as "latest" anyway.
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.strip_prefix('v').unwrap_or(v).split('.');
    let triple = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(triple)
}

/// `releases/latest` redirects to `.../releases/tag/<tag>`; with no
/// releases at all it lands back on `.../releases`, which has no tag.
fn tag_from_latest_url(url: &str) -> Option<&str> {
    let tag = url.trim_end_matches('/').rsplit_once("/tag/")?.1;
    (!tag.is_empty()).then_some(tag)
}

fn latest_tag() -> Result<String> {
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

pub fn run(check: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
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

    let tag = latest_tag()?;
    let latest = tag.strip_prefix('v').unwrap_or(&tag);
    let newer = match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => latest != current,
    };
    if !newer {
        println!("a16p {current} is up to date (latest release: {latest})");
        return Ok(());
    }
    if check {
        println!("a16p {latest} is available (installed: {current})");
        match refused {
            Some(reason) => println!("{reason}"),
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

    #[test]
    fn parse_version_accepts_plain_semver_with_or_without_v() {
        assert_eq!(parse_version("0.1.1"), Some((0, 1, 1)));
        assert_eq!(parse_version("v10.20.30"), Some((10, 20, 30)));
    }

    #[test]
    fn parse_version_rejects_prerelease_and_malformed() {
        assert_eq!(parse_version("1.2.0-rc.1"), None);
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn versions_compare_numerically_not_lexically() {
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
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
