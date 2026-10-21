//! Local data directory and the skin file cache.
//!
//! Skins are small `.fantome` archives. Purple generates them from the game's own files
//! ([`crate::skingen`]) and keeps them under `<data>/skins/<champion id>/<skin id>.gen.fantome`, with
//! a stamp of the game files they came from so a game patch regenerates them. When generation
//! fails, the file is fetched from the LeagueSkins repository instead and kept as
//! `<skin id>.fantome` (with its `ETag` next to it, so a file regenerated upstream for a new game
//! patch is fetched again). `PURPLE_SKIN_SOURCE=leagueskins` reverses that order.

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
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

/// Where a skin file comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinSource {
    /// Generated from the game's files.
    Generated,
    /// Downloaded from the LeagueSkins repository.
    LeagueSkins,
}

/// Returns the `.fantome` for a skin and where it came from.
///
/// It is generated from the game in `game_dir` (no network), or downloaded from LeagueSkins when
/// that fails, for instance when `alias` (the champion's archive name) is unknown. With
/// `PURPLE_SKIN_SOURCE=leagueskins`, LeagueSkins is tried first: a few of its files carry
/// hand-made fixes (animations, sounds) that cannot be derived from the game.
///
/// `parent` is the skin a chroma belongs to (LeagueSkins files chromas under it), `None` for a skin.
pub fn ensure_fantome(
    game_dir: &Path,
    alias: Option<&str>,
    champion_id: u32,
    skin_id: u32,
    parent: Option<u32>,
) -> Result<(PathBuf, SkinSource)> {
    let generate = || -> Result<PathBuf> {
        let alias = alias.context("the champion's name is unknown (no catalog yet and no client)")?;
        ensure_generated(game_dir, alias, champion_id, skin_id)
    };
    let prefer_download = std::env::var("PURPLE_SKIN_SOURCE").is_ok_and(|v| v.eq_ignore_ascii_case("leagueskins"));
    let (first, second) = if prefer_download {
        (SkinSource::LeagueSkins, SkinSource::Generated)
    } else {
        (SkinSource::Generated, SkinSource::LeagueSkins)
    };
    let get = |source| match source {
        SkinSource::Generated => generate(),
        SkinSource::LeagueSkins => ensure_downloaded(champion_id, skin_id, parent),
    };
    match get(first) {
        Ok(path) => Ok((path, first)),
        Err(first_error) => {
            log(&format!("skin {skin_id}: {first:?} failed, trying {second:?}: {first_error:#}"));
            match get(second) {
                Ok(path) => Ok((path, second)),
                Err(second_error) => Err(second_error.context(format!("{first:?}: {first_error:#}"))),
            }
        }
    }
}

/// Bumped whenever [`crate::skingen`] changes what it writes, so cached files are regenerated.
const GENERATOR_VERSION: u32 = 1;

/// The generated `.fantome` for a skin, made again when the game files it comes from changed.
fn ensure_generated(game_dir: &Path, alias: &str, champion_id: u32, skin_id: u32) -> Result<PathBuf> {
    let path = fantome_path(champion_id, skin_id)?.with_extension("gen.fantome");
    let stamp_path = path.with_extension("stamp");
    let stamp = generation_stamp(game_dir, alias)?;
    if path.exists() && fs::read_to_string(&stamp_path).is_ok_and(|s| s == stamp) {
        return Ok(path);
    }
    let bytes = crate::skingen::generate(game_dir, alias, skin_id, &format!("Skin {skin_id}"))?;
    fs::create_dir_all(path.parent().context("cache path has no parent")?)?;
    let part = path.with_extension("part");
    fs::write(&part, &bytes)?;
    fs::rename(&part, &path)?;
    fs::write(&stamp_path, stamp)?;
    Ok(path)
}

/// Identifies the game files a generated skin is made from (their size and modification time).
fn generation_stamp(game_dir: &Path, alias: &str) -> Result<String> {
    let mut stamp = format!("v{GENERATOR_VERSION}");
    let ui = game_dir.join("DATA").join("FINAL").join("UI.wad.client");
    for (file, required) in [(crate::skingen::champion_wad(game_dir, alias), true), (ui, false)] {
        match fs::metadata(&file) {
            Ok(meta) => {
                let modified = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
                stamp.push_str(&format!(" {}:{}", meta.len(), modified.map_or(0, |d| d.as_secs())));
            }
            Err(e) if required => return Err(e).with_context(|| format!("no game archive {}", file.display())),
            Err(_) => stamp.push_str(" -"),
        }
    }
    Ok(stamp)
}

/// Returns the LeagueSkins `.fantome` for a skin, downloading it when it is missing or has changed
/// upstream.
///
/// LeagueSkins regenerates its files for every game patch, so a cached copy can go stale. Each
/// call asks GitHub whether the file changed (conditional request on the saved `ETag`, ~1 small
/// request per arm). When GitHub cannot be reached, the cached copy is used as is.
fn ensure_downloaded(champion_id: u32, skin_id: u32, parent: Option<u32>) -> Result<PathBuf> {
    let path = fantome_path(champion_id, skin_id)?;
    let etag_path = path.with_extension("etag");
    let cached_etag = if path.exists() { fs::read_to_string(&etag_path).ok() } else { None };
    match download_fantome(champion_id, skin_id, parent, cached_etag.as_deref().map(str::trim)) {
        Ok(Download::Unchanged) => Ok(path),
        Ok(Download::New { bytes, etag }) => {
            fs::create_dir_all(path.parent().context("cache path has no parent")?)?;
            let part = path.with_extension("part");
            fs::write(&part, &bytes)?;
            fs::rename(&part, &path)?;
            match etag {
                Some(etag) => fs::write(&etag_path, etag)?,
                None => {
                    let _ = fs::remove_file(&etag_path);
                }
            }
            Ok(path)
        }
        Err(e) if path.exists() => {
            log(&format!("using cached skin {skin_id}, could not check for an update: {e:#}"));
            Ok(path)
        }
        Err(e) => Err(e),
    }
}

enum Download {
    Unchanged,
    New { bytes: Vec<u8>, etag: Option<String> },
}

fn download_fantome(champion_id: u32, skin_id: u32, parent: Option<u32>, etag: Option<&str>) -> Result<Download> {
    // Chromas live in their skin's folder: `<champion>/<skin>/<chroma>/<chroma>.fantome`.
    let folder = match parent {
        Some(parent) => format!("{champion_id}/{parent}/{skin_id}"),
        None => format!("{champion_id}/{skin_id}"),
    };
    let url = format!("{SKINS_BASE_URL}/{folder}/{skin_id}.fantome");
    let agent = ureq::AgentBuilder::new()
        .tls_connector(Arc::new(native_tls::TlsConnector::new()?))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build();
    let mut request = agent.get(&url);
    if let Some(etag) = etag {
        request = request.set("If-None-Match", etag);
    }
    let resp = match request.call() {
        Ok(r) if r.status() == 304 => return Ok(Download::Unchanged),
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => {
            bail!("no skin file for {skin_id} in the LeagueSkins repository ({url})")
        }
        Err(e) => return Err(e).with_context(|| format!("downloading {url}")),
    };
    let etag = resp.header("ETag").map(str::to_owned);
    let mut bytes = Vec::new();
    resp.into_reader().take(64 * 1024 * 1024).read_to_end(&mut bytes)?;
    // A .fantome is a zip. Anything else (e.g. an HTML error page) must never reach the cache.
    if !bytes.starts_with(b"PK") {
        bail!("{url} did not return a zip archive");
    }
    Ok(Download::New { bytes, etag })
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
