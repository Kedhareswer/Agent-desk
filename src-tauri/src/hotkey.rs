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
    /// Either Alt key. On layouts with AltGr, Right Alt sends a fake Left
    /// Ctrl first; the hook reports that fake Ctrl as `Other`, so typing
    /// "@" or "€" never wakes the assistant.
    Alt,
    Other,
}

const TAP_MAX_MS: u64 = 300;
/// A held key repeats (the last one pressed, after at most a second), so
/// when no Ctrl or Alt event at all has come for this long, the keys were
/// let go without the hook seeing it (a lock screen, an elevated window, a
/// dropped hook).
const STALE_MS: u64 = 1500;
/// Nobody talks this long in one go: a hold whose release was missed ends.
const MAX_HOLD_MS: u64 = 120_000;
const DOUBLE_TAP_GAP_MS: u64 = 400;

/// What a key event meant.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Gesture {
    None,
    /// Ctrl+Alt went down together: start listening.
    Down,
    /// They were released after this many milliseconds. A short hold is a
    /// tap (open the bar to type); a longer one was speech.
    Up(u64),
    /// Another key joined the chord, so it was an ordinary shortcut.
    Cancel,
    /// Double-tap Ctrl.
    Tap,
}

/// Pure gesture state machine, kept separate from the hook so it can be
/// tested on any platform.
pub struct Detector {
    pub mode: Mode,
    ctrl: bool,
    alt: bool,
    /// When Ctrl or Alt was last seen down (presses and repeats).
    mod_at: u64,
    /// Another key was pressed during this chord, so it is a normal shortcut.
    dirty: bool,
    armed: bool,
    press_start: u64,
    last_tap: Option<u64>,
}

impl Detector {
    pub const fn new(mode: Mode) -> Self {
        Self {
            mode,
            ctrl: false,
            alt: false,
            mod_at: 0,
            dirty: false,
            armed: false,
            press_start: 0,
            last_tap: None,
        }
    }

    /// Feed one key event.
    pub fn on_key(&mut self, key: Key, down: bool, now_ms: u64) -> Gesture {
        match self.mode {
            Mode::Off => Gesture::None,
            Mode::CtrlAlt => self.ctrl_alt(key, down, now_ms),
            Mode::DoubleCtrl => {
                if self.double_ctrl(key, down, now_ms) {
                    Gesture::Tap
                } else {
                    Gesture::None
                }
            }
        }
    }

    fn ctrl_alt(&mut self, key: Key, down: bool, now: u64) -> Gesture {
        // Forget a modifier whose release was never seen, so the gesture
        // cannot stay stuck until that key happens to be pressed again.
        if self.armed {
            if now.saturating_sub(self.press_start) > MAX_HOLD_MS {
                *self = Self::new(self.mode);
                return Gesture::Cancel;
            }
        } else if (self.ctrl || self.alt) && now.saturating_sub(self.mod_at) > STALE_MS {
            self.ctrl = false;
            self.alt = false;
            self.dirty = false;
        }
        if down {
            match key {
                Key::Ctrl => {
                    self.ctrl = true;
                    self.mod_at = now;
                }
                Key::Alt => {
                    self.alt = true;
                    self.mod_at = now;
                }
                Key::Other => {
                    let was_armed = self.armed;
                    self.dirty = self.ctrl || self.alt;
                    self.armed = false;
                    return if was_armed {
                        Gesture::Cancel
                    } else {
                        Gesture::None
                    };
                }
            }
            if self.ctrl && self.alt && !self.dirty && !self.armed {
                self.armed = true;
                self.press_start = now;
                return Gesture::Down;
            }
            return Gesture::None;
        }
        let mut gesture = Gesture::None;
        if key != Key::Other {
            if key == Key::Ctrl {
                self.ctrl = false;
            } else {
                self.alt = false;
            }
            if self.armed {
                gesture = Gesture::Up(now.saturating_sub(self.press_start));
            }
            self.armed = false;
        }
        if !self.ctrl && !self.alt {
            self.dirty = false;
        }
        gesture
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
    use super::{Detector, Gesture, Key, Mode};
    use std::sync::{
        atomic::{AtomicU32, Ordering},
        Mutex, OnceLock,
    };
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_MENU, VK_RCONTROL, VK_RMENU,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, KillTimer, PostThreadMessageW, SetTimer, SetWindowsHookExW,
        UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN, WM_TIMER,
    };

    /// AltGr's fake Left Ctrl carries this scan code.
    const ALTGR_CTRL_SCAN: u32 = 0x21D;
    /// Windows silently removes a low-level hook that is too slow to answer
    /// (easily, while speech is being transcribed on every core), and never
    /// says so. Putting the hook back now and then keeps Ctrl + Alt working.
    const REHOOK_MS: u32 = 15_000;

