//! The background loop: follows the League client and keeps the patcher armed with the chosen skin.
//!
//! It reports what happens through [`Event`]s so the command line and the window can both show it.

use crate::config::Config;
use crate::lcu::Lcu;
use crate::{catalog, overlay, patcher, store};
use anyhow::Result;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum Event {
    ClientConnected,
    ClientLost,
    /// The client's gameflow phase (`Lobby`, `ChampSelect`, `InProgress`, ...).
    Phase(String),
    Armed { champion: u32, skin: u32, seconds: f32 },
    NoSkin { champion: u32 },
    SessionEnded,
    Patcher { state: String, message: String },
    /// A log record from the patcher DLL running inside the game; callers decide what to show.
    Game { level: String, message: String },
    /// The patcher binaries refuse this game build and must be updated.
    PatcherOutdated(String),
    /// The game could not read the overlay, so no skin will show.
    OverlayUnreadable(String),
    Error(String),
}

/// The champion the local player has hovered or locked in, or 0 when none.
fn local_champion(lcu: &Lcu) -> Result<u32> {
    let Some(session) = lcu.get("/lol-champ-select/v1/session")? else {
        return Ok(0);
    };
    let cell = session["localPlayerCellId"].as_i64();
    let champion = session["myTeam"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["cellId"].as_i64() == cell)
        .and_then(|p| p["championId"].as_u64())
        .unwrap_or(0);
    Ok(u32::try_from(champion).unwrap_or(0))
}

/// Builds the overlay for `skin` and points the patcher at it. Returns the time it took.
fn arm(lcu: &Lcu, champion: u32, skin: u32, host: &mut Option<patcher::Host>, host_exe: &Path) -> Result<f32> {
    let started = Instant::now();
    let game_dir = lcu.install_dir().join("Game");
    let alias = catalog::champion_alias(champion, Some(lcu));
    let (fantome, source) = store::ensure_fantome(&game_dir, alias.as_deref(), champion, skin)?;
    store::log(&format!("skin {skin}: {source:?} file {}", fantome.display()));
    let overlay = overlay::build(&fantome, skin, &game_dir)?;
    if !host.as_mut().is_some_and(patcher::Host::is_alive) {
        *host = Some(patcher::Host::spawn(host_exe)?);
    }
    let host = host.as_mut().expect("host was just spawned");
    host.stop_session()?;
    host.start_scan(&overlay)?;
    Ok(started.elapsed().as_secs_f32())
}

/// Forwards what the patcher reports and keeps `in_game` true while it is attached to a game.
fn forward_host_events(host: &mut Option<patcher::Host>, in_game: &mut bool, emit: &mut dyn FnMut(Event)) {
    let Some(host) = host else { return };
    while let Ok(event) = host.events.try_recv() {
        match event {
            patcher::Event::Status { state, message } => {
                match state.as_str() {
                    "injected" | "waiting" => *in_game = true,
                    "exited" | "failed" => *in_game = false,
                    _ => {}
                }
                emit(Event::Patcher { state, message });
            }
            patcher::Event::Error(message) => emit(Event::Error(format!("patcher: {message}"))),
            patcher::Event::Dll { level, message } => {
                if message.contains("end of life") {
                    emit(Event::PatcherOutdated(message.clone()));
                } else if message.contains("overlay verified 0 wad") {
                    emit(Event::OverlayUnreadable(message.clone()));
                }
                emit(Event::Game { level, message });
            }
        }
    }
}

/// Runs until `stop` is set. Waits for the client, then follows champion select and the game.
pub fn run(stop: &AtomicBool, emit: &mut dyn FnMut(Event)) -> Result<()> {
    let host_exe = patcher::locate()?;
    let mut host: Option<patcher::Host> = None;
    let mut armed = false;
    let mut in_game = false;
    let mut last_phase = String::new();

    while !stop.load(Ordering::Relaxed) {
        let Ok(lcu) = Lcu::connect() else {
            thread::sleep(Duration::from_secs(2));
            continue;
        };
        emit(Event::ClientConnected);
        // (champion, skin wanted for it): re-arm whenever either changes during champion select.
        let mut last_key: Option<(u32, Option<u32>)> = None;

        while !stop.load(Ordering::Relaxed) {
            let phase = match lcu.get("/lol-gameflow/v1/gameflow-phase") {
                Ok(Some(v)) => v.as_str().unwrap_or("").to_owned(),
                // Client closed: leave a running session alone, the game may still be playing.
                _ => {
                    emit(Event::ClientLost);
                    break;
                }
            };
            if phase != last_phase {
                emit(Event::Phase(phase.clone()));
                last_phase.clone_from(&phase);
            }
            match phase.as_str() {
                "ChampSelect" => match local_champion(&lcu) {
                    Ok(champion) if champion != 0 => {
                        let wanted = Config::load().ok().and_then(|c| c.skins.get(&champion).copied());
                        if last_key != Some((champion, wanted)) {
                            last_key = Some((champion, wanted));
                            match wanted {
                                Some(skin) => match arm(&lcu, champion, skin, &mut host, &host_exe) {
                                    Ok(seconds) => {
                                        armed = true;
                                        emit(Event::Armed { champion, skin, seconds });
                                    }
                                    Err(e) => emit(Event::Error(format!("could not arm champion {champion}: {e:#}"))),
                                },
                                None => {
                                    if std::mem::take(&mut armed) {
                                        if let Some(h) = host.as_mut() {
                                            let _ = h.stop_session();
                                        }
                                    }
                                    emit(Event::NoSkin { champion });
                                }
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => emit(Event::Error(format!("champion select: {e:#}"))),
                },
                "GameStart" | "InProgress" | "Reconnect" => {}
                _ => {
                    last_key = None;
                    // Never tear the patcher down while it is attached to a running game.
                    if !in_game && std::mem::take(&mut armed) {
                        if let Some(h) = host.as_mut() {
                            let _ = h.stop_session();
                        }
                        emit(Event::SessionEnded);
                    }
                }
            }
            forward_host_events(&mut host, &mut in_game, emit);
            thread::sleep(Duration::from_secs(1));
        }
    }
    Ok(())
}
