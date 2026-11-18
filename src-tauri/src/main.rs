// Agent Desk: a small always-available AI overlay for the desktop.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod hotkey;
mod platform;
mod settings;

use backend::{AiEvent, Ask, Engine};
use serde::Serialize;
use settings::Settings;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, State, WebviewWindow, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, Shortcut, ShortcutState};

const OVERLAY_WIDTH: f64 = 720.0;
const MAX_HEIGHT: f64 = 640.0;
/// Long edge of the screenshot sent to the model.
const SHOT_MAX_EDGE: u32 = 1568;

/// What was on screen when the overlay was opened.
#[derive(Default)]
struct Snapshot {
    foreground: platform::Foreground,
    shot: Option<platform::RawImage>,
}

struct App {
    dir: PathBuf,
    settings: Mutex<Settings>,
    engine: Engine,
    snapshot: Mutex<Snapshot>,
    shown_at: Mutex<Instant>,
    capture_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
}

#[derive(Serialize, Clone)]
struct WakeInfo {
    title: String,
    process: String,
    has_screen: bool,
}

fn overlay(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn own_hwnd(_win: &WebviewWindow) -> isize {
    #[cfg(windows)]
    {
        _win.hwnd().map(|h| h.0 as isize).unwrap_or(0)
    }
    #[cfg(not(windows))]
    {
        0
    }
}

fn hide_overlay(app: &AppHandle) {
    if let Some(win) = overlay(app) {
        let _ = win.hide();
    }
}

/// Open the overlay near the top of the screen the user is working on.
fn show_overlay(app: &AppHandle) {
    let Some(win) = overlay(app) else { return };
    let state = app.state::<App>();
    let settings = state.settings.lock().unwrap().clone();
    let ours = own_hwnd(&win);

    // Look at the screen before the overlay appears on it.
    let foreground = platform::foreground();
    let mut info = None;
    if foreground.hwnd != ours || ours == 0 {
        let shot = if settings.include_screen { platform::capture_monitor(foreground.hwnd) } else { None };
        info = Some(WakeInfo { title: foreground.title.clone(), process: foreground.process.clone(), has_screen: shot.is_some() });
        *state.snapshot.lock().unwrap() = Snapshot { foreground, shot };
    }

    let cursor = app.cursor_position().ok();
    let monitor = cursor
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        // The window's own size reads as zero before it is first shown, so
        // the width comes from the known logical size instead.
        let width = (OVERLAY_WIDTH * m.scale_factor()) as i32;
        let x = m.position().x + (m.size().width as i32 - width) / 2;
        let y = m.position().y + (m.size().height as f64 * 0.14) as i32;
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }

    *state.shown_at.lock().unwrap() = Instant::now();
    let _ = win.show();
    let _ = win.set_focus();
    platform::force_foreground(ours);
    if let Some(info) = info {
        let _ = app.emit("wake", info);
    } else {
        let _ = app.emit("refocus", ());
    }

    // Start the AI process now, while the user is still typing.
    let app2 = app.clone();
    std::thread::spawn(move || {
        let state = app2.state::<App>();
        state.engine.prewarm(&settings.provider, &settings.claude_model, &settings.claude_path);
    });
}

fn toggle_overlay(app: &AppHandle) {
    let Some(win) = overlay(app) else { return };
    let visible = win.is_visible().unwrap_or(false);
    let focused = win.is_focused().unwrap_or(false);
    if visible && focused {
        hide_overlay(app);
    } else {
        show_overlay(app);
    }
}

