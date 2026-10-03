//! Small Win32 helpers: single-instance guards, window focus, process launching, message loop.

use anyhow::{Context, Result};
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId, OpenMutexW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, FindWindowW, GetMessageW, PostThreadMessageW, SetForegroundWindow, ShowWindow,
    TranslateMessage, MSG, SW_RESTORE, WM_APP,
};

/// Held for as long as the process runs; its named mutex marks the program as already running.
pub struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateMutexW and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

pub const DAEMON_MUTEX: &str = "Local\\Purple.Daemon";
pub const UI_MUTEX: &str = "Local\\Purple.Window";
/// Title of the window; distinctive so focusing it never hits an unrelated window.
pub const WINDOW_TITLE: &str = "Purple Skin Changer";

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
/// Thread message: the engine's state changed, refresh the tray.
pub const MSG_WAKE: u32 = WM_APP;
/// Thread message: leave the message loop.
pub const MSG_QUIT: u32 = WM_APP + 1;

/// A job object that kills every process assigned to it when the last handle to it closes,
/// i.e. when this program exits for any reason, including being killed.
pub struct KillOnCloseJob(HANDLE);

// SAFETY: a job object handle may be used from any thread.
unsafe impl Send for KillOnCloseJob {}
unsafe impl Sync for KillOnCloseJob {}

impl KillOnCloseJob {
    pub fn new() -> Result<Self> {
        // SAFETY: plain Win32 calls; the handle is closed on every failure path and in Drop.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            anyhow::ensure!(!job.is_null(), "CreateJobObjectW failed");
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())?;
            let ok = SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const info).cast(), size);
            if ok == 0 {
                CloseHandle(job);
                anyhow::bail!("SetInformationJobObject failed");
            }
            Ok(Self(job))
        }
    }

    /// Ties `process` to this job: it dies when the job closes.
    pub fn add(&self, process: &std::process::Child) -> Result<()> {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: both handles are valid for the duration of the call.
        let ok = unsafe { AssignProcessToJobObject(self.0, process.as_raw_handle()) };
        anyhow::ensure!(ok != 0, "AssignProcessToJobObject failed");
        Ok(())
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        // SAFETY: closed exactly once; this is what kills the assigned processes.
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Claims the named mutex; `None` when another process already holds it.
pub fn single_instance(name: &str) -> Option<Instance> {
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle.is_null() {
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            return None;
        }
        Some(Instance(handle))
    }
}

/// Whether some process currently holds the named mutex.
pub fn is_running(name: &str) -> bool {
    let name = wide(name);
    // SAFETY: `name` is NUL-terminated and outlives the call.
    unsafe {
        let handle = OpenMutexW(SYNCHRONIZE, 0, name.as_ptr());
        if handle.is_null() {
            return false;
        }
        CloseHandle(handle);
        true
    }
}

/// Brings the top-level window titled `title` to the front. Returns whether one was found.
pub fn focus_window(title: &str) -> bool {
    let title = wide(title);
    // SAFETY: `title` is NUL-terminated and outlives the calls.
    unsafe {
        let window = FindWindowW(std::ptr::null(), title.as_ptr());
        if window.is_null() {
            return false;
        }
        ShowWindow(window, SW_RESTORE);
        SetForegroundWindow(window);
        true
    }
}

fn sibling_exe(name: &str) -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe().context("locating the running program")?;
    Ok(exe.with_file_name(name))
}

/// Opens the window (`purple-ui.exe`, next to this program).
pub fn spawn_ui() -> Result<()> {
    let path = sibling_exe("purple-ui.exe")?;
    Command::new(&path).spawn().with_context(|| format!("starting {}", path.display()))?;
    Ok(())
}

/// Starts the tray program (`purple.exe`, next to this program) without a console.
pub fn spawn_daemon() -> Result<()> {
    let path = sibling_exe("purple.exe")?;
    Command::new(&path)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| format!("starting {}", path.display()))?;
    Ok(())
}

pub fn current_thread_id() -> u32 {
    // SAFETY: no preconditions.
    unsafe { GetCurrentThreadId() }
}

/// Posts a thread message to `thread_id` (the thread running [`message_loop`]).
pub fn post(thread_id: u32, message: u32) {
    // SAFETY: posting a message with no payload has no preconditions.
    unsafe { PostThreadMessageW(thread_id, message, 0, 0) };
}

/// Runs the Win32 message loop that the tray icon needs. `on_wake` is called for each
/// [`MSG_WAKE`] and returns true to leave the loop; [`MSG_QUIT`] leaves it unconditionally.
pub fn message_loop(mut on_wake: impl FnMut() -> bool) {
    // SAFETY: standard message pump; `msg` is initialised by GetMessageW before use.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if msg.hwnd.is_null() {
                match msg.message {
                    MSG_WAKE if on_wake() => return,
                    MSG_WAKE => continue,
                    MSG_QUIT => return,
                    _ => {}
                }
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_process_in_the_job_dies_when_the_job_closes() {
        let mut child = Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("ping is available on Windows");
        let job = KillOnCloseJob::new().unwrap();
        job.add(&child).unwrap();
        assert!(child.try_wait().unwrap().is_none(), "still running while the job is open");
        drop(job);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        panic!("the process survived the job closing");
    }

    #[test]
    fn a_named_mutex_is_exclusive_and_visible() {
        let name = r"Local\Purple.Test.Mutex";
        assert!(!is_running(name));
        let first = single_instance(name).expect("first claim succeeds");
        assert!(is_running(name));
        assert!(single_instance(name).is_none(), "second claim fails");
        drop(first);
        assert!(!is_running(name));
    }
}
