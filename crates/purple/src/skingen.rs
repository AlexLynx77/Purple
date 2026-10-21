//! Generates a skin's `.fantome` from the game's own files, so skins need no download.
//!
//! A skin file only redirects: for each character of the champion (the champion itself, and
//! characters it brings along such as Annie's Tibbers or Naafiri's packmates), it holds a
//! `data/characters/<name>/skins/skin0.bin` made of two objects copied from the game's
//! `skins/skin<N>.bin`, `Characters/<name>/Skins/Skin<N>` and `.../Skin<N>/Resources`, renamed to
//! `Skin0`, with `DATA/Characters/<name>/Skins/Skin<N>.bin` as their dependency (plus, for the
//! champion, the skin's gameplay bins from `UI.wad.client`). The game then loads skin N's data
//! whenever it asks for the base skin. Models, textures and effects stay in the game's archive.
//!
//! This is what the LeagueSkins files contain: on 374 skins (patch 16.21), 359 came out byte for
//! byte the same. The others are hand-made files of that repository (full bins from an older patch,
//! animation or sound overrides) that cannot be derived from the game.

use anyhow::{bail, Context, Result};
use ltk_hash::{BinHash, Hash, WadHash};
use ltk_meta::{Bin, BinFile, BinObject};
use ltk_wad::{Wad, WadBuilder, WadChunkBuilder, WadChunkCompression};
use std::collections::BTreeMap;
use std::io::{BufReader, Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};

/// The champion's archive in a game directory (`<game>/DATA/FINAL/Champions/<alias>.wad.client`).
pub fn champion_wad(game_dir: &Path, alias: &str) -> PathBuf {
    game_dir.join("DATA").join("FINAL").join("Champions").join(format!("{alias}.wad.client"))
}

/// Builds the `.fantome` (a zip) for `skin_id` from the champion's archive in `game_dir`.
pub fn generate(game_dir: &Path, alias: &str, skin_id: u32, display_name: &str) -> Result<Vec<u8>> {
    let index = skin_index(skin_id)?;
    let wad_path = champion_wad(game_dir, alias);
    let mut wad = mount(&wad_path)?;
    let ui_wad = game_dir.join("DATA").join("FINAL").join("UI.wad.client");
    let gameplay = match mount(&ui_wad) {
        Ok(ui) => gameplay_bins(&ui, alias, index),
        Err(e) => {
            crate::store::log(&format!("skin {skin_id}: no gameplay bins ({e:#})"));
            Vec::new()
        }
    };
    let bins = skin_bins(&mut wad, alias, index, &gameplay)?;
    fantome(alias, display_name, &bins)
}

fn mount(path: &Path) -> Result<Wad<BufReader<std::fs::File>>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    Wad::mount(BufReader::new(file)).with_context(|| format!("reading {}", path.display()))
}

/// The gameplay bins the game keeps for skin `index` in `UI.wad.client` (a skin's own HUD, for
/// instance). The game finds them by the skin's number, so a skin shown as the base skin must
/// name them as dependencies. They are named `gameplay.<champion>skin<N>[viewcontroller].bin`.
fn gameplay_bins<R: Read + Seek>(ui: &Wad<R>, alias: &str, index: u32) -> Vec<String> {
    let lower = alias.to_ascii_lowercase();
    ["", "viewcontroller"]
        .into_iter()
        .map(|suffix| format!("gameplay.{lower}skin{index}{suffix}.bin"))
        .filter(|name| ui.chunks().contains(WadHash::hash_str(name)))
        .collect()
}

/// The skin's index within its champion: skin ids are `champion id * 1000 + index`.
fn skin_index(skin_id: u32) -> Result<u32> {
    match skin_id % 1000 {
        0 => bail!("skin {skin_id} is a base skin, there is nothing to generate"),
        index => Ok(index),
    }
}

