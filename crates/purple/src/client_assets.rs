//! Reads the League client's game data (champion list, skins, pictures) straight from its files.
//!
//! The client serves `/lol-game-data/assets/...` out of the archives in
//! `<install>/Plugins/rcp-be-lol-game-data/` (`default-assets*.wad` plus one per language, e.g.
//! `fr_FR-assets.wad`). Reading them directly works with the client closed and needs no network.

use crate::lcu;
use anyhow::{bail, Context, Result};
use ltk_hash::Hash as _;
use ltk_wad::{Wad, WadHash};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

const PLUGIN_DIR: &str = "Plugins/rcp-be-lol-game-data";
const CLIENT_PREFIX: &str = "/lol-game-data/assets/";
const ARCHIVE_PREFIX: &str = "plugins/rcp-be-lol-game-data/global";

pub struct ClientAssets {
    archives: Vec<Wad<BufReader<File>>>,
    /// Lower-case language of the installed client (`fr_fr`), when there is a language archive.
    locale: Option<String>,
}

impl ClientAssets {
    /// Opens the game-data archives of the first League install found.
    pub fn open() -> Result<Self> {
        let dir = lcu::install_dirs()
            .into_iter()
            .map(|d| d.join(PLUGIN_DIR))
            .find(|d| d.is_dir())
            .context("League of Legends client files not found")?;
        Self::open_dir(&dir)
    }

    fn open_dir(dir: &Path) -> Result<Self> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wad")))
            .collect();
        // Language archives first: their files override the default ones.
        paths.sort_by_key(|p| file_name(p).starts_with("default"));
        let locale = paths
            .iter()
            .map(|p| file_name(p))
            .find(|n| !n.starts_with("default"))
            .and_then(|n| n.strip_suffix("-assets.wad").map(str::to_lowercase));
        let mut archives = Vec::new();
        for path in paths {
            let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
            archives.push(Wad::mount(BufReader::new(file)).with_context(|| format!("reading {}", path.display()))?);
        }
        if archives.is_empty() {
            bail!("no archive in {}", dir.display());
        }
        Ok(Self { archives, locale })
    }

    /// The bytes of a client game-data path (`/lol-game-data/assets/...`).
    pub fn get(&mut self, client_path: &str) -> Result<Vec<u8>> {
        let rest = client_path.strip_prefix(CLIENT_PREFIX).with_context(|| format!("not a game-data path: {client_path}"))?;
        let rest = rest.to_lowercase();
        let mut candidates = Vec::new();
        if let Some(locale) = &self.locale {
            candidates.push(format!("{ARCHIVE_PREFIX}/{locale}/{rest}"));
        }
        candidates.push(format!("{ARCHIVE_PREFIX}/default/{rest}"));
        for candidate in &candidates {
            let hash = WadHash::hash_str(candidate);
            for archive in &mut self.archives {
                if let Some(chunk) = archive.chunks().get(hash).cloned() {
                    return Ok(archive.load_chunk_decompressed(&chunk)?.into_vec());
                }
            }
        }
        bail!("{client_path} is not in the client's files")
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::ClientAssets;

    /// Runs only on a machine with League installed.
    #[test]
    fn reads_the_champion_list_and_a_picture_from_an_installed_client() {
        if !crate::lcu::install_dirs().iter().any(|d| d.join(super::PLUGIN_DIR).is_dir()) {
            return;
        }
        let mut assets = ClientAssets::open().unwrap();
        assert_eq!(assets.locale.as_deref(), Some("fr_fr"), "this machine's client is in French");
        let summary = assets.get("/lol-game-data/assets/v1/champion-summary.json").unwrap();
        let champions: serde_json::Value = serde_json::from_slice(&summary).unwrap();
        assert!(champions.as_array().unwrap().len() > 100);
        let portrait = assets.get("/lol-game-data/assets/v1/champion-icons/1.png").unwrap();
        assert!(portrait.starts_with(b"\x89PNG"));
        let tile = assets.get("/lol-game-data/assets/ASSETS/Characters/Annie/Skins/Base/Images/annie_splash_tile_0.jpg").unwrap();
        assert!(tile.starts_with(&[0xFF, 0xD8]), "a JPEG");
    }
}
