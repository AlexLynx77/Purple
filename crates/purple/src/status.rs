//! What the engine is doing, shared with the window through `<data>/status.json`.
//!
//! The tray program owns the engine and writes this file whenever something changes; the window
//! only reads it. That keeps the always-on process small and lets the window come and go.

use crate::engine::Event;
use crate::store;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Armed {
    pub champion: u32,
    pub skin: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub client_up: bool,
    /// The client's gameflow phase (`ChampSelect`, `InProgress`, ...); empty when unknown.
    pub phase: String,
    /// True while the patcher is attached to a running game.
    pub in_game: bool,
    pub armed: Option<Armed>,
    /// Champion picked without any skin chosen for it.
    pub no_skin_for: Option<u32>,
    pub patcher_outdated: bool,
    pub overlay_unreadable: bool,
    pub error: Option<String>,
}

impl Status {
    /// Folds an engine event in; returns whether anything changed.
    pub fn apply(&mut self, event: &Event) -> bool {
        let before = self.clone();
        match event {
            Event::ClientConnected => self.client_up = true,
            Event::ClientLost => {
                self.client_up = false;
                self.phase.clear();
            }
            Event::Phase(phase) => self.phase.clone_from(phase),
            Event::Armed { champion, skin, .. } => {
                self.armed = Some(Armed { champion: *champion, skin: *skin });
                self.no_skin_for = None;
                self.error = None;
                self.overlay_unreadable = false;
            }
            Event::NoSkin { champion } => {
                self.armed = None;
                self.no_skin_for = Some(*champion);
            }
            Event::SessionEnded => {
                self.armed = None;
                self.no_skin_for = None;
                self.overlay_unreadable = false;
            }
            Event::Patcher { state, .. } => match state.as_str() {
                "injected" | "waiting" => self.in_game = true,
                "exited" | "failed" => self.in_game = false,
                _ => {}
            },
            Event::PatcherOutdated(_) => self.patcher_outdated = true,
            Event::OverlayUnreadable(_) => self.overlay_unreadable = true,
            Event::Error(message) => self.error = Some(message.clone()),
            Event::Game { .. } => {}
        }
        *self != before
    }

    fn path() -> Result<PathBuf> {
        Ok(store::data_dir()?.join("status.json"))
    }

    pub fn write(&self) -> Result<()> {
        let path = Self::path()?;
        std::fs::create_dir_all(path.parent().context("status path has no parent")?)?;
        // Write then rename so a reader never sees half a file.
        let part = path.with_extension("json.part");
        std::fs::write(&part, serde_json::to_string(self)?)?;
        std::fs::rename(part, path)?;
        Ok(())
    }

    pub fn read() -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(Self::path().ok()?).ok()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_a_game_from_champion_select_to_the_end() {
        let mut s = Status::default();
        assert!(s.apply(&Event::ClientConnected));
        assert!(s.apply(&Event::Armed { champion: 1, skin: 1001, seconds: 0.5 }));
        assert_eq!(s.armed, Some(Armed { champion: 1, skin: 1001 }));
        assert!(s.apply(&Event::Patcher { state: "injected".into(), message: String::new() }));
        assert!(s.in_game);
        assert!(s.apply(&Event::Patcher { state: "exited".into(), message: String::new() }));
        assert!(!s.in_game);
        assert!(s.apply(&Event::SessionEnded));
        assert_eq!(s.armed, None);
    }

    #[test]
    fn reports_no_change_for_repeated_or_irrelevant_events() {
        let mut s = Status::default();
        assert!(s.apply(&Event::Phase("Lobby".into())));
        assert!(!s.apply(&Event::Phase("Lobby".into())));
        assert!(!s.apply(&Event::Game { level: "INFO".into(), message: "x".into() }));
    }

    #[test]
    fn losing_the_client_clears_the_phase_but_not_a_running_game() {
        let mut s = Status::default();
        s.apply(&Event::ClientConnected);
        s.apply(&Event::Phase("InProgress".into()));
        s.apply(&Event::Patcher { state: "waiting".into(), message: String::new() });
        s.apply(&Event::ClientLost);
        assert!(!s.client_up && s.phase.is_empty());
        assert!(s.in_game, "the game keeps running when the client closes");
    }

    #[test]
    fn survives_a_json_round_trip() {
        let s = Status { client_up: true, armed: Some(Armed { champion: 266, skin: 266033 }), ..Status::default() };
        let back: Status = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