/// The `skin0.bin` of every character of the archive that has a skin `index`, keyed by chunk path.
/// The champion's also depends on `gameplay`. Fails when the champion itself has no skin `index`.
fn skin_bins<R: Read + Seek>(
    wad: &mut Wad<R>,
    alias: &str,
    index: u32,
    gameplay: &[String],
) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut bins = BTreeMap::new();
    for character in characters(wad, alias) {
        let lower = character.to_ascii_lowercase();
        let source = format!("data/characters/{lower}/skins/skin{index}.bin");
        let target = format!("data/characters/{lower}/skins/skin0.bin");
        let chunks = wad.chunks();
        let (Some(chunk), true) = (chunks.get(WadHash::hash_str(&source)).copied(), chunks.contains(WadHash::hash_str(&target)))
        else {
            continue;
        };
        let data = wad.load_chunk_decompressed(&chunk).with_context(|| format!("reading {source}"))?;
        let bin = BinFile::from_reader(&mut Cursor::new(&data[..]))
            .with_context(|| format!("parsing {source}"))?
            .into_prop()
            .with_context(|| format!("{source} is not a property bin"))?;
        if let Some(mut redirect) = redirect(&bin, &character, index) {
            if character == alias {
                redirect.dependencies.extend_from_slice(gameplay);
            }
            let mut bytes = Cursor::new(Vec::new());
            redirect.to_writer(&mut bytes)?;
            bins.insert(target, bytes.into_inner());
        }
    }
    if !bins.keys().any(|path| path.starts_with(&format!("data/characters/{}/", alias.to_ascii_lowercase()))) {
        bail!("the game has no skin {index} for {alias}");
    }
    Ok(bins)
}

/// The champion and every character the archive's property bins depend on (`DATA/Characters/<name>/...`),
/// without duplicates. The champion keeps its alias's spelling and the others are lower case, like in
/// the LeagueSkins files (the game ignores case), so both give the very same overlay.
fn characters<R: Read + Seek>(wad: &mut Wad<R>, alias: &str) -> Vec<String> {
    let mut names = vec![alias.to_owned()];
    let chunks: Vec<_> = wad.chunks().iter().copied().collect();
    for chunk in chunks {
        // Dependencies are listed at the very start of a bin: its first bytes are enough.
        let Ok(raw) = wad.load_chunk_raw_prefix(&chunk, 16 * 1024) else { continue };
        let Ok(head) = ltk_wad::decompress_prefix(&raw, chunk.compression_type(), 8 * 1024) else { continue };
        for dependency in bin_dependencies(&head) {
            let mut parts = dependency.split(['/', '\\']);
            if let (Some(data), Some(characters), Some(name)) = (parts.next(), parts.next(), parts.next()) {
                if data.eq_ignore_ascii_case("data")
                    && characters.eq_ignore_ascii_case("characters")
                    && !name.is_empty()
                    && !is_alternate_mode(name)
                    && !names.iter().any(|n| n.eq_ignore_ascii_case(name))
                {
                    names.push(name.to_ascii_lowercase());
                }
            }
        }
    }
    names
}

/// The alternate game mode's copies of the characters (`Jade_<name>`), which share the champion's
/// archive. LeagueSkins leaves them alone, and so does Purple (see `catalog`).
fn is_alternate_mode(character: &str) -> bool {
    character.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("jade_"))
}

/// The dependencies a `PROP` bin lists in its header (as many as `head` holds whole).
fn bin_dependencies(head: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    if head.len() < 12 || &head[..4] != b"PROP" {
        return found;
    }
    let read_u32 = |at: usize| head.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let version = read_u32(4).unwrap_or(0);
    if version < 2 {
        return found;
    }
    let count = read_u32(8).unwrap_or(0);
    let mut at = 12;
    for _ in 0..count {
        let Some(len) = head.get(at..at + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]]))) else { break };
        let Some(text) = head.get(at + 2..at + 2 + len) else { break };
        found.push(String::from_utf8_lossy(text).into_owned());
        at += 2 + len;
    }
    found
}

/// The `skin0.bin` that sends `character`'s base skin to skin `index`, from that skin's bin.
/// `None` when the bin does not hold the skin's data object.
fn redirect(skin_bin: &Bin, character: &str, index: u32) -> Option<Bin> {
    let path = |skin: u32, suffix: &str| BinHash::hash_str(format!("Characters/{character}/Skins/Skin{skin}{suffix}"));
    let mut objects = Vec::new();
    for suffix in ["", "/Resources"] {
        if let Some(object) = skin_bin.objects.get(&path(index, suffix)) {
            objects.push(BinObject { path_hash: path(0, suffix), ..object.clone() });
        } else if suffix.is_empty() {
            return None;
        }
    }
    Some(Bin::new(objects, [format!("DATA/Characters/{character}/Skins/Skin{index}.bin")]))
}

