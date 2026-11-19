// Agent Desk: a small always-available AI overlay for the desktop.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod audio;
mod backend;
mod dock;
mod files;
mod hotkey;
mod mcp;
mod platform;
mod schedule;
mod settings;
mod setup;
mod stt;
mod tasks;
mod uia;
mod update;

use backend::{AiEvent, Ask, Engine};
use dock::Rect;
use hotkey::Gesture;
use serde::Serialize;
use settings::Settings;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow,
    WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, Shortcut, ShortcutState};

/// Height of the notch in logical pixels: the part that stays put on a side
/// edge while the answer panel grows below it.
const NOTCH_HEIGHT: f64 = 58.0;
const MAX_WIDTH: f64 = 640.0;
const MAX_HEIGHT: f64 = 640.0;
/// Long edge of the screenshot sent to the model.
pub(crate) const SHOT_MAX_EDGE: u32 = 1568;
/// Holding the wake keys for less than this is a tap (open to type);
/// longer is speech.
const TAP_MS: u64 = 280;
/// Pastes the latest answer into the app in front, without touching the overlay.
const INSERT_SHORTCUT: &str = "Ctrl+Alt+Enter";

/// How a screenshot the AI saw maps back to the screen, so it can click
/// at x/y in that picture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShotGeom {
    left: i32,
    top: i32,
    /// Size of the picture the AI got.
    width: u32,
    height: u32,
    /// Screen pixels per picture pixel.
    scale: f64,
}

impl ShotGeom {
    fn new(origin: (i32, i32), w: u32, h: u32) -> Self {
        let long = w.max(h).max(1);
        let scale = if long > SHOT_MAX_EDGE {
            long as f64 / SHOT_MAX_EDGE as f64
        } else {
            1.0
        };
        Self {
            left: origin.0,
            top: origin.1,
            width: (w as f64 / scale) as u32,
            height: (h as f64 / scale) as u32,
            scale,
        }
    }
    fn to_screen(self, x: i32, y: i32) -> (i32, i32) {
        (
            self.left + (x as f64 * self.scale).round() as i32,
            self.top + (y as f64 * self.scale).round() as i32,
        )
    }
}

/// What was on screen when the assistant was called.
#[derive(Default)]
struct Snapshot {
    foreground: platform::Foreground,
    shot: Option<platform::RawImage>,
}

/// A push-to-talk recording in progress.
struct Talk {
    id: u64,
    recording: audio::Recording,
    /// Whether "Listening" has been shown for it yet.
    shown: Arc<AtomicBool>,
}

/// "Work with me" live mode while it is running.
struct LiveSession {
    stop: Arc<AtomicBool>,
    _listeners: Vec<audio::Listener>,
}

/// The screen the overlay is on: its usable area, full bounds and scale.
#[derive(Clone, Copy)]
struct Screen {
    work: Rect,
    full: Rect,
    scale: f64,
}

struct App {
    dir: PathBuf,
    /// Actions waiting for the user's OK, by action id.
    pending: Mutex<HashMap<u64, std::sync::mpsc::Sender<tasks::Decision>>>,
    action_ids: AtomicU64,
    /// Where typing and key presses go; 0 means "the window in front".
    target: Mutex<isize>,
    schedules: Mutex<Vec<schedule::Schedule>>,
    /// The last list from find_on_screen, and when it was made.
    elements: Mutex<(Vec<uia::Element>, Option<Instant>)>,
    /// Where the last screenshot the AI saw sits on screen.
    shot: Mutex<Option<ShotGeom>>,
    settings: Mutex<Settings>,
    engine: Engine,
    speech: stt::Transcriber,
    /// Only one model download at a time.
    model_download: Mutex<()>,
    /// Set while someone is waiting on the model download and wants to see
    /// its progress. The download may have been started quietly (while the
    /// user was still holding the keys), so the downloader checks this on
    /// every tick instead of deciding once at the start.
    show_download: AtomicBool,
    /// Last download percentage, so a waiter can show it straight away.
    download_pct: AtomicU8,
    /// The speech model the setup screen asked to download, while it runs.
    downloading: Mutex<Option<String>>,
    snapshot: Mutex<Snapshot>,
    talk: Mutex<Option<Talk>>,
    talk_ids: AtomicU64,
    shortcut_down: Mutex<Option<Instant>>,
    screen: Mutex<Option<Screen>>,
    /// Overlay size last requested by the interface, in logical pixels.
    size: Mutex<(f64, f64)>,
    /// Where the overlay was last put by the app itself, to tell the app's
    /// own moves from the user dragging it.
    placed: Mutex<(i32, i32)>,
    moves: AtomicU64,
    shown_at: Mutex<Instant>,
    capture_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    live: Mutex<Option<LiveSession>>,
    live_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    updates: update::Updates,
    update_item: Mutex<Option<MenuItem<tauri::Wry>>>,
}

#[derive(Serialize, Clone)]
struct WakeInfo {
    title: String,
    process: String,
    has_screen: bool,
    /// True when the overlay has keyboard focus (opened to type).
    typing: bool,
}

#[derive(Serialize, Clone)]
struct Phase {
    /// "listening" | "transcribing" | "preparing" | "cancelled" | "error"
    phase: &'static str,
    text: String,
}

#[derive(Serialize, Clone)]
struct Asked {
    turn: u64,
    text: String,
    /// A live-mode check-in: show the answer only if there is one.
    quiet: bool,
    /// A scheduled task: it starts a fresh conversation.
    scheduled: bool,
}

#[derive(Serialize, Clone)]
struct Heard {
    who: &'static str,
    text: String,
}

