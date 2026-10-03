//! Drives `ltk_patcher_host.exe` over its line protocol (commands on stdin, events on stdout).
//!
//! The host injects `ltk_patcher_dll.dll` into the game; the DLL makes the game read the
//! overlay in place of its own WAD files. See NOTES.md for the protocol.

use crate::{store, win};
use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::OnceLock;
use std::thread;

/// `CSLOL_HOOK_OPT_OUT_AH_V1`: do not abort on the anti-skinhack scan. Required for skin swaps.
const FLAG_OPT_OUT_SKINHACK_SCAN: u32 = 4;
const LOG_LEVEL_INFO: u32 = 0x10;
const LOG_LEVEL_DEBUG: u32 = 0x20;

#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    /// `injecting | injected | waiting | exited | failed`
    Status { state: String, message: String },
    Error(String),
    /// A log record from the DLL running inside the game.
    Dll { level: String, message: String },
}

pub struct Host {
    child: Child,
    stdin: ChildStdin,
    pub events: Receiver<Event>,
}

/// Where the patcher binaries live: `$PURPLE_PATCHER_DIR`, `<exe dir>/patcher`, or the repo's vendor dir.
pub fn locate() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("PURPLE_PATCHER_DIR") {
        candidates.push(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join("patcher"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/ltk-patcher"));
    for dir in candidates {
        let exe = dir.join("ltk_patcher_host.exe");
        if exe.exists() && dir.join("ltk_patcher_dll.dll").exists() {
            return Ok(exe);
        }
    }
    bail!("ltk_patcher_host.exe + ltk_patcher_dll.dll not found (set PURPLE_PATCHER_DIR)")
}

impl Host {
    pub fn spawn(exe: &Path) -> Result<Self> {
        let mut command = Command::new(exe);
        command
            .current_dir(exe.parent().context("patcher path has no parent")?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // The host is a console program: without this, Windows opens a console window for it
        // whenever Purple runs windowed (no console of its own to inherit).
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().with_context(|| format!("starting {}", exe.display()))?;
        // The host must not outlive Purple: left running it would patch a later game with a stale overlay.
        static JOB: OnceLock<Option<win::KillOnCloseJob>> = OnceLock::new();
        let job = JOB.get_or_init(|| win::KillOnCloseJob::new().map_err(|e| store::log(&format!("no job object: {e:#}"))).ok());
        if let Some(Err(e)) = job.as_ref().map(|job| job.add(&child)) {
            store::log(&format!("could not tie the patcher to Purple's lifetime: {e:#}"));
        }
        let stdin = child.stdin.take().context("host stdin")?;
        let stdout = child.stdout.take().context("host stdout")?;
        let (tx, events) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(event) = parse_event(&line) {
                    if tx.send(event).is_err() {
                        break;
                    }
                }
            }
        });
        Ok(Self { child, stdin, events })
    }

    fn send(&mut self, line: &str) -> Result<()> {
        writeln!(self.stdin, "{line}")?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Serve `overlay` and wait for the next game to start. Must be called before the game launches.
    pub fn start_scan(&mut self, overlay: &Path) -> Result<()> {
        let prefix = format!("{}\\", overlay.display().to_string().trim_end_matches(['/', '\\']));
        let level = if std::env::var_os("PURPLE_DEBUG").is_some() { LOG_LEVEL_DEBUG } else { LOG_LEVEL_INFO };
        self.send(&format!("config loglevel {level}"))?;
        self.send(&format!("config flags {FLAG_OPT_OUT_SKINHACK_SCAN}"))?;
        self.send(&format!("config prefix {prefix}"))?;
        self.send("start scan")
    }

    /// Ends the current session; the host stays alive for the next `start_scan`.
    pub fn stop_session(&mut self) -> Result<()> {
        self.send("stop")
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Parses one host stdout line: `status <ts> <state> <msg>`, `error <ts> <msg>`,
/// `dll <ts> <pid> <tid> <level> <msg>`. Anything else (host's own log lines) is ignored.
fn parse_event(line: &str) -> Option<Event> {
    let mut parts = line.trim_end().splitn(2, ' ');
    let keyword = parts.next()?;
    let rest = parts.next().unwrap_or("");
    match keyword {
        "status" => {
            let mut it = rest.splitn(3, ' ');
            let _ts = it.next()?;
            Some(Event::Status { state: it.next()?.to_owned(), message: it.next().unwrap_or("").to_owned() })
        }
        "error" => Some(Event::Error(rest.split_once(' ').map_or(rest, |(_timestamp, message)| message).to_owned())),
        "dll" => {
            let mut it = rest.splitn(5, ' ');
            let (_ts, _pid, _tid) = (it.next()?, it.next()?, it.next()?);
            Some(Event::Dll { level: it.next()?.to_owned(), message: it.next().unwrap_or("").to_owned() })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status() {
        assert_eq!(
            parse_event("status 0.0012345 injecting scanning for game"),
            Some(Event::Status { state: "injecting".into(), message: "scanning for game".into() })
        );
    }

    #[test]
    fn parses_dll_log() {
        let e = parse_event("dll 100.30 25060 24460 WARN ltk_patcher_dll::verify: AH wad scan failed").unwrap();
        assert_eq!(e, Event::Dll { level: "WARN".into(), message: "ltk_patcher_dll::verify: AH wad scan failed".into() });
    }

    #[test]
    fn ignores_host_log_lines() {
        assert_eq!(parse_event("   0.000023900s  INFO ltk_patcher_host: host starting"), None);
    }
}
