//! User choices: which skin to apply for each champion. Stored in `<data>/config.json`.

use crate::store;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Champion id -> skin id to apply when that champion is picked.
    #[serde(default)]
    pub skins: BTreeMap<u32, u32>,
}

fn path() -> Result<PathBuf> {
    Ok(store::data_dir()?.join("config.json"))
}

impl Config {
    pub fn load() -> Result<Self> {
        match std::fs::read_to_string(path()?) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = path()?;
        std::fs::create_dir_all(path.parent().expect("config path has a parent"))?;
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}