fn is_live(app: &AppHandle) -> bool {
    app.state::<App>().live.lock().unwrap().is_some()
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

fn phase(app: &AppHandle, phase: &'static str, text: impl Into<String>) {
    let _ = app.emit_to(
        "main",
        "phase",
        Phase {
            phase,
            text: text.into(),
        },
    );
}

/* ---------- placing the overlay ---------- */

fn screen_under_cursor(app: &AppHandle, win: &WebviewWindow) -> Option<Screen> {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten())?;
    Some(to_screen(&monitor))
}

fn to_screen(m: &tauri::Monitor) -> Screen {
    let area = m.work_area();
    Screen {
        work: Rect {
            x: area.position.x,
            y: area.position.y,
            w: area.size.width as i32,
            h: area.size.height as i32,
        },
        full: Rect {
            x: m.position().x,
            y: m.position().y,
            w: m.size().width as i32,
            h: m.size().height as i32,
        },
        scale: m.scale_factor(),
    }
}

/// Size the overlay to what the interface asked for and dock it.
fn place_overlay(app: &AppHandle) {
    let Some(win) = overlay(app) else { return };
    let state = app.state::<App>();
    let Some(screen) = *state.screen.lock().unwrap() else {
        return;
    };
    let (w, h) = *state.size.lock().unwrap();
    let (edge, along) = {
        let s = state.settings.lock().unwrap();
        (s.dock_edge, s.dock_along)
    };
    let size = (
        (w * screen.scale).round() as i32,
        (h * screen.scale).round() as i32,
    );
    let grip = (NOTCH_HEIGHT * screen.scale) as i32;
    let (x, y) = dock::place(edge, along, size, screen.work, grip);
    *state.placed.lock().unwrap() = (x, y);
    let _ = win.set_size(LogicalSize::new(w, h));
    let _ = win.set_position(PhysicalPosition::new(x, y));
}

/// The user let go of the overlay after dragging it: snap to the nearest edge.
fn finish_drag(app: &AppHandle) {
    let Some(win) = overlay(app) else { return };
    // Minimizing or hiding also moves the window (far off screen); that is
    // not the user choosing a new place for it.
    if win.is_minimized().unwrap_or(false) || !win.is_visible().unwrap_or(false) {
        return;
    }
    let state = app.state::<App>();
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return;
    };
    let screen = win.current_monitor().ok().flatten().map(|m| to_screen(&m));
    // Off every monitor: not a drop the user made.
    let Some(screen) = screen else { return };
    *state.screen.lock().unwrap() = Some(screen);
    let grip = (NOTCH_HEIGHT * screen.scale) as i32;
    let dropped = Rect {
        x: pos.x,
        y: pos.y,
        w: size.width as i32,
        h: size.height as i32,
    };
    let (edge, along) = dock::snap(dropped, screen.work, grip);
    let updated = {
        let mut s = state.settings.lock().unwrap();
        s.dock_edge = edge;
        s.dock_along = along;
        s.clone()
    };
    let _ = updated.save(&state.dir);
    place_overlay(app);
    let _ = app.emit("settings-changed", updated);
}

/* ---------- showing and hiding ---------- */

/// Ask WebView2 to give back memory while a window is hidden, and to use
/// its normal amount again when it is shown. The notch is idle most of the
/// time, so this keeps the tray app light.
fn webview_memory(_win: &WebviewWindow, _low: bool) {
    #[cfg(windows)]
    {
        let low = _low;
        let _ = _win.with_webview(move |view| unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
            };
            use windows::core::Interface;
            if let Ok(core) = view.controller().CoreWebView2() {
                if let Ok(v19) = core.cast::<ICoreWebView2_19>() {
                    let _ = v19.SetMemoryUsageTargetLevel(if low {
                        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
                    } else {
                        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
                    });
                }
            }
        });
    }
}

fn hide_overlay(app: &AppHandle) {
    live_stop(app);
    if let Some(win) = overlay(app) {
        let _ = win.hide();
        webview_memory(&win, true);
    }
    if let Ok(sc) = INSERT_SHORTCUT.parse::<Shortcut>() {
        let _ = app.global_shortcut().unregister(sc);
    }
    glow(app, false);
}

/// Remember what the user is looking at, before the overlay appears.
/// Returns None when the overlay itself is in front (nothing new to see).
fn capture_context(app: &AppHandle) -> Option<WakeInfo> {
    let win = overlay(app)?;
    let state = app.state::<App>();
    let include_screen = state.settings.lock().unwrap().include_screen;
    let ours = own_hwnd(&win);
    let foreground = platform::foreground();
    if foreground.hwnd == ours && ours != 0 {
        return None;
    }
    if let Some(screen) = screen_under_cursor(app, &win) {
        *state.screen.lock().unwrap() = Some(screen);
    }
    let shot = if include_screen {
        platform::capture_monitor(foreground.hwnd)
    } else {
        None
    };
    let info = WakeInfo {
        title: foreground.title.clone(),
        process: foreground.process.clone(),
        has_screen: shot.is_some(),
        typing: false,
    };
    *state.snapshot.lock().unwrap() = Snapshot { foreground, shot };
    Some(info)
}

