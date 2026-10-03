//! League client (LCU) access: lockfile discovery and a tiny blocking REST client.
//!
//! The client listens on a random localhost port with a self-signed certificate and
//! publishes `LeagueClient:<pid>:<port>:<password>:https` in `lockfile` while it runs.

use anyhow::{bail, Context, Result};
use base64::Engine;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub struct Lcu {
    agent: ureq::Agent,
    base: String,
    auth: String,
    install_dir: PathBuf,
}

/// League install directories, most likely first: whatever the Riot Client recorded, then the default.
pub fn install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(program_data) = std::env::var("PROGRAMDATA") {
        let path = PathBuf::from(program_data).join("Riot Games").join("RiotClientInstalls.json");
        if let Some(json) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            if let Some(map) = json.get("associated_client").and_then(Value::as_object) {
                dirs.extend(map.keys().map(PathBuf::from));
            }
        }
    }
    dirs.push(PathBuf::from(r"C:\Riot Games\League of Legends"));
    dirs
}

/// The `Game` directory of the first League install found on disk (the client need not be running).
pub fn game_dir() -> Result<PathBuf> {
    install_dirs()
        .into_iter()
        .map(|dir| dir.join("Game"))
        .find(|game| game.join("League of Legends.exe").exists())
        .context("no League of Legends install found")
}

/// Returns `(port, password)` from a lockfile's contents.
fn parse_lockfile(text: &str) -> Result<(u16, String)> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() < 5 {
        bail!("malformed lockfile");
    }
    Ok((parts[2].parse().context("lockfile port")?, parts[3].to_owned()))
}

impl Lcu {
    /// Connects to the running client, or fails if it is not running.
    pub fn connect() -> Result<Self> {
        for dir in install_dirs() {
            let Ok(text) = std::fs::read_to_string(dir.join("lockfile")) else {
                continue;
            };
            let (port, password) = parse_lockfile(&text)?;
            let tls = native_tls::TlsConnector::builder()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true)
                .build()?;
            let agent = ureq::AgentBuilder::new()
                .tls_connector(Arc::new(tls))
                .timeout(Duration::from_secs(5))
                .build();
            let auth = format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!("riot:{password}"))
            );
            return Ok(Self {
                agent,
                base: format!("https://127.0.0.1:{port}"),
                auth,
                install_dir: dir,
            });
        }
        bail!("League client is not running (no lockfile found)")
    }

    pub fn install_dir(&self) -> &PathBuf {
        &self.install_dir
    }

    /// GET `path` as raw bytes (images), capped at 8 MiB.
    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        use std::io::Read;
        let resp = self
            .agent
            .get(&format!("{}{path}", self.base))
            .set("Authorization", &self.auth)
            .call()
            .with_context(|| format!("GET {path}"))?;
        let mut bytes = Vec::new();
        resp.into_reader().take(8 * 1024 * 1024).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// GET `path` as JSON; `Ok(None)` for a 404 (e.g. no champ select in progress).
    pub fn get(&self, path: &str) -> Result<Option<Value>> {
        let request = self.agent.get(&format!("{}{path}", self.base)).set("Authorization", &self.auth);
        match request.call() {
            Ok(resp) => Ok(Some(resp.into_json()?)),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(e).with_context(|| format!("GET {path}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_lockfile;

    #[test]
    fn parses_lockfile() {
        let (port, pw) = parse_lockfile("LeagueClient:1234:54321:s3cret:https\n").unwrap();
        assert_eq!((port, pw.as_str()), (54321, "s3cret"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_lockfile("nope").is_err());
    }
}
