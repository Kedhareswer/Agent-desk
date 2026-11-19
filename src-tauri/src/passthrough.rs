//! The notch is visible but out of the way: clicks, scrolling and dragging
//! go straight through it to whatever is underneath, as if it were not
//! there. Only its controls take the mouse (buttons, the text field while
//! typing, Settings and the history while open), and holding Ctrl over the
//! answer lets the user scroll or select it.
//!
//! Windows decides per window, not per pixel, whether a window takes the
//! mouse, so a light loop watches where the cursor is (only while the notch
//! is on screen) and switches the window between click-through and normal
//! as the cursor moves over a control. The interface sends the controls'
//! rectangles whenever its layout changes (`set_hit_regions`).
//!
//! The same loop tells the interface when the cursor is over the notch,
//! because a click-through window gets no hover events of its own.

use serde::Deserialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// A rectangle in the notch window's CSS pixels: x, y, width, height.
pub type Rect = [f64; 4];

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Regions {
    /// The visible shape: hovering it holds the notch open.
    pub notch: Option<Rect>,
    /// Controls that always take the mouse.
    pub controls: Vec<Rect>,
    /// Parts that take the mouse only while Ctrl is held (the answer).
    pub with_ctrl: Vec<Rect>,
}

static REGIONS: Mutex<Option<Regions>> = Mutex::new(None);

/// The interface's latest layout.
#[tauri::command]
pub fn set_hit_regions(regions: Regions) {
    *REGIONS.lock().unwrap() = Some(regions);
}

fn inside(r: &Rect, (x, y): (f64, f64)) -> bool {
    x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3]
}

/// Should the window take the mouse with the cursor at this point
/// (window CSS pixels)? With no layout yet, it does (as before).
pub fn takes_mouse(regions: Option<&Regions>, at: (f64, f64), ctrl: bool) -> bool {
    let Some(r) = regions else { return true };
    r.controls.iter().any(|c| inside(c, at)) || (ctrl && r.with_ctrl.iter().any(|c| inside(c, at)))
}

pub fn hovering(regions: Option<&Regions>, at: (f64, f64)) -> bool {
    regions
        .and_then(|r| r.notch.as_ref())
        .is_some_and(|n| inside(n, at))
}

/// Watch the cursor while the notch is visible. Cheap: one cursor read per
/// 40 ms while shown, and the window is only touched when something
/// changes.
pub fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let mut taking: Option<bool> = None;
        let mut hovered = false;
        loop {
            let Some(win) = app.get_webview_window("main") else {
                return;
            };
            if !win.is_visible().unwrap_or(false) {
                taking = None;
                if hovered {
                    hovered = false;
                    let _ = app.emit_to("main", "hover", false);
                }
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            let (Ok(pos), Ok(scale)) = (win.outer_position(), win.scale_factor()) else {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            };
            let (cx, cy) = crate::uia::cursor_pos();
            let at = ((cx - pos.x) as f64 / scale, (cy - pos.y) as f64 / scale);
            let regions = REGIONS.lock().unwrap().clone();
            // A mouse button held down (a drag, a text selection) keeps the
            // window as it is until it is let go.
            if !crate::uia::mouse_down() {
                let take = takes_mouse(regions.as_ref(), at, crate::uia::ctrl_down());
                if taking != Some(take) {
                    let _ = win.set_ignore_cursor_events(!take);
                    taking = Some(take);
                }
            }
            let over = hovering(regions.as_ref(), at);
            if over != hovered {
                hovered = over;
                let _ = app.emit_to("main", "hover", over);
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Regions {
        Regions {
            notch: Some([20.0, 0.0, 600.0, 300.0]),
            controls: vec![[560.0, 20.0, 24.0, 24.0]],
            with_ctrl: vec![[20.0, 60.0, 600.0, 200.0]],
        }
    }

    #[test]
    fn only_controls_take_the_mouse() {
        let r = layout();
        assert!(takes_mouse(Some(&r), (570.0, 30.0), false), "a button");
        assert!(
            !takes_mouse(Some(&r), (200.0, 120.0), false),
            "answer text is click-through"
        );
        assert!(
            takes_mouse(Some(&r), (200.0, 120.0), true),
            "unless Ctrl is held"
        );
        assert!(
            !takes_mouse(Some(&r), (700.0, 30.0), true),
            "outside the notch"
        );
        assert!(
            takes_mouse(None, (1.0, 1.0), false),
            "no layout yet: as before"
        );
    }

    #[test]
    fn hovering_is_the_visible_shape() {
        let r = layout();
        assert!(hovering(Some(&r), (300.0, 100.0)));
        assert!(
            !hovering(Some(&r), (5.0, 100.0)),
            "the margin is not the notch"
        );
        assert!(!hovering(None, (300.0, 100.0)));
    }
}