/// Show the overlay. With `typing` it takes keyboard focus; without, it
/// appears without disturbing the app the user is in.
fn show_overlay(app: &AppHandle, typing: bool, info: Option<WakeInfo>) {
    let Some(win) = overlay(app) else { return };
    let state = app.state::<App>();
    if state.screen.lock().unwrap().is_none() {
        *state.screen.lock().unwrap() = screen_under_cursor(app, &win);
    }
    let _ = win.set_focusable(typing);
    // "Show desktop" (Win + D) minimizes every window, the notch included.
    // It has no taskbar button, so nothing else would ever bring it back.
    if win.is_minimized().unwrap_or(false) {
        let _ = win.unminimize();
    }
    place_overlay(app);
    *state.shown_at.lock().unwrap() = Instant::now();
    webview_memory(&win, false);
    let _ = win.show();
    if typing {
        let _ = win.set_focus();
        platform::force_foreground(own_hwnd(&win));
    }
    if let Ok(sc) = INSERT_SHORTCUT.parse::<Shortcut>() {
        let shortcuts = app.global_shortcut();
        if !shortcuts.is_registered(sc) {
            let _ = shortcuts.register(sc);
        }
    }
    match info {
        Some(mut info) => {
            info.typing = typing;
            let _ = app.emit_to("main", "wake", info);
        }
        None => {
            let _ = app.emit_to("main", "refocus", typing);
        }
    }

    // Start the AI process now, before the question is ready.
    let app2 = app.clone();
    std::thread::spawn(move || {
        let state = app2.state::<App>();
        let live = is_live(&app2);
        let s = state.settings.lock().unwrap().clone();
        state.engine.prewarm(
            &s.provider,
            &s.claude_model,
            &s.claude_path,
            live,
            s.actions_enabled,
        );
    });
}

/// A tap of the wake keys: open to type, or hide if already typing.
fn toggle_overlay(app: &AppHandle) {
    let Some(win) = overlay(app) else { return };
    let visible = win.is_visible().unwrap_or(false);
    let focused = win.is_focused().unwrap_or(false);
    if visible && focused {
        hide_overlay(app);
    } else {
        let info = capture_context(app);
        show_overlay(app, true, info);
    }
}

/// The soft light around the screen edge while listening.
fn glow(app: &AppHandle, on: bool) {
    let Some(win) = app.get_webview_window("glow") else {
        return;
    };
    let state = app.state::<App>();
    if on {
        if !state.settings.lock().unwrap().edge_glow {
            return;
        }
        if let Some(screen) = *state.screen.lock().unwrap() {
            let _ = win.set_position(PhysicalPosition::new(screen.full.x, screen.full.y));
            let _ = win.set_size(PhysicalSize::new(
                screen.full.w as u32,
                screen.full.h as u32,
            ));
        }
        let _ = win.set_ignore_cursor_events(true);
        webview_memory(&win, false);
        let _ = win.show();
        let _ = app.emit_to("glow", "glow", true);
    } else if win.is_visible().unwrap_or(false) {
        let _ = app.emit_to("glow", "glow", false);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(450));
            let _ = win.hide();
            webview_memory(&win, true);
        });
    }
}

/* ---------- asking ---------- */

fn send_question(app: &AppHandle, prompt: &str, with_screen: bool) -> Result<u64, String> {
    let state = app.state::<App>();
    let s = state.settings.lock().unwrap().clone();
    let (title, process, shot, shot_window) = {
        let mut snap = state.snapshot.lock().unwrap();
        // The screenshot is sent once; follow-up questions rely on the
        // conversation already containing it.
        (
            snap.foreground.title.clone(),
            snap.foreground.process.clone(),
            snap.shot.take(),
            snap.foreground.hwnd,
        )
    };
    let jpeg = if with_screen {
        shot.and_then(|img| {
            let geom = ShotGeom::new(platform::monitor_origin(shot_window), img.width, img.height);
            let jpeg = platform::to_jpeg(img, SHOT_MAX_EDGE)?;
            // The AI may click at x/y in this picture.
            *state.shot.lock().unwrap() = Some(geom);
            Some(jpeg)
        })
    } else {
        None
    };

    let live = is_live(app);
    let tools = s.actions_enabled && !live;
    // A new request may act on the window the user came from.
    *state.target.lock().unwrap() = 0;
    let mut text = String::new();
    if !title.is_empty() || !process.is_empty() {
        text.push_str(&format!("[Active window: {title} ({process})]\n"));
    }
    if tools {
        // Needed to schedule "at 5" or "tomorrow" correctly.
        text.push_str(&format!(
            "[Local time: {}]\n",
            schedule::format_stamp(platform::local_stamp())
        ));
    }
    if jpeg.is_some() {
        text.push_str("[A screenshot of my screen is attached.]\n");
    }
    let request = prompt.trim();
    text.push_str(if request.is_empty() {
        "Help me with what is on my screen."
    } else {
        request
    });

    state.engine.ask(Ask {
        prompt: text,
        jpeg,
        provider: s.provider,
        claude_model: s.claude_model,
        claude_path: s.claude_path,
        codex_model: s.codex_model,
        codex_path: s.codex_path,
        live,
        tools,
    })
}

/* ---------- push to talk ---------- */

/// Make sure the speech model is downloaded and in memory.
fn prepare_speech(app: &AppHandle, announce: bool) -> Result<(), String> {
    let state = app.state::<App>();
    let model = state.settings.lock().unwrap().voice_model.clone();
    if !stt::model_ready(&state.dir, &model) {
        if announce {
            state.show_download.store(true, Ordering::SeqCst);
            let pct = state.download_pct.load(Ordering::SeqCst);
            phase(app, "preparing", format!("Downloading speech model {pct}%"));
        }
        let result = (|| {
            let _one_at_a_time = state
                .model_download
                .lock()
                .map_err(|_| "download lock poisoned")?;
            let app2 = app.clone();
            stt::ensure_model(&state.dir, &model, &move |pct| {
                let state = app2.state::<App>();
                state.download_pct.store(pct, Ordering::SeqCst);
                if state.show_download.load(Ordering::SeqCst) {
                    phase(
                        &app2,
                        "preparing",
                        format!("Downloading speech model {pct}%"),
                    );
                }
            })
        })();
        if announce {
            state.show_download.store(false, Ordering::SeqCst);
        }
        result?;
    }
    state.speech.load(&state.dir, &model)
}

