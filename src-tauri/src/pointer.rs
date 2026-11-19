//! Agent Desk's own pointer.
//!
//! Windows has one mouse cursor, and it belongs to the user. When Agent Desk
//! acts, a separate arrow labelled "Agent Desk" glides to the spot instead,
//! so it is always clear who is doing what. The click itself is done through
//! UI Automation where possible (no cursor involved); otherwise the real
//! cursor jumps there for a moment and straight back (see `uia::at_point`).
//!
//! The pointer is the click-through "pointer" window. Its arrow tip is the
//! window's top-left corner, placed one pixel past the target, so the target
//! itself is never covered by our own window.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::Duration;
use tauri::{AppHandle, Manager, PhysicalPosition};

static GENERATION: AtomicU64 = AtomicU64::new(0);
static LAST: Mutex<Option<(i32, i32)>> = Mutex::new(None);

/// Where the pointer last went, for actions without a target (scrolling).
pub fn last() -> Option<(i32, i32)> {
    *LAST.lock().unwrap()
}

/// Glide the pointer to a screen point (physical pixels) and wait until it
/// is there, so the user sees where the next step happens before it does.
pub fn glide(app: &AppHandle, x: i32, y: i32) {
    *LAST.lock().unwrap() = Some((x, y));
    GENERATION.fetch_add(1, Ordering::SeqCst);
    let Some(win) = app.get_webview_window("pointer") else {
        return;
    };
    let to = (x + 1, y + 1);
    let from = if win.is_visible().unwrap_or(false) {
        win.outer_position().map(|p| (p.x, p.y)).unwrap_or(to)
    } else {
        // It comes off the user's own cursor, then goes its own way.
        let (cx, cy) = crate::uia::cursor_pos();
        let _ = win.set_position(PhysicalPosition::new(cx + 1, cy + 1));
        let _ = win.set_ignore_cursor_events(true);
        let _ = win.show();
        (cx + 1, cy + 1)
    };
    let steps = path(from, to);
    for (px, py) in steps {
        let _ = win.set_position(PhysicalPosition::new(px, py));
        std::thread::sleep(Duration::from_millis(12));
    }
}

/// The pointer stays where it last worked for as long as Agent Desk is
/// working (it is hidden when the answer ends, see `crate::working`), so
/// the user can see who is busy. This only hides it if a step leaves it
/// standing much longer than that.
pub fn hide_soon(app: &AppHandle) {
    let generation = GENERATION.load(Ordering::SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        if GENERATION.load(Ordering::SeqCst) == generation {
            hide(&app);
        }
    });
}

pub fn hide(app: &AppHandle) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    *LAST.lock().unwrap() = None;
    if let Some(win) = app.get_webview_window("pointer") {
        let _ = win.hide();
    }
}

/// Positions for an eased glide: quick start, gentle stop. 18 to 40 steps
/// of 12 ms, so it takes about a fifth to half a second: slow enough to
/// follow with the eye, quick enough not to hold the task up.
pub fn path(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dy) = ((to.0 - from.0) as f64, (to.1 - from.1) as f64);
    let distance = (dx * dx + dy * dy).sqrt();
    if distance < 1.0 {
        return vec![to];
    }
    let steps = (distance / 25.0).clamp(18.0, 40.0) as i32;
    (1..=steps)
        .map(|s| {
            let t = s as f64 / steps as f64;
            let t = 1.0 - (1.0 - t).powi(3);
            (
                from.0 + (dx * t).round() as i32,
                from.1 + (dy * t).round() as i32,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::path;

    #[test]
    fn the_glide_ends_on_the_target_and_eases_out() {
        let p = path((0, 0), (800, 0));
        assert_eq!(*p.last().unwrap(), (800, 0));
        assert!(p.len() >= 18 && p.len() <= 40);
        // Bigger steps first, smaller at the end.
        assert!(p[0].0 > p[p.len() - 1].0 - p[p.len() - 2].0);
        assert!(p.windows(2).all(|w| w[1].0 >= w[0].0));
        assert_eq!(path((5, 5), (5, 5)), vec![(5, 5)]);
    }
}
