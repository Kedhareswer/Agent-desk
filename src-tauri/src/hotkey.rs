#![cfg_attr(not(windows), allow(dead_code))]
//! Modifier-only wake gestures ("tap Ctrl+Alt", "double-tap Ctrl").
//!
//! Windows' normal hotkey API cannot register a shortcut made only of
//! modifier keys, so these gestures are detected with a low-level keyboard
//! hook. The hook only watches for the gesture; it never blocks, changes or
//! records keystrokes. It is installed only while one of these modes is on.

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Off,
    CtrlAlt,
    DoubleCtrl,
}

impl Mode {
    pub fn parse(s: &str) -> Self {
        match s {
            "ctrl_alt" => Mode::CtrlAlt,
            "double_ctrl" => Mode::DoubleCtrl,
            _ => Mode::Off,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Ctrl,
    /// Left Alt only. Right Alt is AltGr on many layouts (it arrives as
    /// Ctrl+Alt), so it counts as `Other` to keep typing "@" or "€" from
    /// waking the assistant.
    Alt,
    Other,
}

const TAP_MAX_MS: u64 = 300;
const DOUBLE_TAP_GAP_MS: u64 = 400;

/// Pure gesture state machine, kept separate from the hook so it can be
/// tested on any platform.
pub struct Detector {
    pub mode: Mode,
    ctrl: bool,
    alt: bool,
    /// Another key was pressed during this chord, so it is a normal shortcut.
    dirty: bool,
    armed: bool,
    press_start: u64,
    last_tap: Option<u64>,
}

impl Detector {
    pub const fn new(mode: Mode) -> Self {
        Self { mode, ctrl: false, alt: false, dirty: false, armed: false, press_start: 0, last_tap: None }
    }

    /// Feed one key event. Returns true when the wake gesture completed.
    pub fn on_key(&mut self, key: Key, down: bool, now_ms: u64) -> bool {
        match self.mode {
            Mode::Off => false,
            Mode::CtrlAlt => self.ctrl_alt(key, down),
            Mode::DoubleCtrl => self.double_ctrl(key, down, now_ms),
        }
    }

    fn ctrl_alt(&mut self, key: Key, down: bool) -> bool {
        if down {
            match key {
                Key::Ctrl => self.ctrl = true,
                Key::Alt => self.alt = true,
                Key::Other => {
                    self.dirty = true;
                    self.armed = false;
                }
            }
            if self.ctrl && self.alt && !self.dirty {
                self.armed = true;
            }
            return false;
        }
        let mut fire = false;
        if key != Key::Other {
            if key == Key::Ctrl {
                self.ctrl = false;
            } else {
                self.alt = false;
            }
            fire = self.armed && !self.dirty;
            self.armed = false;
        }
        if !self.ctrl && !self.alt {
            self.dirty = false;
        }
        fire
    }

    fn double_ctrl(&mut self, key: Key, down: bool, now: u64) -> bool {
        if key != Key::Ctrl {
            if down {
                self.dirty = true;
                self.last_tap = None;
            }
            return false;
        }
        if down {
            if !self.ctrl {
                self.ctrl = true;
                self.dirty = false;
                self.press_start = now;
            }
            return false;
        }
        self.ctrl = false;
        let clean_tap = !self.dirty && now.saturating_sub(self.press_start) <= TAP_MAX_MS;
        if !clean_tap {
            self.last_tap = None;
            return false;
        }
        match self.last_tap {
            Some(prev) if now.saturating_sub(prev) <= DOUBLE_TAP_GAP_MS => {
                self.last_tap = None;
                true
            }
            _ => {
                self.last_tap = Some(now);
                false
            }
        }
    }
}

#[cfg(windows)]
mod hook {
    use super::{Detector, Key, Mode};
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Mutex, OnceLock,
    };
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_RCONTROL,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
        KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    };