/// The wake keys went down: start recording straight away.
fn talk_start(app: &AppHandle) {
    let state = app.state::<App>();
    let id = {
        let mut slot = state.talk.lock().unwrap();
        if slot.is_some() {
            return;
        }
        let id = state.talk_ids.fetch_add(1, Ordering::SeqCst) + 1;
        let (level_app, last) = (app.clone(), AtomicU64::new(0));
        let started = Instant::now();
        let recording = audio::Recording::start(move |level| {
            // About 25 updates a second is plenty for the meter.
            let now = started.elapsed().as_millis() as u64;
            if now.saturating_sub(last.load(Ordering::Relaxed)) >= 40 {
                last.store(now, Ordering::Relaxed);
                let _ = level_app.emit_to("main", "level", level);
            }
        });
        let shown = Arc::new(AtomicBool::new(false));
        *slot = Some(Talk {
            id,
            recording,
            shown: shown.clone(),
        });
        (id, shown)
    };
    let (id, shown) = id;
    let info = capture_context(app);

    // Only show "Listening" once it is clearly a hold and not a tap.
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(TAP_MS - 60));
        let state = app.state::<App>();
        if state.talk.lock().unwrap().as_ref().map(|t| t.id) != Some(id) {
            return;
        }
        shown.store(true, Ordering::SeqCst);
        show_overlay(&app, false, info);
        phase(&app, "listening", "");
        glow(&app, true);
        // Load the speech model while the user is still talking.
        let _ = prepare_speech(&app, false);
    });
}

/// The wake keys were released after `held_ms`.
fn talk_end(app: &AppHandle, held_ms: u64) {
    let state = app.state::<App>();
    let Some(talk) = state.talk.lock().unwrap().take() else {
        return;
    };
    if held_ms < TAP_MS {
        talk.recording.cancel();
        toggle_overlay(app);
        return;
    }
    glow(app, false);
    phase(app, "transcribing", "");
    let result = (|| -> Result<u64, String> {
        let samples = talk.recording.finish()?;
        if !audio::has_speech(&samples) {
            return Err("I didn't hear anything. Hold the keys while you speak.".into());
        }
        prepare_speech(app, true)?;
        phase(app, "transcribing", "");
        let text = state.speech.transcribe(&samples)?;
        if text.is_empty() {
            return Err(
                "I couldn't make out any words. Try again a little closer to the microphone."
                    .into(),
            );
        }
        let turn = send_question(app, &text, true)?;
        let _ = app.emit_to(
            "main",
            "asked",
            Asked {
                turn,
                text,
                quiet: false,
                scheduled: false,
            },
        );
        Ok(turn)
    })();
    if let Err(message) = result {
        phase(app, "error", message);
    }
}

fn talk_cancel(app: &AppHandle) {
    let state = app.state::<App>();
    let talk = state.talk.lock().unwrap().take();
    if let Some(talk) = talk {
        talk.recording.cancel();
        glow(app, false);
        // Nothing to undo on screen if "Listening" never appeared.
        if talk.shown.load(Ordering::SeqCst) {
            phase(app, "cancelled", "");
        }
    }
}

/* ---------- live mode ("work with me") ---------- */

/// How often a fresh screenshot goes along with what was heard.
const LIVE_SHOT_EVERY: Duration = Duration::from_secs(15);

fn set_live_item(app: &AppHandle, on: bool) {
    if let Some(item) = app.state::<App>().live_item.lock().unwrap().as_ref() {
        let _ = item.set_checked(on);
    }
}

fn live_toggle(app: &AppHandle) {
    if is_live(app) {
        live_stop(app);
    } else {
        live_start(app);
    }
}

fn live_stop(app: &AppHandle) {
    let session = app.state::<App>().live.lock().unwrap().take();
    if let Some(session) = session {
        session.stop.store(true, Ordering::SeqCst);
        set_live_item(app, false);
        let _ = app.emit_to("main", "live", false);
    }
}