/// A `.fantome` holding `bins` (uncompressed, as LeagueSkins stores them) in `WAD/<alias>.wad.client`.
fn fantome(alias: &str, display_name: &str, bins: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut builder = WadBuilder::default();
    for path in bins.keys() {
        builder = builder.with_chunk(WadChunkBuilder::default().with_path(path).with_force_compression(WadChunkCompression::None));
    }
    let by_hash: BTreeMap<WadHash, &Vec<u8>> = bins.iter().map(|(path, data)| (WadHash::hash_str(path), data)).collect();
    let mut wad = Cursor::new(Vec::new());
    builder.build_to_writer(&mut wad, |hash, out| {
        out.write_all(by_hash.get(&hash).expect("every chunk has its bytes"))?;
        Ok(())
    })?;

    let info = serde_json::json!({
        "Name": display_name,
        "Author": "Purple",
        "Description": "Generated from the game's files",
        "Version": "1.0.0",
    });
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(format!("WAD/{alias}.wad.client"), stored)?;
    zip.write_all(wad.get_ref())?;
    zip.start_file("META/info.json", stored)?;
    zip.write_all(serde_json::to_string_pretty(&info)?.as_bytes())?;
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ltk_meta::property::values::I32;

    /// A bin like the game's `skins/skin<index>.bin`: the skin's data, its resources, and an
    /// unrelated object.
    fn game_skin_bin(character: &str, index: u32) -> Bin {
        let object = |path: String, class: u32, value: i32| {
            let mut object = BinObject::new(BinHash::hash_str(path), class);
            object.insert(BinHash(1), I32::new(value));
            object
        };
        Bin::new(
            [
                object(format!("Characters/{character}/Skins/Skin{index}"), 0x9b67_e9f6, 7),
                object("Some/Particle".into(), 0x45cd_899f, 8),
                object(format!("Characters/{character}/Skins/Skin{index}/Resources"), 0xef3a_0f33, 9),
            ],
            [format!("DATA/Characters/{character}/{character}.bin")],
        )
    }

    fn bytes(bin: &Bin) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        bin.to_writer(&mut out).unwrap();
        out.into_inner()
    }

    #[test]
    fn redirect_renames_the_two_skin_objects_to_skin0() {
        let bin = redirect(&game_skin_bin("Annie", 1), "Annie", 1).unwrap();
        let paths: Vec<_> = bin.objects.keys().copied().collect();
        assert_eq!(
            paths,
            [BinHash::hash_str("Characters/Annie/Skins/Skin0"), BinHash::hash_str("Characters/Annie/Skins/Skin0/Resources")]
        );
        assert_eq!(bin.dependencies, ["DATA/Characters/Annie/Skins/Skin1.bin"]);
        // The properties are kept as they are.
        assert_eq!(bin.objects[0].properties, game_skin_bin("Annie", 1).objects[0].properties);
        assert!(redirect(&game_skin_bin("Annie", 1), "Annie", 2).is_none());
    }

    #[test]
    fn reads_the_dependencies_of_a_bin_header() {
        let data = bytes(&game_skin_bin("AnnieTibbers", 1));
        assert_eq!(bin_dependencies(&data), ["DATA/Characters/AnnieTibbers/AnnieTibbers.bin"]);
        assert_eq!(bin_dependencies(&data[..20]), Vec::<String>::new());
        assert!(bin_dependencies(b"not a bin at all").is_empty());
    }

    #[test]
    fn rejects_base_skins() {
        assert!(skin_index(1000).is_err());
        assert_eq!(skin_index(950_029).unwrap(), 29);
    }

    /// A small champion archive: Annie and Tibbers each have a base skin and skin 1, and
    /// Annie's skin 2 has no Tibbers.
    fn game_wad() -> Vec<u8> {
        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (character, skins) in [("Annie", &[0, 1, 2][..]), ("AnnieTibbers", &[0, 1][..])] {
            for &skin in skins {
                let mut bin = game_skin_bin(character, skin);
                if character == "Annie" {
                    bin.dependencies.push("DATA/Characters/AnnieTibbers/AnnieTibbers.bin".into());
                }
                files.insert(format!("data/characters/{}/skins/skin{skin}.bin", character.to_lowercase()), bytes(&bin));
            }
        }
        files.insert("assets/characters/annie/texture.tex".into(), vec![0; 64]);
        let mut builder = WadBuilder::default();
        for path in files.keys() {
            builder = builder.with_chunk(WadChunkBuilder::default().with_path(path));
        }
        let by_hash: BTreeMap<WadHash, Vec<u8>> = files.iter().map(|(p, d)| (WadHash::hash_str(p), d.clone())).collect();
        let mut out = Cursor::new(Vec::new());
        builder
            .build_to_writer(&mut out, |hash, cursor| {
                cursor.write_all(&by_hash[&hash])?;
                Ok(())
            })
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn finds_the_champion_and_its_companions() {
        let mut wad = Wad::mount(Cursor::new(game_wad())).unwrap();
        assert_eq!(characters(&mut wad, "Annie"), ["Annie", "annietibbers"]);
        let bins = skin_bins(&mut wad, "Annie", 1, &[]).unwrap();
        assert_eq!(
            bins.keys().collect::<Vec<_>>(),
            ["data/characters/annie/skins/skin0.bin", "data/characters/annietibbers/skins/skin0.bin"]
        );
        // Skin 2 has no Tibbers: only Annie is redirected. Skin 3 does not exist.
        assert_eq!(skin_bins(&mut wad, "Annie", 2, &[]).unwrap().len(), 1);
        assert!(skin_bins(&mut wad, "Annie", 3, &[]).is_err());
    }

    #[test]
    fn only_the_champion_depends_on_the_gameplay_bins() {
        let mut wad = Wad::mount(Cursor::new(game_wad())).unwrap();
        let gameplay = ["gameplay.annieskin1.bin".to_owned()];
        let bins = skin_bins(&mut wad, "Annie", 1, &gameplay).unwrap();
        let dependencies = |path: &str| {
            BinFile::from_reader(&mut Cursor::new(&bins[path])).unwrap().into_prop().unwrap().dependencies
        };
        assert_eq!(
            dependencies("data/characters/annie/skins/skin0.bin"),
            ["DATA/Characters/Annie/Skins/Skin1.bin", "gameplay.annieskin1.bin"]
        );
        assert_eq!(dependencies("data/characters/annietibbers/skins/skin0.bin"), ["DATA/Characters/annietibbers/Skins/Skin1.bin"]);
    }

    #[test]
    fn finds_gameplay_bins_by_name() {
        let mut builder = WadBuilder::default();
        for path in ["gameplay.briarskin1.bin", "gameplay.briarskin10viewcontroller.bin"] {
            builder = builder.with_chunk(WadChunkBuilder::default().with_path(path));
        }
        let mut out = Cursor::new(Vec::new());
        builder.build_to_writer(&mut out, |_, cursor| Ok(cursor.write_all(b"PROP")?)).unwrap();
        let ui = Wad::mount(Cursor::new(out.into_inner())).unwrap();
        assert_eq!(gameplay_bins(&ui, "Briar", 1), ["gameplay.briarskin1.bin"]);
        assert_eq!(gameplay_bins(&ui, "Briar", 10), ["gameplay.briarskin10viewcontroller.bin"]);
        assert!(gameplay_bins(&ui, "Briar", 2).is_empty());
    }

    #[test]
    fn writes_a_fantome_that_ltk_overlay_reads() {
        let mut wad = Wad::mount(Cursor::new(game_wad())).unwrap();
        let bins = skin_bins(&mut wad, "Annie", 1, &[]).unwrap();
        let zip = fantome("Annie", "Goth Annie", &bins).unwrap();
        let mut content = ltk_overlay::FantomeContent::new(Cursor::new(zip)).unwrap();
        use ltk_overlay::ModContentProvider as _;
        assert_eq!(content.mod_project().unwrap().display_name, "Goth Annie");
        assert_eq!(content.list_layer_wads("base").unwrap(), ["annie.wad.client"]);
    }
}
