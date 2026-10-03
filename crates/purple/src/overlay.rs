//! Builds the WAD overlay for one skin with `ltk_overlay`.

use crate::store;
use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use ltk_overlay::{EnabledMod, FantomeContent, OverlayBuilder};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

/// Builds the overlay for `fantome` and returns the overlay root the patcher must serve.
///
/// The overlay lives under [`store::overlay_base_dir`], outside the user profile, because the
/// game cannot read inside it. Each skin gets its own `overlay/<skin id>` directory so a game that
/// still has the previous overlay open never races with the rebuild; older directories are pruned.
pub fn build(fantome: &Path, skin_id: u32, game_dir: &Path) -> Result<PathBuf> {
    let base = store::overlay_base_dir()?;
    let name = skin_id.to_string();
    let overlay_root = base.join("overlay").join(&name);
    let state_dir = base.join("state").join(&name);
    std::fs::create_dir_all(&overlay_root)?;
    std::fs::create_dir_all(&state_dir)?;

    let utf8 = |p: &Path| {
        Utf8PathBuf::from_path_buf(p.to_path_buf()).map_err(|p| anyhow::anyhow!("non-UTF-8 path: {}", p.display()))
    };
    let content = FantomeContent::new(BufReader::new(File::open(fantome).context("opening skin file")?))?
        .with_archive_path(utf8(fantome)?);

    let mut builder = OverlayBuilder::new(utf8(game_dir)?, utf8(&overlay_root)?, utf8(&state_dir)?);
    builder.set_enabled_mods(vec![EnabledMod {
        id: name.clone(),
        content: Box::new(content),
        enabled_layers: None,
    }]);
    builder.build().context("building overlay")?;

    store::prune_except(&base.join("overlay"), &name);
    store::prune_except(&base.join("state"), &name);
    Ok(overlay_root)
}