/// Start following along: listen to the microphone (and, if enabled, the
/// computer's own audio), transcribe each thing that is said, and pass it
/// to the AI together with an occasional screenshot.
fn live_start(app: &AppHandle) {
    let state = app.state::<App>();
    let system_audio = state.settings.lock().unwrap().live_system_audio;
    let info = capture_context(app);

    let (heard, hearing) = std::sync::mpsc::channel::<(&'static str, Vec<f32>)>();
    let mut listeners = Vec::new();
    let mut note = None;

    let listen = |who: &'static str, source: audio::Source, with_meter: bool| {
        let tx = heard.clone();
        let segmenter = Mutex::new(audio::Segmenter::default());
        let (level_app, started, last) = (app.clone(), Instant::now(), AtomicU64::new(0));
        audio::Listener::start(source, move |chunk| {
            if with_meter {
                let now = started.elapsed().as_millis() as u64;
                if now.saturating_sub(last.load(Ordering::Relaxed)) >= 60 {
                    last.store(now, Ordering::Relaxed);
                    let _ = level_app.emit_to("main", "level", audio::loudness(chunk));
                }
            }
            if let Ok(mut seg) = segmenter.lock() {
                for utterance in seg.push(chunk) {
                    let _ = tx.send((who, utterance));
                }
            }
        })
    };
    match listen("Me", audio::Source::Microphone, true) {
        Ok(l) => listeners.push(l),
        Err(e) => {
            show_overlay(app, false, info);
            phase(app, "error", e);
            return;
        }
    }
    if system_audio {
        match listen("Them", audio::Source::System, false) {
            Ok(l) => listeners.push(l),
            Err(e) => note = Some(format!("Only your microphone is being heard. {e}")),
        }
    }

    let stop = Arc::new(AtomicBool::new(false));
    *state.live.lock().unwrap() = Some(LiveSession {
        stop: stop.clone(),
        _listeners: listeners,
    });
    set_live_item(app, true);
    show_overlay(app, false, info);
    let _ = app.emit_to("main", "live", true);
    if let Some(note) = note {
        let _ = app.emit_to("main", "live-note", note);
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<App>();
        if let Err(e) = prepare_speech(&app, true) {
            phase(&app, "error", e);
            live_stop(&app);
            return;
        }
        let _ = app.emit_to("main", "live", true);
        let mut pending: Vec<String> = Vec::new();
        let mut last_shot: Option<Instant> = None;
        while !stop.load(Ordering::SeqCst) {
            if let Ok((who, samples)) = hearing.recv_timeout(Duration::from_millis(300)) {
                // Speech recognition is slower than speech when several
                // people talk at once; anything said meanwhile waits in the
                // channel and is handled next time round.
                if let Ok(text) = state.speech.transcribe(&samples) {
                    if text.split_whitespace().count() >= 2 {
                        let _ = app.emit_to(
                            "main",
                            "live-heard",
                            Heard {
                                who,
                                text: text.clone(),
                            },
                        );
                        pending.push(format!("{who}: {text}"));
                    }
                }
            }
            // One question at a time: what is heard during an answer goes
            // along with the next one.
            if pending.is_empty() || state.engine.is_busy() || stop.load(Ordering::SeqCst) {
                continue;
            }
            let with_shot = last_shot.is_none_or(|t| t.elapsed() >= LIVE_SHOT_EVERY);
            let shown = pending.last().cloned().unwrap_or_default();
            match send_live(&app, &pending, with_shot) {
                Ok(turn) => {
                    if with_shot {
                        last_shot = Some(Instant::now());
                    }
                    pending.clear();
                    let _ = app.emit_to(
                        "main",
                        "asked",
                        Asked {
                            turn,
                            text: shown,
                            quiet: true,
                            scheduled: false,
                        },
                    );
                }
                Err(e) => {
                    phase(&app, "error", e);
                    live_stop(&app);
                    return;
                }
            }
        }
    });
}

/// Pass what was just heard to the AI, with the screen as it is now.
fn send_live(app: &AppHandle, lines: &[String], with_shot: bool) -> Result<u64, String> {
    let state = app.state::<App>();
    let s = state.settings.lock().unwrap().clone();
    let foreground = platform::foreground();
    let ours = overlay(app).map(|w| own_hwnd(&w)).unwrap_or(0);
    let jpeg = if with_shot && s.include_screen && (foreground.hwnd != ours || ours == 0) {
        platform::capture_monitor(foreground.hwnd)
            .and_then(|img| platform::to_jpeg(img, SHOT_MAX_EDGE))
    } else {
        None
    };
    let mut text = String::new();
    if !foreground.title.is_empty() {
        text.push_str(&format!(
            "[Active window: {} ({})]\n",
            foreground.title, foreground.process
        ));
    }
    if jpeg.is_some() {
        text.push_str("[A screenshot of my screen right now is attached.]\n");
    }
    text.push_str(&lines.join("\n"));
    state.engine.ask(Ask {
        prompt: text,
        jpeg,
        provider: s.provider,
        claude_model: s.claude_model,
        claude_path: s.claude_path,
        codex_model: s.codex_model,
        codex_path: s.codex_path,
        live: true,
        tools: false,
    })
}

/* ---------- settings ---------- */

/// Apply everything in the settings that has an effect outside the UI.
/// Returns a message when something could not be applied.
fn apply_settings(app: &AppHandle, s: &Settings) -> Option<String> {
    let mut problem = None;

    for label in ["main", "glow"] {
        if let Some(win) = app.get_webview_window(label) {
            let _ = win.set_content_protected(s.hide_from_capture);
        }
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
                    problem = Some(format!(
                        "Could not register {}: {e}. Another app may be using it.",
                        s.shortcut
                    ));
                }
            }
            Err(e) => problem = Some(format!("{} is not a valid shortcut: {e}", s.shortcut)),
        }
    }
    if !s.live_shortcut.trim().is_empty() {
        match s.live_shortcut.trim().parse::<Shortcut>() {
            Ok(sc) => {
                if let Err(e) = shortcuts.register(sc) {
                    problem.get_or_insert(format!(
                        "Could not register {}: {e}. Another app may be using it.",
                        s.live_shortcut
                    ));
                }
            }
            Err(e) => {
                problem.get_or_insert(format!("{} is not a valid shortcut: {e}", s.live_shortcut));
            }
        }
    }
    hotkey::set_mode(hotkey::Mode::parse(&s.wake_mode));
    app.state::<App>()
        .engine
        .set_accounts(&s.claude_config_dir, &s.codex_home);

    let auto = app.autolaunch();
    if auto.is_enabled().unwrap_or(false) != s.autostart {
        let result = if s.autostart {
            auto.enable()
        } else {
            auto.disable()
        };
        if let Err(e) = result {
            problem.get_or_insert(format!("Could not change start-with-Windows: {e}"));
        }
    }
    place_overlay(app);
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
fn save_settings(
    app: AppHandle,
    state: State<App>,
    mut settings: Settings,
) -> Result<SaveResult, String> {
    settings.opacity = settings.opacity.clamp(0.3, 1.0);
    settings.dock_along = settings.dock_along.clamp(0.0, 1.0);
    settings.save(&state.dir)?;
    *state.settings.lock().unwrap() = settings.clone();
    let warning = apply_settings(&app, &settings);
    // Applying settings re-registers shortcuts, which drops the insert key.
    if overlay(&app).is_some_and(|w| w.is_visible().unwrap_or(false)) {
        if let Ok(sc) = INSERT_SHORTCUT.parse::<Shortcut>() {
            let _ = app.global_shortcut().register(sc);
        }
    }
    Ok(SaveResult { settings, warning })
}