    static DETECTOR: Mutex<Detector> = Mutex::new(Detector::new(Mode::Off));
    static CALLBACK: OnceLock<Box<dyn Fn(Gesture) + Send + Sync>> = OnceLock::new();
    static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn on_event(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            // Skip keys typed by software (including our own paste).
            if (info.flags.0 & LLKHF_INJECTED.0) == 0 {
                let vk = info.vkCode as u16;
                let key = if vk == VK_LCONTROL.0 && info.scanCode == ALTGR_CTRL_SCAN {
                    Key::Other // AltGr, not the user's Ctrl
                } else if vk == VK_LCONTROL.0 || vk == VK_RCONTROL.0 || vk == VK_CONTROL.0 {
                    Key::Ctrl
                } else if vk == VK_LMENU.0 || vk == VK_RMENU.0 || vk == VK_MENU.0 {
                    Key::Alt
                } else {
                    Key::Other
                };
                let msg = wparam.0 as u32;
                let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
                let gesture = DETECTOR
                    .lock()
                    .map(|mut d| d.on_key(key, down, info.time as u64))
                    .unwrap_or(Gesture::None);
                if gesture != Gesture::None {
                    if let Some(cb) = CALLBACK.get() {
                        cb(gesture);
                    }
                }
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    pub fn init(callback: Box<dyn Fn(Gesture) + Send + Sync>) {
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
            // Answer key events first, even when every core is busy.
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
            let module: Option<windows::Win32::Foundation::HINSTANCE> =
                GetModuleHandleW(None).ok().map(|m| m.into());
            let install = || SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_event), module, 0).ok();
            let mut hook: Option<HHOOK> = install();
            if hook.is_some() {
                let timer = SetTimer(None, 0, REHOOK_MS, None);
                // A low-level hook is delivered through this thread's
                // message queue, so it has to keep pumping messages.
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    if msg.message == WM_TIMER {
                        // New hook first, then the old one goes: no gap.
                        if let Some(fresh) = install() {
                            if let Some(old) = hook.replace(fresh) {
                                let _ = UnhookWindowsHookEx(old);
                            }
                        }
                    }
                }
                let _ = KillTimer(None, timer);
                if let Some(h) = hook {
                    let _ = UnhookWindowsHookEx(h);
                }
            }
            HOOK_THREAD.store(0, Ordering::SeqCst);
        });
    }
}

#[cfg(not(windows))]
mod hook {
    use super::{Gesture, Mode};
    pub fn init(_callback: Box<dyn Fn(Gesture) + Send + Sync>) {}
    pub fn set_mode(_mode: Mode) {}
}

pub use hook::{init, set_mode};

#[cfg(test)]
mod tests {
    use super::*;
    use Gesture::*;

    fn run(mode: Mode, events: &[(Key, bool, u64)]) -> Vec<Gesture> {
        let mut d = Detector::new(mode);
        events
            .iter()
            .map(|(k, down, t)| d.on_key(*k, *down, *t))
            .filter(|g| *g != None)
            .collect()
    }