    static DETECTOR: Mutex<Detector> = Mutex::new(Detector::new(Mode::Off));
    static CALLBACK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
    static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn on_event(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            // Skip keys typed by software (including our own paste).
            if (info.flags.0 & LLKHF_INJECTED.0) == 0 {
                let vk = info.vkCode as u16;
                let key = if vk == VK_LCONTROL.0 || vk == VK_RCONTROL.0 || vk == VK_CONTROL.0 {
                    Key::Ctrl
                } else if vk == VK_LMENU.0 {
                    Key::Alt
                } else {
                    Key::Other
                };
                let msg = wparam.0 as u32;
                let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let fire = DETECTOR
                    .lock()
                    .map(|mut d| d.on_key(key, down, info.time as u64))
                    .unwrap_or(false);
                if fire {
                    if let Some(cb) = CALLBACK.get() {
                        cb();
                    }
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    pub fn init(callback: Box<dyn Fn() + Send + Sync>) {
        let _ = CALLBACK.set(callback);
    }

    pub fn set_mode(mode: Mode) {
        if let Ok(mut d) = DETECTOR.lock() {
            *d = Detector::new(mode);
        }
        let running = HOOK_THREAD.load(Ordering::SeqCst);
        if mode == Mode::Off {
            if running != 0 {
                unsafe {
                    let _ = PostThreadMessageW(running, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
            return;
        }
        if running != 0 {
            return;
        }
        std::thread::spawn(|| unsafe {
            HOOK_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
            let module = GetModuleHandleW(None).ok().map(|m| m.into());
            if let Ok(hook) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_event), module, 0) {
                // A low-level hook is delivered through this thread's
                // message queue, so it has to keep pumping messages.
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
                let _ = UnhookWindowsHookEx(hook);
            }
            HOOK_THREAD.store(0, Ordering::SeqCst);
        });
    }
}

#[cfg(not(windows))]
mod hook {
    use super::Mode;
    pub fn init(_callback: Box<dyn Fn() + Send + Sync>) {}
    pub fn set_mode(_mode: Mode) {}
}

pub use hook::{init, set_mode};

#[cfg(test)]
mod tests {
    use super::*;

    fn run(mode: Mode, events: &[(Key, bool, u64)]) -> usize {
        let mut d = Detector::new(mode);
        events.iter().filter(|(k, down, t)| d.on_key(*k, *down, *t)).count()
    }

    #[test]
    fn ctrl_alt_tap_fires_once() {
        let n = run(Mode::CtrlAlt, &[(Key::Ctrl, true, 0), (Key::Alt, true, 10), (Key::Alt, false, 80), (Key::Ctrl, false, 90)]);
        assert_eq!(n, 1);
    }

    #[test]
    fn ctrl_alt_with_another_key_is_a_normal_shortcut() {
        // Ctrl+Alt+T, Ctrl+Alt+Delete and friends must not wake the app.
        let n = run(
            Mode::CtrlAlt,
            &[(Key::Ctrl, true, 0), (Key::Alt, true, 5), (Key::Other, true, 20), (Key::Other, false, 40), (Key::Alt, false, 60), (Key::Ctrl, false, 70)],
        );
        assert_eq!(n, 0);
    }

    #[test]
    fn altgr_does_not_fire() {
        // AltGr arrives as Left Ctrl + Right Alt; Right Alt maps to Other.
        let n = run(Mode::CtrlAlt, &[(Key::Ctrl, true, 0), (Key::Other, true, 0), (Key::Other, false, 50), (Key::Ctrl, false, 50)]);
        assert_eq!(n, 0);
    }

    #[test]
    fn ctrl_alone_does_not_fire() {
        assert_eq!(run(Mode::CtrlAlt, &[(Key::Ctrl, true, 0), (Key::Ctrl, false, 50)]), 0);
    }

    #[test]
    fn ctrl_alt_works_again_after_a_dirty_chord() {
        let n = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0), (Key::Alt, true, 5), (Key::Other, true, 20), (Key::Other, false, 30),
                (Key::Alt, false, 40), (Key::Ctrl, false, 50),
                (Key::Ctrl, true, 500), (Key::Alt, true, 510), (Key::Ctrl, false, 560), (Key::Alt, false, 570),
            ],
        );
        assert_eq!(n, 1);
    }

    #[test]
    fn double_ctrl_fires_on_second_quick_tap() {
        let n = run(Mode::DoubleCtrl, &[(Key::Ctrl, true, 0), (Key::Ctrl, false, 60), (Key::Ctrl, true, 200), (Key::Ctrl, false, 260)]);
        assert_eq!(n, 1);
    }

    #[test]
    fn slow_or_interrupted_taps_do_not_fire() {
        let slow = run(Mode::DoubleCtrl, &[(Key::Ctrl, true, 0), (Key::Ctrl, false, 60), (Key::Ctrl, true, 900), (Key::Ctrl, false, 960)]);
        assert_eq!(slow, 0);
        // Ctrl+C then Ctrl+V is two Ctrl presses, but not a double tap.
        let copy_paste = run(
            Mode::DoubleCtrl,
            &[(Key::Ctrl, true, 0), (Key::Other, true, 20), (Key::Other, false, 40), (Key::Ctrl, false, 60),
              (Key::Ctrl, true, 150), (Key::Other, true, 170), (Key::Other, false, 190), (Key::Ctrl, false, 210)],
        );
        assert_eq!(copy_paste, 0);
    }

    #[test]
    fn key_repeat_while_holding_ctrl_is_not_a_tap() {
        let n = run(Mode::DoubleCtrl, &[(Key::Ctrl, true, 0), (Key::Ctrl, true, 250), (Key::Ctrl, true, 500), (Key::Ctrl, false, 520), (Key::Ctrl, true, 600), (Key::Ctrl, false, 650)]);
        assert_eq!(n, 0);
    }
}
