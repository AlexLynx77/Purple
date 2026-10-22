//! Keeps an official copy of LTK Patcher (`ltk_patcher_host.exe` + `ltk_patcher_dll.dll`).
//!
//! Purple does not ship the patcher: its license only allows redistribution with LeagueToolkit's
//! code signature removed. Instead Purple downloads the files of the latest stable LTK Manager
//! release from GitHub, checks that both carry a valid Authenticode signature, and keeps them in
//! `<overlay base>/patcher/<release tag>/`. The game reads the DLL, so it lives next to the
//! overlays, outside the user profile.

use crate::store;
use anyhow::{bail, Context, Result};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const REPO: &str = "LeagueToolkit/ltk-manager";
pub const HOST_EXE: &str = "ltk_patcher_host.exe";
const DLL: &str = "ltk_patcher_dll.dll";
/// Records which release directory is in use.
const CURRENT_FILE: &str = "current.txt";

fn root() -> Result<PathBuf> {
    Ok(store::overlay_base_dir()?.join("patcher"))
}

pub(crate) fn agent() -> Result<ureq::Agent> {
    Ok(ureq::AgentBuilder::new()
        .tls_connector(Arc::new(native_tls::TlsConnector::new()?))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("Purple/", env!("CARGO_PKG_VERSION")))
        .build())
}

/// The installed release tag and its directory, if a complete copy is installed.
pub fn installed() -> Option<(String, PathBuf)> {
    let root = root().ok()?;
    let tag = fs::read_to_string(root.join(CURRENT_FILE)).ok()?.trim().to_owned();
    let dir = root.join(&tag);
    (dir.join(HOST_EXE).is_file() && dir.join(DLL).is_file()).then_some((tag, dir))
}

/// Tag of the latest stable LTK Manager release.
fn latest_tag() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let release: serde_json::Value = agent()?.get(&url).call().with_context(|| format!("asking GitHub for {url}"))?.into_json()?;
    let tag = release["tag_name"].as_str().context("no tag_name in the GitHub answer")?;
    // The tag becomes a directory name.
    if tag.is_empty() || !tag.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        bail!("unexpected release tag {tag:?}");
    }
    Ok(tag.to_owned())
}

/// Downloads `tag`'s patcher into `<root>/<tag>` and makes it the current one.
fn install(tag: &str) -> Result<PathBuf> {
    let root = root()?;
    let dir = root.join(tag);
    let part = root.join(format!("{tag}.part"));
    let _ = fs::remove_dir_all(&part);
    fs::create_dir_all(&part)?;
    let agent = agent()?;
    for file in [HOST_EXE, DLL] {
        let url = format!("https://github.com/{REPO}/raw/{tag}/src-tauri/resources/{file}");
        let resp = agent.get(&url).call().with_context(|| format!("downloading {url}"))?;
        let mut bytes = Vec::new();
        resp.into_reader().take(32 * 1024 * 1024).read_to_end(&mut bytes)?;
        // Refuse anything that is not a Windows executable (an HTML error page, for instance).
        if bytes.len() < 100 * 1024 || !bytes.starts_with(b"MZ") {
            bail!("{url} is not a Windows executable");
        }
        let path = part.join(file);
        fs::write(&path, &bytes)?;
        if !has_valid_signature(&path) {
            let _ = fs::remove_dir_all(&part);
            bail!("{file} from {tag} has no valid code signature: refusing to use it");
        }
    }
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("replacing {}", dir.display()))?;
    }
    fs::rename(&part, &dir)?;
    // Older releases are no longer needed (best effort: a running host keeps its files locked).
    store::prune_except(&root, tag);
    fs::write(root.join(CURRENT_FILE), tag)?;
    store::log(&format!("patcher {tag} installed"));
    Ok(dir)
}

/// Returns the patcher host to run.
///
/// `$PURPLE_PATCHER_DIR` wins when set. Otherwise the managed copy is used, downloaded first when
/// there is none. With `check_update`, GitHub is asked for a newer release; when it cannot be
/// reached, the installed copy is kept.
pub fn ensure(check_update: bool) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("PURPLE_PATCHER_DIR") {
        let host = PathBuf::from(dir).join(HOST_EXE);
        if !host.is_file() {
            bail!("PURPLE_PATCHER_DIR is set but {} does not exist", host.display());
        }
        return Ok(host);
    }
    let installed = installed();
    if check_update || installed.is_none() {
        let update = latest_tag().and_then(|tag| match &installed {
            Some((current, dir)) if *current == tag => Ok(dir.clone()),
            _ => install(&tag),
        });
        match (update, &installed) {
            (Ok(dir), _) => return Ok(dir.join(HOST_EXE)),
            (Err(e), Some((tag, _))) => store::log(&format!("keeping patcher {tag}, could not update it: {e:#}")),
            (Err(e), None) => return Err(e).context("downloading the patcher (an internet connection is needed once)"),
        }
    }
    let (_, dir) = installed.context("no patcher installed")?;
    Ok(dir.join(HOST_EXE))
}

/// Whether `path` carries a valid Authenticode signature (trusted chain, file unmodified).
fn has_valid_signature(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
        WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: every pointer handed to WinVerifyTrust points to live, initialised data for the
    // duration of both calls; the state opened by the first call is closed by the second.
    unsafe {
        let mut file = WINTRUST_FILE_INFO {
            cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: wide.as_ptr(),
            hFile: std::ptr::null_mut(),
            pgKnownSubject: std::ptr::null_mut(),
        };
        let mut data: WINTRUST_DATA = std::mem::zeroed();
        data.cbStruct = size_of::<WINTRUST_DATA>() as u32;
        data.dwUIChoice = WTD_UI_NONE;
        data.fdwRevocationChecks = WTD_REVOKE_NONE;
        data.dwUnionChoice = WTD_CHOICE_FILE;
        data.Anonymous = WINTRUST_DATA_0 { pFile: &mut file };
        data.dwStateAction = WTD_STATEACTION_VERIFY;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let result = WinVerifyTrust(std::ptr::null_mut(), &mut action, (&raw mut data).cast());
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        WinVerifyTrust(std::ptr::null_mut(), &mut action, (&raw mut data).cast());
        result == 0
    }
}

#[cfg(test)]
mod tests {
    use super::has_valid_signature;

    #[test]
    fn a_signed_windows_file_passes_and_an_unsigned_one_fails() {
        // Edge ships with Windows and carries an embedded signature.
        let edge = std::path::Path::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe");
        if edge.exists() {
            assert!(has_valid_signature(edge));
        }
        let unsigned = std::env::temp_dir().join("purple-unsigned-test.exe");
        std::fs::write(&unsigned, b"MZ not really a program").unwrap();
        assert!(!has_valid_signature(&unsigned));
        let _ = std::fs::remove_file(unsigned);
    }
}