    #[test]
    fn holding_ctrl_alt_reports_down_then_up_with_duration() {
        let g = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0),
                (Key::Alt, true, 10),
                (Key::Alt, false, 2510),
                (Key::Ctrl, false, 2520),
            ],
        );
        assert_eq!(g, vec![Down, Up(2500)]);
    }

    #[test]
    fn key_repeat_while_holding_does_not_restart() {
        let g = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0),
                (Key::Alt, true, 10),
                (Key::Alt, true, 500),
                (Key::Ctrl, true, 530),
                (Key::Ctrl, false, 900),
                (Key::Alt, false, 910),
            ],
        );
        assert_eq!(g, vec![Down, Up(890)]);
    }

    #[test]
    fn ctrl_alt_with_another_key_cancels() {
        // Ctrl+Alt+T, Ctrl+Alt+Delete and friends are ordinary shortcuts.
        let g = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0),
                (Key::Alt, true, 5),
                (Key::Other, true, 20),
                (Key::Other, false, 40),
                (Key::Alt, false, 60),
                (Key::Ctrl, false, 70),
            ],
        );
        assert_eq!(g, vec![Down, Cancel]);
    }

    #[test]
    fn altgr_does_nothing() {
        // AltGr arrives as Left Ctrl + Right Alt; Right Alt maps to Other.
        let g = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0),
                (Key::Other, true, 0),
                (Key::Other, false, 50),
                (Key::Ctrl, false, 50),
            ],
        );
        assert!(g.is_empty());
    }

    #[test]
    fn ctrl_alone_and_typing_do_nothing() {
        assert!(run(
            Mode::CtrlAlt,
            &[(Key::Ctrl, true, 0), (Key::Ctrl, false, 50)]
        )
        .is_empty());
        assert!(run(
            Mode::CtrlAlt,
            &[(Key::Other, true, 0), (Key::Other, false, 50)]
        )
        .is_empty());
        // Ctrl+C then adding Alt later is still not a clean chord.
        let g = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0),
                (Key::Other, true, 10),
                (Key::Other, false, 20),
                (Key::Alt, true, 30),
                (Key::Alt, false, 40),
                (Key::Ctrl, false, 50),
            ],
        );
        assert!(g.is_empty());
    }

    #[test]
    fn works_again_after_a_cancelled_chord() {
        let g = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0),
                (Key::Alt, true, 5),
                (Key::Other, true, 20),
                (Key::Other, false, 30),
                (Key::Alt, false, 40),
                (Key::Ctrl, false, 50),
                (Key::Ctrl, true, 500),
                (Key::Alt, true, 510),
                (Key::Ctrl, false, 560),
                (Key::Alt, false, 570),
            ],
        );
        assert_eq!(g, vec![Down, Cancel, Down, Up(50)]);
    }

    #[test]
    fn a_missed_release_does_not_leave_it_stuck() {
        // Ctrl went down, then its release was never seen (say, the lock
        // screen took it). Long after, a fresh Ctrl+Alt still works.
        let g = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0),
                (Key::Other, true, 10),
                (Key::Other, false, 20),
                (Key::Ctrl, true, 60_000),
                (Key::Alt, true, 60_010),
                (Key::Alt, false, 60_300),
                (Key::Ctrl, false, 60_310),
            ],
        );
        assert_eq!(g, vec![Down, Up(290)]);
    }

    #[test]
    fn a_hold_whose_release_was_missed_is_cancelled() {
        let g = run(
            Mode::CtrlAlt,
            &[
                (Key::Ctrl, true, 0),
                (Key::Alt, true, 10),
                // ... no release seen for minutes ...
                (Key::Other, false, 130_000),
            ],
        );
        assert_eq!(g, vec![Down, Cancel]);
    }

    #[test]
    fn a_long_hold_with_key_repeat_is_not_stale() {
        // Only the last key pressed repeats (here none at all): a long
        // hold still ends normally.
        let events = [
            (Key::Ctrl, true, 0),
            (Key::Alt, true, 10),
            (Key::Alt, false, 30_010),
            (Key::Ctrl, false, 30_020),
        ];
        assert_eq!(run(Mode::CtrlAlt, &events), vec![Down, Up(30_000)]);
    }

    #[test]
    fn double_ctrl_fires_on_second_quick_tap() {
        let g = run(
            Mode::DoubleCtrl,
            &[
                (Key::Ctrl, true, 0),
                (Key::Ctrl, false, 60),
                (Key::Ctrl, true, 200),
                (Key::Ctrl, false, 260),
            ],
        );
        assert_eq!(g, vec![Tap]);
    }

    #[test]
    fn slow_or_interrupted_taps_do_not_fire() {
        let slow = run(
            Mode::DoubleCtrl,
            &[
                (Key::Ctrl, true, 0),
                (Key::Ctrl, false, 60),
                (Key::Ctrl, true, 900),
                (Key::Ctrl, false, 960),
            ],
        );
        assert!(slow.is_empty());
        // Ctrl+C then Ctrl+V is two Ctrl presses, but not a double tap.
        let copy_paste = run(
            Mode::DoubleCtrl,
            &[
                (Key::Ctrl, true, 0),
                (Key::Other, true, 20),
                (Key::Other, false, 40),
                (Key::Ctrl, false, 60),
                (Key::Ctrl, true, 150),
                (Key::Other, true, 170),
                (Key::Other, false, 190),
                (Key::Ctrl, false, 210),
            ],
        );
        assert!(copy_paste.is_empty());
    }

    #[test]
    fn key_repeat_while_holding_ctrl_is_not_a_tap() {
        let g = run(
            Mode::DoubleCtrl,
            &[
                (Key::Ctrl, true, 0),
                (Key::Ctrl, true, 250),
                (Key::Ctrl, true, 500),
                (Key::Ctrl, false, 520),
                (Key::Ctrl, true, 600),
                (Key::Ctrl, false, 650),
            ],
        );
        assert!(g.is_empty());
    }
}