/// Apply everything in the settings that has an effect outside the UI.
/// Returns a message when something could not be applied.
fn apply_settings(app: &AppHandle, s: &Settings) -> Option<String> {
    let mut problem = None;

    if let Some(win) = overlay(app) {
        let _ = win.set_content_protected(s.hide_from_capture);
    }
    if let Some(item) = app.state::<App>().capture_item.lock().unwrap().as_ref() {
        let _ = item.set_checked(s.hide_from_capture);
    }

    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    if !s.shortcut.trim().is_empty() {
        match s.shortcut.trim().parse::<Shortcut>() {
            Ok(sc) => {
                if let Err(e) = shortcuts.register(sc) {
                    problem = Some(format!("Could not register {}: {e}. Another app may be using it.", s.shortcut));
                }
            }
            Err(e) => problem = Some(format!("{} is not a valid shortcut: {e}", s.shortcut)),
        }
    }
    hotkey::set_mode(hotkey::Mode::parse(&s.wake_mode));

    let auto = app.autolaunch();
    if auto.is_enabled().unwrap_or(false) != s.autostart {
        let result = if s.autostart { auto.enable() } else { auto.disable() };
        if let Err(e) = result {
            problem.get_or_insert(format!("Could not change start-with-Windows: {e}"));
        }
    }
    problem
}

#[tauri::command]
fn get_settings(state: State<App>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[derive(Serialize)]
struct SaveResult {
    settings: Settings,
    warning: Option<String>,
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<App>, settings: Settings) -> Result<SaveResult, String> {
    settings.save(&state.dir)?;
    *state.settings.lock().unwrap() = settings.clone();
    let warning = apply_settings(&app, &settings);
    Ok(SaveResult { settings, warning })
}

#[derive(Serialize)]
struct Status {
    provider: String,
    cli_found: bool,
    detail: String,
    hook_supported: bool,
}

/// Whether the chosen CLI is installed, for the hint under the prompt.
#[tauri::command(async)]
fn status(state: State<App>) -> Status {
    let s = state.settings.lock().unwrap().clone();
    let found = if s.provider == "codex" { backend::resolve_cli("codex", &s.codex_path) } else { backend::resolve_cli("claude", &s.claude_path) };
    Status {
        provider: s.provider,
        cli_found: found.is_ok(),
        detail: match found {
            Ok(p) => p.display().to_string(),
            Err(e) => e,
        },
        hook_supported: cfg!(windows),
    }
}

#[tauri::command(async)]
fn ask(state: State<App>, prompt: String, with_screen: bool) -> Result<u64, String> {
    let s = state.settings.lock().unwrap().clone();
    let (title, process, shot) = {
        let mut snap = state.snapshot.lock().unwrap();
        // The screenshot is sent once; follow-up questions rely on the
        // conversation already containing it.
        (snap.foreground.title.clone(), snap.foreground.process.clone(), snap.shot.take())
    };
    let jpeg = if with_screen { shot.and_then(|img| platform::to_jpeg(img, SHOT_MAX_EDGE)) } else { None };

    let mut text = String::new();
    if !title.is_empty() || !process.is_empty() {
        text.push_str(&format!("[Active window: {title} ({process})]\n"));
    }
    if jpeg.is_some() {
        text.push_str("[A screenshot of my screen is attached.]\n");
    }
    let request = prompt.trim();
    text.push_str(if request.is_empty() { "Help me with what is on my screen." } else { request });

    state.engine.ask(Ask {
        prompt: text,
        jpeg,
        provider: s.provider,
        claude_model: s.claude_model,
        claude_path: s.claude_path,
        codex_model: s.codex_model,
        codex_path: s.codex_path,
    })
}

#[tauri::command(async)]
fn stop(state: State<App>) {
    state.engine.stop();
}

#[tauri::command(async)]
fn new_chat(state: State<App>) {
    state.engine.new_chat();
    let s = state.settings.lock().unwrap().clone();
    state.engine.prewarm(&s.provider, &s.claude_model, &s.claude_path);
}

#[tauri::command]
fn hide(app: AppHandle) {
    hide_overlay(&app);
}

#[tauri::command]
fn resize(app: AppHandle, height: f64) {
    if let Some(win) = overlay(&app) {
        let _ = win.set_size(LogicalSize::new(OVERLAY_WIDTH, height.clamp(60.0, MAX_HEIGHT)));
    }
}

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).map_err(|e| e.to_string())
}

