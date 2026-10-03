//! Local data directory and the skin file cache.
//!
//! Skins are small `.fantome` archives fetched on demand from the LeagueSkins repository
//! and kept under `<data>/skins/<champion id>/<skin id>.fantome`.

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const SKINS_BASE_URL: &str = "https://raw.githubusercontent.com/Alban1911/LeagueSkins/main/skins";

/// `$PURPLE_DATA_DIR`, or `%LOCALAPPDATA%\Purple`.
pub fn data_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("PURPLE_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not set")?;
    Ok(PathBuf::from(base).join("Purple"))
}

/// Appends a timestamped line (UTC) to `<data>/purple.log`. Best effort: the tray program has no
/// console, so this file is where its problems go.
pub fn log(line: &str) {
    use std::io::Write;
    let Ok(dir) = data_dir() else { return };
    let _ = fs::create_dir_all(&dir);
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let file = fs::OpenOptions::new().create(true).append(true).open(dir.join("purple.log"));
    if let Ok(mut file) = file {
        let _ = writeln!(file, "{} {line}", utc_timestamp(seconds));
    }
}

/// Starts the log afresh once it grows past 1 MiB.
pub fn trim_log() {
    if let Ok(dir) = data_dir() {
        let path = dir.join("purple.log");
        if fs::metadata(&path).is_ok_and(|m| m.len() > 1024 * 1024) {
            let _ = fs::remove_file(path);
        }
    }
}

/// `YYYY-MM-DD HH:MM:SS` for a Unix time (UTC), without a date library.
fn utc_timestamp(unix_seconds: u64) -> String {
    let (days, rest) = (unix_seconds / 86_400, unix_seconds % 86_400);
    // Days since 1970-01-01 -> civil date (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// Where overlays are built: `$PURPLE_DATA_DIR`, or `%PROGRAMDATA%\Purple`.
///
/// This must be outside the user profile. The game cannot traverse `C:\Users\<name>` (its ACL
/// grants only the account, SYSTEM and administrators), so an overlay under `%LOCALAPPDATA%` is
/// invisible to it and the patcher reports `overlay verified 0 wad(s)`. `ProgramData` is readable
/// by every user.
pub fn overlay_base_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("PURPLE_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base = std::env::var_os("PROGRAMDATA").context("PROGRAMDATA is not set")?;
    Ok(PathBuf::from(base).join("Purple"))
}

fn fantome_path(champion_id: u32, skin_id: u32) -> Result<PathBuf> {
    Ok(data_dir()?.join("skins").join(champion_id.to_string()).join(format!("{skin_id}.fantome")))
}

/// Returns the cached `.fantome` for a skin, downloading it first when it is not cached yet.
pub fn ensure_fantome(champion_id: u32, skin_id: u32) -> Result<PathBuf> {
    let path = fantome_path(champion_id, skin_id)?;
    if path.exists() {
        return Ok(path);
    }
    let url = format!("{SKINS_BASE_URL}/{champion_id}/{skin_id}/{skin_id}.fantome");
    let agent = ureq::AgentBuilder::new()
        .tls_connector(Arc::new(native_tls::TlsConnector::new()?))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build();
    let resp = match agent.get(&url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => {
            bail!("no skin file for {skin_id} in the LeagueSkins repository ({url})")
        }
        Err(e) => return Err(e).with_context(|| format!("downloading {url}")),
    };
    let mut bytes = Vec::new();
    resp.into_reader().take(64 * 1024 * 1024).read_to_end(&mut bytes)?;
    // A .fantome is a zip. Anything else (e.g. an HTML error page) must never reach the cache.
    if !bytes.starts_with(b"PK") {
        bail!("{url} did not return a zip archive");
    }
    fs::create_dir_all(path.parent().context("cache path has no parent")?)?;
    let part = path.with_extension("part");
    fs::write(&part, &bytes)?;
    fs::rename(&part, &path)?;
    Ok(path)
}

/// Removes every file in `dir` except the entry named `keep` (best effort: locked files stay).
pub fn prune_except(dir: &std::path::Path, keep: &str) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name() != keep {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::utc_timestamp;

    #[test]
    fn formats_known_instants() {
        assert_eq!(utc_timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29 00:00:00");
        // Cross-checked against Python's datetime.
        assert_eq!(utc_timestamp(1_791_050_096), "2026-10-03 17:54:56");
    }
}
