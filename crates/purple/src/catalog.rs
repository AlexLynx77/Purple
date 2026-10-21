//! Skin and champion catalog, read from the client's own game-data assets and cached on disk
//! so the window still has something to show when the client is closed.

use crate::lcu::Lcu;
use crate::store;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

const ALTERNATE_MODE_ID_START: u32 = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skin {
    pub id: u32,
    pub champion_id: u32,
    pub name: String,
    pub is_base: bool,
    /// Client asset path of the skin's tile image.
    pub tile_path: String,
    /// Colour variants of this skin (the base skin can have some too).
    #[serde(default)]
    pub chromas: Vec<Chroma>,
}

/// A colour variant of a skin. It has its own skin id and is applied like a skin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chroma {
    pub id: u32,
    pub name: String,
    /// One or two colours (RGB) that represent it, as shown by the client.
    pub colors: Vec<[u8; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Champion {
    pub id: u32,
    pub name: String,
    pub alias: String,
    /// Client asset path of the champion's square portrait.
    pub icon_path: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Catalog {
    pub champions: Vec<Champion>,
    pub skins: Vec<Skin>,
}

fn cache_path() -> Result<PathBuf> {
    Ok(store::data_dir()?.join("catalog.json"))
}

impl Catalog {
    pub fn load(lcu: &Lcu) -> Result<Self> {
        let get = |path: &str| lcu.get(path)?.with_context(|| format!("{path} not available"));
        Self::from_sources(get)
    }

    /// The catalog read from the client's files on disk (works with the client closed).
    pub fn load_from_files() -> Result<Self> {
        let assets = std::cell::RefCell::new(crate::client_assets::ClientAssets::open()?);
        Self::from_sources(|path| Ok(serde_json::from_slice(&assets.borrow_mut().get(path)?)?))
    }

    fn from_sources(get: impl Fn(&str) -> Result<Value>) -> Result<Self> {
        let champs = get("/lol-game-data/assets/v1/champion-summary.json")?;
        let mut champions: Vec<Champion> = champs
            .as_array()
            .context("champion-summary is not an array")?
            .iter()
            .filter_map(parse_champion)
            .collect();
        champions.sort_by_key(|c| c.name.to_lowercase());

        let skins_json = get("/lol-game-data/assets/v1/skins.json")?;
        let mut skins: Vec<Skin> = skins_json
            .as_object()
            .context("skins.json is not an object")?
            .values()
            .filter_map(parse_skin)
            .collect();
        skins.sort_by_key(|s| s.id);
        let mut catalog = Self { champions, skins };
        catalog.retain_standard();
        Ok(catalog)
    }

    /// The catalog saved by the last successful [`Catalog::save`], if any.
    pub fn load_cached() -> Option<Self> {
        let text = std::fs::read_to_string(cache_path().ok()?).ok()?;
        let mut catalog: Self = serde_json::from_str(&text).ok()?;
        catalog.retain_standard();
        Some(catalog)
    }

    /// Drops the champions of the alternate game mode (ids 60000 and up, aliased `Jade_<Name>`).
    /// They duplicate every name of the standard roster and cannot be picked in a normal game.
    fn retain_standard(&mut self) {
        self.champions.retain(|c| c.id < ALTERNATE_MODE_ID_START);
        self.skins.retain(|s| s.champion_id < ALTERNATE_MODE_ID_START);
    }

    pub fn save(&self) -> Result<()> {
        let path = cache_path()?;
        std::fs::create_dir_all(path.parent().context("cache path has no parent")?)?;
        std::fs::write(path, serde_json::to_string(self)?)?;
        Ok(())
    }

    /// Finds a champion by numeric id, name or alias (case-insensitive).
    pub fn champion(&self, query: &str) -> Option<&Champion> {
        if let Ok(id) = query.parse::<u32>() {
            return self.champions.iter().find(|c| c.id == id);
        }
        let q = query.to_lowercase();
        self.champions
            .iter()
            .find(|c| c.name.to_lowercase() == q || c.alias.to_lowercase() == q)
    }

    pub fn skins_of(&self, champion_id: u32) -> impl Iterator<Item = &Skin> {
        self.skins.iter().filter(move |s| s.champion_id == champion_id)
    }

    /// The skin that `chroma_id` is a colour variant of, if it is a chroma.
    pub fn chroma_parent(&self, chroma_id: u32) -> Option<&Skin> {
        self.skins_of(chroma_id / 1000).find(|s| s.chromas.iter().any(|c| c.id == chroma_id))
    }

    /// The name of a skin or of a chroma.
    pub fn name_of(&self, id: u32) -> Option<&str> {
        let skin = self.skins.iter().find(|s| s.id == id).map(|s| s.name.as_str());
        skin.or_else(|| self.chroma_parent(id)?.chromas.iter().find(|c| c.id == id).map(|c| c.name.as_str()))
    }
}

/// The champion's alias (`MonkeyKing` for Wukong), which names its game archive: from the cached
/// catalog, else from the client when it is running.
pub fn champion_alias(champion_id: u32, lcu: Option<&Lcu>) -> Option<String> {
    let cached = Catalog::load_cached().and_then(|c| c.champion(&champion_id.to_string()).map(|c| c.alias.clone()));
    cached.or_else(|| {
        let champion = lcu?.get(&format!("/lol-game-data/assets/v1/champions/{champion_id}.json")).ok()??;
        Some(champion.get("alias")?.as_str()?.to_owned())
    })
}

fn parse_champion(v: &Value) -> Option<Champion> {
    let id = u32::try_from(v.get("id")?.as_i64()?).ok().filter(|id| *id > 0)?;
    Some(Champion {
        id,
        name: v.get("name")?.as_str()?.to_owned(),
        alias: v.get("alias")?.as_str()?.to_owned(),
        icon_path: v
            .get("squarePortraitPath")
            .and_then(Value::as_str)
            .map_or_else(|| format!("/lol-game-data/assets/v1/champion-icons/{id}.png"), str::to_owned),
    })
}

fn parse_skin(v: &Value) -> Option<Skin> {
    let id = u32::try_from(v.get("id")?.as_i64()?).ok()?;
    Some(Skin {
        id,
        // Skin ids are `champion_id * 1000 + index`.
        champion_id: id / 1000,
        name: v.get("name")?.as_str()?.to_owned(),
        is_base: v.get("isBase").and_then(Value::as_bool).unwrap_or(false),
        tile_path: v.get("tilePath").and_then(Value::as_str).unwrap_or_default().to_owned(),
        chromas: v.get("chromas").and_then(Value::as_array).into_iter().flatten().filter_map(parse_chroma).collect(),
    })
}

fn parse_chroma(v: &Value) -> Option<Chroma> {
    let colors = v
        .get("colors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| parse_hex_color(c.as_str()?))
        .collect();
    Some(Chroma { id: u32::try_from(v.get("id")?.as_i64()?).ok()?, name: v.get("name")?.as_str()?.to_owned(), colors })
}

/// `#RRGGBB` to RGB.
fn parse_hex_color(text: &str) -> Option<[u8; 3]> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_id_encodes_champion() {
        let v: Value = serde_json::json!({"id": 1001, "name": "Goth Annie", "isBase": false, "tilePath": "/x.jpg"});
        let s = parse_skin(&v).unwrap();
        assert_eq!((s.champion_id, s.id, s.name.as_str(), s.tile_path.as_str()), (1, 1001, "Goth Annie", "/x.jpg"));
    }

    #[test]
    fn reads_chromas_and_finds_their_skin() {
        let v: Value = serde_json::json!({"id": 107049, "name": "Victorious Rengar", "tilePath": "/t.jpg", "chromas": [
            {"id": 107050, "name": "Victorious Rengar (Bronze)", "colors": ["#764D43", "#764D43"]},
            {"id": 107051, "name": "Victorious Rengar (Silver)", "colors": ["nonsense"]}
        ]});
        let skin = parse_skin(&v).unwrap();
        assert_eq!(skin.chromas.len(), 2);
        assert_eq!(skin.chromas[0].colors, [[0x76, 0x4D, 0x43], [0x76, 0x4D, 0x43]]);
        assert!(skin.chromas[1].colors.is_empty());
        let catalog = Catalog { champions: vec![], skins: vec![skin] };
        assert_eq!(catalog.chroma_parent(107050).unwrap().id, 107049);
        assert!(catalog.chroma_parent(107049).is_none());
        assert_eq!(catalog.name_of(107051), Some("Victorious Rengar (Silver)"));
        assert_eq!(catalog.name_of(107049), Some("Victorious Rengar"));
    }

    #[test]
    fn champion_icon_falls_back_to_the_standard_path() {
        let v: Value = serde_json::json!({"id": 266, "name": "Aatrox", "alias": "Aatrox"});
        assert_eq!(parse_champion(&v).unwrap().icon_path, "/lol-game-data/assets/v1/champion-icons/266.png");
    }

    #[test]
    fn alternate_mode_champions_are_dropped() {
        let champion = |id, alias: &str| Champion { id, name: "Ahri".into(), alias: alias.into(), icon_path: String::new() };
        let skin = |id: u32| Skin { id, champion_id: id / 1000, name: String::new(), is_base: false, tile_path: String::new(), chromas: vec![] };
        let mut catalog = Catalog {
            champions: vec![champion(103, "Ahri"), champion(60103, "Jade_Ahri")],
            skins: vec![skin(103_001), skin(60_103_001)],
        };
        catalog.retain_standard();
        assert_eq!(catalog.champions.len(), 1);
        assert_eq!(catalog.skins.len(), 1);
    }

    #[test]
    fn catalog_survives_a_json_round_trip() {
        let catalog = Catalog {
            champions: vec![Champion { id: 1, name: "Annie".into(), alias: "Annie".into(), icon_path: "/i.png".into() }],
            skins: vec![Skin { id: 1001, champion_id: 1, name: "Goth".into(), is_base: false, tile_path: "/t.jpg".into(), chromas: vec![] }],
        };
        let back: Catalog = serde_json::from_str(&serde_json::to_string(&catalog).unwrap()).unwrap();
        assert_eq!(back.skins[0].id, 1001);
        assert_eq!(back.champion("annie").unwrap().id, 1);
    }
}