/// Paste the answer into the app the user came from.
#[tauri::command(async)]
fn insert_text(app: AppHandle, state: State<App>, text: String) -> Result<(), String> {
    copy_text(text)?;
    let target = state.snapshot.lock().unwrap().foreground.hwnd;
    hide_overlay(&app);
    if target == 0 {
        return Err("Copied to the clipboard. Pasting into other apps works on Windows only.".into());
    }
    platform::paste_into(target);
    Ok(())
}

fn build_tray(app: &AppHandle, hide_from_capture: bool) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Agent Desk", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let capture = CheckMenuItem::with_id(app, "capture", "Hide from screen sharing", true, hide_from_capture, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &settings, &capture, &PredefinedMenuItem::separator(app)?, &quit])?;
    *app.state::<App>().capture_item.lock().unwrap() = Some(capture);

    let mut tray = TrayIconBuilder::with_id("tray")
        .tooltip("Agent Desk")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_overlay(app),
            "settings" => {
                show_overlay(app);
                let _ = app.emit("open-settings", ());
            }
            "capture" => {
                let state = app.state::<App>();
                let updated = {
                    let mut s = state.settings.lock().unwrap();
                    s.hide_from_capture = !s.hide_from_capture;
                    s.clone()
                };
                let _ = updated.save(&state.dir);
                if let Some(win) = overlay(app) {
                    let _ = win.set_content_protected(updated.hide_from_capture);
                }
                if let Some(item) = state.capture_item.lock().unwrap().as_ref() {
                    let _ = item.set_checked(updated.hide_from_capture);
                }
                let _ = app.emit("settings-changed", updated);
            }
            "quit" => {
                app.state::<App>().engine.stop();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_overlay(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_overlay(app)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_overlay(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            status,
            ask,
            stop,
            new_chat,
            hide,
            resize,
            copy_text,
            insert_text
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let dir = app.path().app_config_dir()?;
            let settings = Settings::load(&dir);

            let emitter = handle.clone();
            let emit: Arc<dyn Fn(AiEvent) + Send + Sync> = Arc::new(move |event| {
                let _ = emitter.emit("ai", event);
            });
            app.manage(App {
                engine: Engine::new(dir.clone(), emit),
                dir,
                settings: Mutex::new(settings.clone()),
                snapshot: Mutex::new(Snapshot::default()),
                shown_at: Mutex::new(Instant::now()),
                capture_item: Mutex::new(None),
            });

            build_tray(&handle, settings.hide_from_capture)?;

            // Modifier-only wake gestures arrive from the keyboard hook
            // thread, which must return quickly, so the work moves elsewhere.
            let hook_app = handle.clone();
            hotkey::init(Box::new(move || {
                let app = hook_app.clone();
                std::thread::spawn(move || toggle_overlay(&app));
            }));
            if let Some(warning) = apply_settings(&handle, &settings) {
                eprintln!("agent-desk: {warning}");
            }

            // Hide when the user clicks elsewhere.
            if let Some(win) = overlay(&handle) {
                let blur_app = handle.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::Focused(false) = event {
                        let state = blur_app.state::<App>();
                        let just_shown = state.shown_at.lock().unwrap().elapsed() < Duration::from_millis(400);
                        if state.settings.lock().unwrap().hide_on_blur && !just_shown {
                            hide_overlay(&blur_app);
                        }
                    }
                });
            }

            // Close the AI process after it has been idle for a while.
            let reaper = handle.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                let state = reaper.state::<App>();
                let minutes = state.settings.lock().unwrap().keep_warm_minutes;
                state.engine.reap_idle(minutes);
            });

            // First launch: show the overlay once so the user knows it is running.
            if !app.path().app_config_dir()?.join("settings.json").exists() {
                let _ = settings.save(&app.path().app_config_dir()?);
                show_overlay(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Agent Desk")
        .run(|_app, event| {
            // Keep running in the tray when the overlay is hidden.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