#[derive(Serialize)]
struct Status {
    provider: String,
    cli_found: bool,
    detail: String,
    hook_supported: bool,
    voice_model_ready: bool,
}

/// Whether the chosen CLI is installed, for the hint in the overlay.
#[tauri::command(async)]
fn status(state: State<App>) -> Status {
    let s = state.settings.lock().unwrap().clone();
    let found = if s.provider == "codex" {
        backend::resolve_cli("codex", &s.codex_path)
    } else {
        backend::resolve_cli("claude", &s.claude_path)
    };
    Status {
        provider: s.provider,
        cli_found: found.is_ok(),
        detail: match found {
            Ok(p) => p.display().to_string(),
            Err(e) => e,
        },
        hook_supported: cfg!(windows),
        voice_model_ready: stt::model_ready(&state.dir, &s.voice_model),
    }
}

#[derive(Serialize)]
struct SetupInfo {
    provider: String,
    claude: setup::CliInfo,
    codex: setup::CliInfo,
    speech: Vec<setup::SpeechModel>,
    /// The speech model being downloaded right now, if any.
    downloading: Option<String>,
    download_pct: u8,
    hook_supported: bool,
}

/// Everything the setup screen shows. Both CLIs are checked at once.
#[tauri::command(async)]
fn setup_info(state: State<App>) -> SetupInfo {
    let s = state.settings.lock().unwrap().clone();
    let (cp, cd) = (s.claude_path.clone(), s.claude_config_dir.clone());
    let claude = std::thread::spawn(move || setup::cli_info("claude", &cp, &cd));
    let codex = setup::cli_info("codex", &s.codex_path, &s.codex_home);
    SetupInfo {
        provider: s.provider.clone(),
        claude: claude.join().unwrap_or_default(),
        codex,
        speech: setup::speech_models(&state.dir),
        downloading: state.downloading.lock().unwrap().clone(),
        download_pct: state.download_pct.load(Ordering::SeqCst),
        hook_supported: cfg!(windows),
    }
}

/// The models a provider's CLI offers, for the model list.
#[tauri::command(async)]
fn list_models(state: State<App>, provider: String) -> Vec<setup::ModelChoice> {
    let s = state.settings.lock().unwrap().clone();
    if provider == "codex" {
        setup::codex_models(&s.codex_path, &s.codex_home)
    } else {
        setup::claude_models(&s.claude_model, &state.engine.resolved_model())
    }
}

#[derive(Serialize, Clone)]
struct SpeechProgress {
    model: String,
    pct: u8,
    done: bool,
    error: String,
}

/// Download a speech model in the background, reporting progress.
#[tauri::command]
fn download_speech(app: AppHandle, model: String) -> Result<(), String> {
    if !stt::MODELS.iter().any(|m| m.0 == model) {
        return Err("Unknown speech model.".into());
    }
    let state = app.state::<App>();
    if state.downloading.lock().unwrap().is_some() {
        return Err("A speech model is already downloading.".into());
    }
    *state.downloading.lock().unwrap() = Some(model.clone());
    std::thread::spawn(move || {
        let state = app.state::<App>();
        let report = |pct: u8, done: bool, error: String| {
            let _ = app.emit_to(
                "main",
                "speech-progress",
                SpeechProgress {
                    model: model.clone(),
                    pct,
                    done,
                    error,
                },
            );
        };
        let result = (|| {
            let _one_at_a_time = state
                .model_download
                .lock()
                .map_err(|_| "download lock poisoned".to_string())?;
            let progress_app = app.clone();
            let m = model.clone();
            stt::ensure_model(&state.dir, &model, &move |pct| {
                progress_app
                    .state::<App>()
                    .download_pct
                    .store(pct, Ordering::SeqCst);
                let _ = progress_app.emit_to(
                    "main",
                    "speech-progress",
                    SpeechProgress {
                        model: m.clone(),
                        pct,
                        done: false,
                        error: String::new(),
                    },
                );
            })
            .map(|_| ())
        })();
        *state.downloading.lock().unwrap() = None;
        match result {
            Ok(()) => report(100, true, String::new()),
            Err(e) => report(0, true, e),
        }
    });
    Ok(())
}

#[tauri::command(async)]
fn ask(app: AppHandle, prompt: String, with_screen: bool) -> Result<u64, String> {
    send_question(&app, &prompt, with_screen)
}

#[tauri::command(async)]
fn stop(state: State<App>) {
    tasks::deny_all(&state);
    state.engine.stop();
}

#[tauri::command(async)]
fn new_chat(app: AppHandle, state: State<App>) {
    let live = is_live(&app);
    tasks::deny_all(&state);
    state.engine.new_chat();
    let s = state.settings.lock().unwrap().clone();
    state.engine.prewarm(
        &s.provider,
        &s.claude_model,
        &s.claude_path,
        live,
        s.actions_enabled,
    );
}

#[tauri::command]
fn hide(app: AppHandle) {
    hide_overlay(&app);
}

/// The interface asks for the window to match its content.
#[tauri::command]
fn resize(app: AppHandle, state: State<App>, width: f64, height: f64) {
    *state.size.lock().unwrap() = (
        width.clamp(120.0, MAX_WIDTH),
        height.clamp(40.0, MAX_HEIGHT),
    );
    place_overlay(&app);
}

/// The user clicked into the overlay to type.
#[tauri::command]
fn focus(app: AppHandle) {
    if let Some(win) = overlay(&app) {
        let _ = win.set_focusable(true);
        let _ = win.set_focus();
        platform::force_foreground(own_hwnd(&win));
    }
}

#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut c| c.set_text(text))
        .map_err(|e| e.to_string())
}

