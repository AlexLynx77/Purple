//! The Purple window: pick a skin per champion. The engine itself lives in `purple.exe` (tray).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use purple::win;

fn main() -> eframe::Result {
    let Some(_instance) = win::single_instance(win::UI_MUTEX) else {
        // The window is already open: bring it forward instead of opening a second one.
        win::focus_window(win::WINDOW_TITLE);
        return Ok(());
    };
    purple::gui::run()
}
