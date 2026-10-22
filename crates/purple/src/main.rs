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
use purple::{cli, icon, store, update, win};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
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
        first => resident(first),
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
fn resident(arg: Option<&str>) {
    let updated = arg == Some(update::UPDATED_ARG);
    let instance = if updated {
        // The previous Purple is still exiting: wait for it to let go.
        wait_for(|| win::single_instance(win::DAEMON_MUTEX))
    } else {
        win::single_instance(win::DAEMON_MUTEX)
    };
    if arg != Some(win::BACKGROUND_ARG) {
        if updated {
            // Otherwise the old window, still closing, would be brought forward instead.
            wait_for(|| (!win::is_running(win::UI_MUTEX)).then_some(()));
        }
        // Already running or not, a launch shows the window.
        if let Err(e) = win::spawn_ui() {
            store::log(&format!("could not open the window: {e:#}"));
        }
    }
    let Some(_instance) = instance else { return };
    store::trim_log();
    update::remove_leftovers();
    if updated {
        // The previous Purple's deletion helper is still finishing: clean up after it.
        thread::spawn(|| {
            thread::sleep(Duration::from_secs(30));
            update::remove_leftovers();
        });
    }
    match run() {
        Ok(Some(exe)) => {
            if let Err(e) = win::spawn_program(&exe, update::UPDATED_ARG) {
                store::log(&format!("could not start the updated Purple: {e:#}"));
            }
        }
        Ok(None) => {}
        Err(e) => store::log(&format!("fatal: {e:#}")),
    }
}

/// Retries `attempt` for up to 30 seconds.
fn wait_for<T>(mut attempt: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(value) = attempt() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(200));
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

/// Folds `event` into the status shared with the window and wakes the tray when it changed.
fn publish(status: &Mutex<Status>, main_thread: u32, event: &Event) {
    log_event(event);
    let Ok(mut current) = status.lock() else { return };
    if current.apply(event) {
        if let Err(e) = current.write() {
            store::log(&format!("could not write the status: {e:#}"));
        }
        win::post(main_thread, win::MSG_WAKE);
    }
}

const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Looks for a newer Purple at start and then daily, and installs it when the window asks for it.
/// Once installed, `restart` holds the program to start and the tray is woken to quit.
fn updater(status: &Mutex<Status>, main_thread: u32, restart: &Mutex<Option<PathBuf>>) {
    let request = match win::NamedEvent::create(update::REQUEST_EVENT) {
        Ok(request) => request,
        Err(e) => return store::log(&format!("updates disabled: {e:#}")),
    };
    let mut latest: Option<update::Release> = None;
    let mut next_check = Instant::now();
    loop {
        if Instant::now() >= next_check {
            next_check = Instant::now() + UPDATE_CHECK_EVERY;
            match update::check() {
                Ok(Some(release)) => {
                    publish(status, main_thread, &Event::UpdateAvailable(release.version.clone()));
                    latest = Some(release);
                }
                Ok(None) => {}
                Err(e) => store::log(&format!("could not check for a Purple update: {e:#}")),
            }
        }
        if !request.wait(next_check.saturating_duration_since(Instant::now())) {
            continue;
        }
        let Some(release) = &latest else { continue };
        // Restarting would pull the patcher out from under a running game.
        if status.lock().is_ok_and(|s| s.in_game) {
            publish(status, main_thread, &Event::UpdateFailed("Purple can update once the game is over.".into()));
            continue;
        }
        publish(status, main_thread, &Event::Updating);
        match update::install(release) {
            Ok(exe) => {
                if let Ok(mut restart) = restart.lock() {
                    *restart = Some(exe);
                }
                win::post(main_thread, win::MSG_WAKE);
                return;
            }
            Err(e) => publish(status, main_thread, &Event::UpdateFailed(format!("{e:#}"))),
        }
    }
}

/// Runs the resident program until it quits. Returns the program to start next after an update.
fn run() -> anyhow::Result<Option<PathBuf>> {
    let status = Arc::new(Mutex::new(Status::default()));
    // Forget whatever a previous run left behind.
    status.lock().expect("status lock").write()?;

    let main_thread = win::current_thread_id();
    let stop = Arc::new(AtomicBool::new(false));
    let quit_after_game = Arc::new(AtomicBool::new(false));
    let restart: Arc<Mutex<Option<PathBuf>>> = Arc::default();

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
            let mut emit = |event: Event| publish(&status, main_thread, &event);
            if let Err(e) = engine::run(&stop, &mut emit) {
                emit(Event::Error(format!("{e:#}")));
            }
        })
    };

    {
        let (status, restart) = (status.clone(), restart.clone());
        thread::spawn(move || updater(&status, main_thread, &restart));
    }

    win::message_loop(|| {
        let current = status.lock().map(|s| s.clone()).unwrap_or_default();
        let text = describe(&current);
        state_item.set_text(text);
        let _ = tray.set_tooltip(Some(format!("Purple - {text}")));
        if quit_after_game.load(Ordering::Relaxed) {
            quit_item.set_text("Quit (after the game)");
            return !current.in_game;
        }
        // An installed update restarts Purple, after the game if one has started meanwhile.
        if restart.lock().is_ok_and(|r| r.is_some()) {
            return !current.in_game;
        }
        false
    });

    // Quitting from the icon also closes the window.
    win::close_window(win::WINDOW_TITLE);
    stop.store(true, Ordering::Relaxed);
    let _ = engine_thread.join();
    let _ = Status::default().write();
    let restart = restart.lock().ok().and_then(|mut r| r.take());
    if restart.is_some() {
        // self-replace deletes the old program once this process exits: the window must have let go of it.
        wait_for(|| (!win::is_running(win::UI_MUTEX)).then_some(()));
    }
    Ok(restart)
}