/// Paste the answer into the app the user came from.
#[tauri::command(async)]
fn insert_text(app: AppHandle, state: State<App>, text: String) -> Result<(), String> {
    copy_text(text)?;
    let target = state.snapshot.lock().unwrap().foreground.hwnd;
    hide_overlay(&app);
    if target == 0 {
        return Err(
            "Copied to the clipboard. Pasting into other apps works on Windows only.".into(),
        );
    }
    platform::paste_into(target);
    Ok(())
}

/* ---------- updates ---------- */

/// Open the notch and show a short message in it.
fn tell(app: &AppHandle, text: impl Into<String>) {
    let info = capture_context(app);
    show_overlay(app, true, info);
    let _ = app.emit_to("main", "notice", text.into());
}

/// Show in the tray that a downloaded update is waiting.
fn mark_update_ready(app: &AppHandle, version: &str) {
    if let Some(item) = app.state::<App>().update_item.lock().unwrap().as_ref() {
        let _ = item.set_text(format!("Restart to update to {version}"));
    }
    if let Some(tray) = app.tray_by_id("tray") {
        let _ = tray.set_tooltip(Some(format!("Agent Desk – version {version} is ready")));
    }
}

/// The tray's update item: install what is waiting, or check now.
fn update_clicked(app: &AppHandle) {
    let state = app.state::<App>();
    let channel = update::channel();
    if channel != update::Channel::Installer {
        tell(app, update::manual_hint(channel));
        return;
    }
    if update::ready_version(&state.updates).is_none() {
        tell(app, "Checking for updates…");
        match update::check(app, &state.updates) {
            Ok(None) => {
                let current = app.package_info().version.to_string();
                tell(app, format!("Agent Desk {current} is the latest version."));
                return;
            }
            Ok(Some(version)) => mark_update_ready(app, &version),
            Err(e) => {
                tell(app, e);
                return;
            }
        }
    }
    // Close the AI process cleanly; the installer closes the app and opens
    // the new version when it is done.
    state.engine.stop();
    if let Err(e) = update::install(&state.updates) {
        tell(app, e);
    }
}

/// Check for updates now and then, if the user allows it.
fn watch_for_updates(app: AppHandle) {
    if update::channel() != update::Channel::Installer {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(update::FIRST_CHECK);
        loop {
            let state = app.state::<App>();
            let allowed = state.settings.lock().unwrap().auto_update;
            if allowed {
                if let Ok(Some(version)) = update::check(&app, &state.updates) {
                    mark_update_ready(&app, &version);
                }
            }
            std::thread::sleep(update::EVERY);
        }
    });
}

