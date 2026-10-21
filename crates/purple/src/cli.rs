//! Command-line mode of `purple.exe`: inspect the client, choose skins, run the engine in a terminal.

use crate::catalog::Catalog;
use crate::config::Config;
use crate::engine::{self, Event};
use crate::lcu::{self, Lcu};
use crate::{overlay, patcher_install, store};
use anyhow::{bail, Context, Result};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

const USAGE: &str = "usage: purple [command]

  (no command)                open the window and keep Purple running in the notification area
  --background                start in the notification area only, without opening the window

  status                      client connection and game phase
  skins <champion>            list a champion's skins (id, name)
  set <champion> <skin>       apply <skin> (id or part of its name) whenever <champion> is picked
  unset <champion>            stop applying a skin for <champion>
  list                        show the configured skins
  run                         wait for champion select and apply the configured skin
  build <champ id> <skin id>  fetch a skin and build its overlay (no client needed; for testing)
  update-patcher              download the latest LTK Patcher release if it is newer

Set PURPLE_DEBUG=1 for the patcher's full logs.";

/// Command names handled here (anything else starts the program normally).
pub const COMMANDS: [&str; 10] =
    ["status", "skins", "set", "unset", "list", "run", "build", "update-patcher", "help", "--help"];

/// Runs one command; `args` excludes the program name.
pub fn run_command(args: &[String]) -> Result<()> {
    let arg = |i: usize| args.get(i).map(String::as_str);
    match (arg(0), arg(1), arg(2)) {
        (Some("status"), ..) => status(),
        (Some("skins"), Some(champ), _) => skins(champ),
        (Some("set"), Some(champ), Some(skin)) => set(champ, skin),
        (Some("unset"), Some(champ), _) => unset(champ),
        (Some("list"), ..) => list(),
        (Some("run"), ..) => run(),
        (Some("build"), Some(champ), Some(skin)) => build(champ.parse()?, skin.parse()?),
        (Some("update-patcher"), ..) => {
            let host = patcher_install::ensure(true)?;
            println!("patcher ready: {}", host.display());
            Ok(())
        }
        (Some("help" | "--help"), ..) => {
            println!("{USAGE}");
            Ok(())
        }
        _ => bail!("{USAGE}"),
    }
}

fn status() -> Result<()> {
    let lcu = Lcu::connect()?;
    println!("install: {}", lcu.install_dir().display());
    let phase = lcu.get("/lol-gameflow/v1/gameflow-phase")?;
    println!("phase: {}", phase.as_ref().and_then(|v| v.as_str()).unwrap_or("?"));
    Ok(())
}

fn skins(query: &str) -> Result<()> {
    let catalog = Catalog::load(&Lcu::connect()?)?;
    let champ = catalog.champion(query).with_context(|| format!("unknown champion: {query}"))?;
    println!("{} (id {})", champ.name, champ.id);
    for s in catalog.skins_of(champ.id) {
        println!("  {:>6}  {}{}", s.id, s.name, if s.is_base { "  [base]" } else { "" });
    }
    Ok(())
}

fn set(champion: &str, skin: &str) -> Result<()> {
    let catalog = Catalog::load(&Lcu::connect()?)?;
    let champ = catalog.champion(champion).with_context(|| format!("unknown champion: {champion}"))?;
    let wanted = skin.to_lowercase();
    let matches: Vec<_> = catalog
        .skins_of(champ.id)
        .filter(|s| !s.is_base && (s.id.to_string() == skin || s.name.to_lowercase().contains(&wanted)))
        .collect();
    let chosen = match matches.as_slice() {
        [one] => *one,
        [] => bail!("no skin of {} matches {skin:?} (see `purple skins {}`)", champ.name, champ.name),
        many => {
            let names: Vec<String> = many.iter().map(|s| format!("{} ({})", s.name, s.id)).collect();
            bail!("{skin:?} is ambiguous: {}", names.join(", "))
        }
    };
    let mut cfg = Config::load()?;
    cfg.skins.insert(champ.id, chosen.id);
    cfg.save()?;
    println!("{}: {} ({})", champ.name, chosen.name, chosen.id);
    Ok(())
}

fn unset(champion: &str) -> Result<()> {
    let catalog = Catalog::load(&Lcu::connect()?)?;
    let champ = catalog.champion(champion).with_context(|| format!("unknown champion: {champion}"))?;
    let mut cfg = Config::load()?;
    if cfg.skins.remove(&champ.id).is_some() {
        cfg.save()?;
        println!("{}: no skin configured any more", champ.name);
    } else {
        println!("{}: nothing was configured", champ.name);
    }
    Ok(())
}

fn list() -> Result<()> {
    let cfg = Config::load()?;
    let catalog = Lcu::connect().ok().and_then(|l| Catalog::load(&l).ok()).or_else(Catalog::load_cached);
    if cfg.skins.is_empty() {
        println!("no skin configured (see `purple set`)");
    }
    for (champ, skin) in &cfg.skins {
        let champ_name = catalog.as_ref().and_then(|c| c.champion(&champ.to_string())).map(|c| c.name.clone());
        let skin_name = catalog.as_ref().and_then(|c| c.skins.iter().find(|s| s.id == *skin)).map(|s| s.name.clone());
        println!(
            "  {} ({champ}) -> {} ({skin})",
            champ_name.unwrap_or_else(|| "?".into()),
            skin_name.unwrap_or_else(|| "?".into())
        );
    }
    Ok(())
}

fn build(champion: u32, skin: u32) -> Result<()> {
    let started = Instant::now();
    let fantome = store::ensure_fantome(champion, skin)?;
    let overlay = overlay::build(&fantome, skin, &lcu::game_dir()?)?;
    println!("overlay ready in {:.1}s: {}", started.elapsed().as_secs_f32(), overlay.display());
    Ok(())
}

fn run() -> Result<()> {
    let verbose = std::env::var_os("PURPLE_DEBUG").is_some();
    println!("waiting for the League client...");
    engine::run(&AtomicBool::new(false), &mut |event| match event {
        Event::ClientConnected => println!("connected to the client"),
        Event::ClientLost => println!("client closed"),
        Event::Phase(phase) => println!("phase: {phase}"),
        Event::Armed { champion, skin, seconds } => println!("armed skin {skin} for champion {champion} ({seconds:.1}s)"),
        Event::NoSkin { champion } => println!("no skin configured for champion {champion} (use `purple set`)"),
        Event::SessionEnded => println!("session ended"),
        Event::Patcher { state, message } => println!("patcher: {state} {message}"),
        Event::Game { level, message } => {
            if verbose || level == "ERROR" || level == "WARN" || message.contains("redirected wad") || message.contains("overlay verified") {
                println!("game: {level} {message}");
            }
        }
        Event::PatcherUpdated(version) => println!("patcher updated to {version}"),
        Event::PatcherOutdated(message) => {
            eprintln!("patcher is too old for this game build, updating it before the next game ({message})");
        }
        Event::OverlayUnreadable(message) => {
            eprintln!("the game could not read the overlay, so no skin will show: {message}");
        }
        Event::Error(message) => eprintln!("{message}"),
    })
}
