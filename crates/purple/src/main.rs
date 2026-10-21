//! `purple.exe`: the only program of Purple.
//!
//! - `purple.exe` starts the resident part (engine + notification-area icon) and opens the window.
//!   Closing the window leaves Purple running; it quits from the icon's menu.
//! - The window runs as a second process of the same program (`purple.exe --window`), so its memory
//!   (~100 MB) is released whenever it is closed while the resident part stays at ~15 MB.
//! - `purple.exe <command>` is the command line (see `purple.exe help`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use purple::engine::{self, Event};
use purple::status::Status;
use purple::{cli, icon, store, win};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some(win::WINDOW_ARG) => window(),
        Some(command) if cli::COMMANDS.contains(&command) => {
            win::attach_parent_console();
            if let Err(e) = cli::run_command(&args) {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        }
        first => resident(first != Some(win::BACKGROUND_ARG)),
    }
}

/// The window process (`purple.exe --window`), started by the resident program.
fn window() {
    let Some(_instance) = win::single_instance(win::UI_MUTEX) else {
        // The window is already open: bring it forward instead of opening a second one.
        win::focus_window(win::WINDOW_TITLE);
        return;
    };
    if let Err(e) = purple::gui::run() {
        store::log(&format!("window: {e}"));
    }
}

/// The resident program: engine + notification-area icon, plus the window unless `--background`.
fn resident(open_window: bool) {
    let instance = win::single_instance(win::DAEMON_MUTEX);
    if open_window {
        // Already running or not, a launch shows the window.
        if let Err(e) = win::spawn_ui() {
            store::log(&format!("could not open the window: {e:#}"));
        }
    }
    let Some(_instance) = instance else { return };
    store::trim_log();
    if let Err(e) = run() {
        store::log(&format!("fatal: {e:#}"));
    }
}

fn describe(status: &Status) -> &'static str {
    if !status.client_up {
        "League client not running"
    } else if status.armed.is_some() {
        "Skin ready for this game"
    } else if status.in_game {
        "In game"
    } else if status.phase == "ChampSelect" {
        "Champion select"
    } else {
        "Waiting for a game"
    }
}

fn log_event(event: &Event) {
    let verbose = std::env::var_os("PURPLE_DEBUG").is_some();
    match event {
        Event::Game { level, message } if verbose || level == "ERROR" || level == "WARN" => {
            store::log(&format!("game {level}: {message}"));
        }
        Event::Game { .. } => {}
        other => store::log(&format!("{other:?}")),
    }
}

fn run() -> anyhow::Result<()> {
    let status = Arc::new(Mutex::new(Status::default()));
    // Forget whatever a previous run left behind.
    status.lock().expect("status lock").write()?;

    let main_thread = win::current_thread_id();
    let stop = Arc::new(AtomicBool::new(false));
    let quit_after_game = Arc::new(AtomicBool::new(false));

    let state_item = MenuItem::new("League client not running", false, None);
    let open_item = MenuItem::new("Open Purple", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[&state_item, &PredefinedMenuItem::separator(), &open_item, &quit_item])?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("Purple")
        .with_icon(Icon::from_rgba(icon::rgba(32), 32, 32)?)
        .build()?;

    let (open_id, quit_id) = (open_item.id().clone(), quit_item.id().clone());
    {
        let (status, quit_after_game) = (status.clone(), quit_after_game.clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == open_id {
                if let Err(e) = win::spawn_ui() {
                    store::log(&format!("could not open the window: {e:#}"));
                }
            } else if event.id == quit_id {
                // Quitting would pull the patcher out from under a running game: wait for it to end.
                if status.lock().is_ok_and(|s| s.in_game) {
                    quit_after_game.store(true, Ordering::Relaxed);
                    win::post(main_thread, win::MSG_WAKE);
                } else {
                    win::post(main_thread, win::MSG_QUIT);
                }
            }
        }));
    }
    TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            if let Err(e) = win::spawn_ui() {
                store::log(&format!("could not open the window: {e:#}"));
            }
        }
    }));

    let engine_thread = {
        let (status, stop) = (status.clone(), stop.clone());
        thread::spawn(move || {
            let mut emit = |event: Event| {
                log_event(&event);
                let Ok(mut current) = status.lock() else { return };
                if current.apply(&event) {
                    if let Err(e) = current.write() {
                        store::log(&format!("could not write the status: {e:#}"));
                    }
                    win::post(main_thread, win::MSG_WAKE);
                }
            };
            if let Err(e) = engine::run(&stop, &mut emit) {
                emit(Event::Error(format!("{e:#}")));
            }
        })
    };

    win::message_loop(|| {
        let current = status.lock().map(|s| s.clone()).unwrap_or_default();
        let text = describe(&current);
        state_item.set_text(text);
        let _ = tray.set_tooltip(Some(format!("Purple - {text}")));
        if quit_after_game.load(Ordering::Relaxed) {
            quit_item.set_text("Quit (after the game)");
            return !current.in_game;
        }
        false
    });

    // Quitting from the icon also closes the window.
    win::close_window(win::WINDOW_TITLE);
    stop.store(true, Ordering::Relaxed);
    let _ = engine_thread.join();
    let _ = Status::default().write();
    Ok(())
}