fn build_tray(app: &AppHandle, hide_from_capture: bool) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Agent Desk", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let setup_item = MenuItem::with_id(app, "setup", "Setup and guide…", true, None::<&str>)?;
    let live = CheckMenuItem::with_id(
        app,
        "live",
        "Work with me (live)",
        true,
        false,
        None::<&str>,
    )?;
    let capture = CheckMenuItem::with_id(
        app,
        "capture",
        "Hide from screen sharing",
        true,
        hide_from_capture,
        None::<&str>,
    )?;
    let update_item = MenuItem::with_id(app, "update", "Check for updates…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &live,
            &settings,
            &setup_item,
            &capture,
            &PredefinedMenuItem::separator(app)?,
            &update_item,
            &quit,
        ],
    )?;
    *app.state::<App>().capture_item.lock().unwrap() = Some(capture);
    *app.state::<App>().live_item.lock().unwrap() = Some(live);
    *app.state::<App>().update_item.lock().unwrap() = Some(update_item);

    let open_typing = |app: &AppHandle| {
        let info = capture_context(app);
        show_overlay(app, true, info);
    };
    let mut tray = TrayIconBuilder::with_id("tray")
        .tooltip("Agent Desk")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => open_typing(app),
            "live" => {
                let app = app.clone();
                std::thread::spawn(move || live_toggle(&app));
            }
            "settings" => {
                open_typing(app);
                let _ = app.emit_to("main", "open-settings", ());
            }
            "setup" => {
                open_typing(app);
                let _ = app.emit_to("main", "open-setup", ());
            }
            "capture" => {
                let state = app.state::<App>();
                let updated = {
                    let mut s = state.settings.lock().unwrap();
                    s.hide_from_capture = !s.hide_from_capture;
                    s.clone()
                };
                let _ = updated.save(&state.dir);
                for label in ["main", "glow"] {
                    if let Some(win) = app.get_webview_window(label) {
                        let _ = win.set_content_protected(updated.hide_from_capture);
                    }
                }
                if let Some(item) = state.capture_item.lock().unwrap().as_ref() {
                    let _ = item.set_checked(updated.hide_from_capture);
                }
                let _ = app.emit("settings-changed", updated);
            }
            "update" => {
                let app = app.clone();
                std::thread::spawn(move || update_clicked(&app));
            }
            "quit" => {
                tasks::deny_all(&app.state::<App>());
                app.state::<App>().engine.stop();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
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
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let info = capture_context(app);
            show_overlay(app, true, info);
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    let app = app.clone();
                    if INSERT_SHORTCUT
                        .parse::<Shortcut>()
                        .is_ok_and(|s| &s == shortcut)
                    {
                        if event.state() == ShortcutState::Pressed {
                            let _ = app.emit_to("main", "insert-request", ());
                        }
                        return;
                    }
                    let live_key = app
                        .state::<App>()
                        .settings
                        .lock()
                        .unwrap()
                        .live_shortcut
                        .clone();
                    if live_key
                        .trim()
                        .parse::<Shortcut>()
                        .is_ok_and(|s| &s == shortcut)
                    {
                        if event.state() == ShortcutState::Pressed {
                            std::thread::spawn(move || live_toggle(&app));
                        }
                        return;
                    }
                    // The user's shortcut works like the wake keys: hold to
                    // talk, tap to type. The work happens off this thread.
                    let pressed = event.state() == ShortcutState::Pressed;
                    std::thread::spawn(move || {
                        let state = app.state::<App>();
                        if pressed {
                            let first = {
                                let mut down = state.shortcut_down.lock().unwrap();
                                let first = down.is_none(); // ignore key repeat
                                if first {
                                    *down = Some(Instant::now());
                                }
                                first
                            };
                            if first {
                                talk_start(&app);
                            }
                        } else {
                            let since = state.shortcut_down.lock().unwrap().take();
                            if let Some(since) = since {
                                talk_end(&app, since.elapsed().as_millis() as u64);
                            }
                        }
                    });
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
            focus,
            copy_text,
            insert_text,
            setup_info,
            list_models,
            download_speech,
            tasks::answer_action,
            tasks::list_schedules,
            tasks::save_schedule,
            tasks::delete_schedule,
            tasks::run_schedule_now
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let dir = app.path().app_config_dir()?;
            let first_run = !dir.join("settings.json").exists();
            let settings = Settings::load(&dir);

            let emitter = handle.clone();
            let emit: Arc<dyn Fn(AiEvent) + Send + Sync> = Arc::new(move |event| {
                let _ = emitter.emit_to("main", "ai", event);
            });
            app.manage(App {
                pending: Mutex::new(HashMap::new()),
                action_ids: AtomicU64::new(0),
                target: Mutex::new(0),
                schedules: Mutex::new(schedule::load(&dir)),
                elements: Mutex::new((Vec::new(), None)),
                shot: Mutex::new(None),
                engine: Engine::new(dir.clone(), emit),
                speech: stt::Transcriber::default(),
                model_download: Mutex::new(()),
                show_download: AtomicBool::new(false),
                download_pct: AtomicU8::new(0),
                downloading: Mutex::new(None),
                dir: dir.clone(),
                settings: Mutex::new(settings.clone()),
                snapshot: Mutex::new(Snapshot::default()),
                talk: Mutex::new(None),
                talk_ids: AtomicU64::new(0),
                shortcut_down: Mutex::new(None),
                screen: Mutex::new(None),
                size: Mutex::new((360.0, NOTCH_HEIGHT)),
                placed: Mutex::new((i32::MIN, i32::MIN)),
                moves: AtomicU64::new(0),
                shown_at: Mutex::new(Instant::now()),
                capture_item: Mutex::new(None),
                live: Mutex::new(None),
                live_item: Mutex::new(None),
                updates: update::Updates::default(),
                update_item: Mutex::new(None),
            });

            build_tray(&handle, settings.hide_from_capture)?;
            watch_for_updates(handle.clone());
            tasks::start_server(&handle);
            tasks::watch_schedules(handle.clone());

            // Gestures arrive on the keyboard hook thread, which must return
            // at once, so the work moves to its own thread.
            let hook_app = handle.clone();
            hotkey::init(Box::new(move |gesture| {
                let app = hook_app.clone();
                std::thread::spawn(move || match gesture {
                    Gesture::Down => talk_start(&app),
                    Gesture::Up(held) => talk_end(&app, held),
                    Gesture::Cancel => talk_cancel(&app),
                    Gesture::Tap => toggle_overlay(&app),
                    Gesture::None => {}
                });
            }));
            if let Some(warning) = apply_settings(&handle, &settings) {
                eprintln!("agent-desk: {warning}");
            }

            if let Some(win) = overlay(&handle) {
                let events_app = handle.clone();
                win.on_window_event(move |event| match event {
                    // Hide when the user clicks elsewhere (if they chose that).
                    WindowEvent::Focused(false) => {
                        let state = events_app.state::<App>();
                        let just_shown =
                            state.shown_at.lock().unwrap().elapsed() < Duration::from_millis(400);
                        // Acting moves focus to other apps; that is not the
                        // user clicking away.
                        let working = state.engine.is_busy();
                        if state.settings.lock().unwrap().hide_on_blur && !just_shown && !working {
                            hide_overlay(&events_app);
                        }
                    }
                    // A move the app did not make is the user dragging. When
                    // the window has been still for a moment, snap it.
                    WindowEvent::Moved(pos) => {
                        let state = events_app.state::<App>();
                        let (px, py) = *state.placed.lock().unwrap();
                        if (pos.x - px).abs() <= 1 && (pos.y - py).abs() <= 1 {
                            return;
                        }
                        let id = state.moves.fetch_add(1, Ordering::SeqCst) + 1;
                        let app = events_app.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(350));
                            if app.state::<App>().moves.load(Ordering::SeqCst) == id {
                                finish_drag(&app);
                            }
                        });
                    }
                    _ => {}
                });
            }

            // Free the AI process and the speech model after a while unused.
            let reaper = handle.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(30));
                let state = reaper.state::<App>();
                let minutes = state.settings.lock().unwrap().keep_warm_minutes;
                // A quiet stretch in a meeting must not end the session.
                if state.live.lock().unwrap().is_none() {
                    state.engine.reap_idle(minutes);
                }
                if state.talk.lock().unwrap().is_none() && state.live.lock().unwrap().is_none() {
                    state.speech.reap_idle(minutes);
                }
            });

            // First launch: show the overlay once so the user knows it is running.
            // Until setup has been finished once, open on the setup screen.
            if first_run {
                let _ = settings.save(&dir);
            }
            if !settings.setup_done {
                let info = capture_context(&handle);
                show_overlay(&handle, true, info);
                let _ = handle.emit_to("main", "open-setup", ());
            }
            // The glow window starts hidden: keep its memory low until used.
            if let Some(glow) = handle.get_webview_window("glow") {
                webview_memory(&glow, true);
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
