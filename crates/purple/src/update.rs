//! Keeps Purple itself up to date from its GitHub releases.
//!
//! The resident program asks GitHub for the latest release when it starts, then once a day. A newer
//! release shows in the window, whose button signals [`REQUEST_EVENT`]. The resident program then
//! downloads that release's `purple.exe` and swaps it in for the running one with `self-replace`:
//! the running file is moved aside and deleted once Purple has exited. Purple then starts again from
//! the new file with [`UPDATED_ARG`]. Only one `purple.exe` is left.
//!
//! `$PURPLE_UPDATE_URL` replaces the GitHub API address of the latest release (for tests).

use crate::patcher_install::agent;
use crate::store;
use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const LATEST_URL: &str = "https://api.github.com/repos/AlexLynx77/Purple/releases/latest";
/// Name of the program in each release.
const ASSET: &str = "purple.exe";
/// Named event the window signals to ask the resident program for the update.
pub const REQUEST_EVENT: &str = "Local\\Purple.Update";
/// Argument of the first start after an update: wait for the previous Purple to finish exiting.
pub const UPDATED_ARG: &str = "--updated";
/// Where the download goes, next to `purple.exe` (so writing there is known to work before the swap).
const DOWNLOAD: &str = ".purple-update.part";

/// A release newer than the running program.
#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    url: String,
    size: u64,
}

/// `"v1.2.3"` or `"1.2.3"` as numbers.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(c), Some(r)) if c > r)
}

/// The latest release when it is newer than this program, `None` when Purple is up to date.
pub fn check() -> Result<Option<Release>> {
    let url = std::env::var("PURPLE_UPDATE_URL").unwrap_or_else(|_| LATEST_URL.to_owned());
    let release: serde_json::Value = agent()?.get(&url).call().with_context(|| format!("asking GitHub for {url}"))?.into_json()?;
    let tag = release["tag_name"].as_str().context("no tag_name in the GitHub answer")?;
    if !is_newer(tag, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let asset = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str() == Some(ASSET))
        .with_context(|| format!("release {tag} has no {ASSET}"))?;
    Ok(Some(Release {
        version: tag.trim_start_matches('v').to_owned(),
        url: asset["browser_download_url"].as_str().context("no download address for the release")?.to_owned(),
        size: asset["size"].as_u64().context("no size for the release")?,
    }))
}

/// Downloads `release` and puts it in place of the running `purple.exe`. Returns the path to start.
///
/// The running program keeps working from its moved-aside file until it exits.
pub fn install(release: &Release) -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locating the running program")?;
    let dir = exe.parent().context("the program's path has no parent")?;
    let mut bytes = Vec::new();
    agent()?
        .get(&release.url)
        .call()
        .with_context(|| format!("downloading Purple {}", release.version))?
        .into_reader()
        .take(128 * 1024 * 1024)
        .read_to_end(&mut bytes)?;
    // A cut download or an error page must never replace the program.
    if bytes.len() as u64 != release.size || !bytes.starts_with(b"MZ") {
        bail!("the downloaded file is not Purple {} ({} bytes)", release.version, bytes.len());
    }
    let download = dir.join(DOWNLOAD);
    fs::write(&download, &bytes).with_context(|| format!("Purple cannot write to {}", dir.display()))?;
    let replaced = self_replace::self_replace(&download);
    let _ = fs::remove_file(&download);
    replaced.context("replacing purple.exe")?;
    store::log(&format!("updated to Purple {}", release.version));
    Ok(exe)
}

/// Removes what an update left behind (best effort: files still in use stay).
///
/// `self-replace` deletes the previous program once it has exited, but the helper it runs for that
/// (a copy in `%TEMP%`) cannot delete itself, and an old window slow to close keeps the previous
/// program open.
pub fn remove_leftovers() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(stem) = exe.file_stem().and_then(|s| s.to_str()) else { return };
    let prefix = format!(".{stem}.");
    let mut dirs = vec![std::env::temp_dir()];
    if let Some(dir) = exe.parent() {
        let _ = fs::remove_file(dir.join(DOWNLOAD));
        dirs.push(dir.to_path_buf());
    }
    for dir in dirs {
        remove_matching(&dir, &prefix);
    }
}

fn remove_matching(dir: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Names self-replace gives the moved-aside program, its deletion helper and its temporary copy.
        let suffixes = [".__relocated__.exe", ".__selfdelete__.exe", ".__temp__.exe"];
        if name.starts_with(prefix) && suffixes.iter().any(|s| name.ends_with(s)) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_as_numbers() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.2.1"));
        assert!(is_newer("0.2.2", "0.2.1"));
        assert!(!is_newer("v0.2.1", "0.2.1"));
        assert!(!is_newer("v0.2.0", "0.2.1"));
    }

    #[test]
    fn ignores_tags_that_are_not_versions() {
        assert!(!is_newer("nightly", "0.2.1"));
        assert!(!is_newer("v1.0", "0.2.1"));
        assert!(!is_newer("v1.0.0-beta", "0.2.1"));
        assert!(!is_newer("v1.0.0.1", "0.2.1"));
    }

    #[test]
    fn removes_only_update_leftovers() {
        let dir = std::env::temp_dir().join(format!("purple-update-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let names = [
            ".purple.abc.__relocated__.exe",
            ".purple.abc.__selfdelete__.exe",
            ".purple.abc.__temp__.exe",
            ".other.abc.__relocated__.exe",
            "purple.exe",
        ];
        for name in names {
            fs::write(dir.join(name), b"x").unwrap();
        }
        remove_matching(&dir, ".purple.");
        let left: Vec<bool> = names.iter().map(|n| dir.join(n).exists()).collect();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(left, [false, false, false, true, true]);
    }
}
