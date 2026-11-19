//! Getting back on its feet after something goes wrong.
//!
//! - Windows is asked to restart Agent Desk if it crashes or hangs.
//! - Crashes (Rust panics and native faults) are written to `crash.log` in
//!   the app's config folder, so the next one can be diagnosed. The log
//!   holds what failed and where, never what the user said.
//! - The request being worked on is kept in `pending-task.json` until its
//!   answer has finished. If the app dies first, it offers to continue the
//!   request when it starts again. The file is deleted as soon as the
//!   answer is done, stopped or dismissed.

use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

const PENDING: &str = "pending-task.json";
const LOG: &str = "crash.log";
const LOG_MAX: u64 = 256 * 1024;
/// A request older than this is not offered again: the moment has passed.
pub const OFFER_FOR_SECS: u64 = 15 * 60;
/// Command-line flag Windows passes when it restarts the app after a crash.
pub const RESTARTED_FLAG: &str = "--restarted";

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// A request that was sent and has not finished yet.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pending {
    /// Turn id of the answer; 0 until the engine has given one.
    pub turn: u64,
    pub prompt: String,
    /// Seconds since 1970 when it was asked.
    pub at: u64,
    /// The AI process already died once on this request and it was resent.
    #[serde(default)]
    pub retried: bool,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn save(dir: &Path, pending: &Pending) {
    let Ok(json) = serde_json::to_vec(pending) else {
        return;
    };
    let tmp = dir.join("pending-task.json.tmp");
    if fs::write(&tmp, json).is_ok() {
        let _ = fs::rename(&tmp, dir.join(PENDING));
    }
}

pub fn load(dir: &Path) -> Option<Pending> {
    let bytes = fs::read(dir.join(PENDING)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn clear(dir: &Path) {
    let _ = fs::remove_file(dir.join(PENDING));
}

/// Is a request left over from a crash still worth offering?
pub fn worth_offering(pending: &Pending, now: u64) -> bool {
    !pending.prompt.trim().is_empty() && now.saturating_sub(pending.at) <= OFFER_FOR_SECS
}

/// The request again, after the app itself crashed part-way.
pub fn resume_prompt(prompt: &str) -> String {
    format!(
        "[Agent Desk closed unexpectedly while doing this request and has just restarted. \
         Some of it may already be done: look at the screen first and do not repeat \
         anything that is already done.]\n{prompt}"
    )
}

/// The request again, after the AI process stopped part-way.
pub fn retry_prompt(prompt: &str) -> String {
    format!(
        "[The previous attempt at this request stopped part-way and the conversation was \
         lost. Some of it may already be done: look at the screen first and do not repeat \
         anything that is already done.]\n{prompt}"
    )
}

/// Append one line to the crash log, keeping it small.
pub fn log(line: &str) {
    let Some(dir) = LOG_DIR.get() else { return };
    let path = dir.join(LOG);
    if fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX) {
        let _ = fs::rename(&path, dir.join("crash.old.log"));
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] v{} {line}", now(), env!("CARGO_PKG_VERSION"));
    }
}

/// Set up the crash log and ask Windows to restart the app after a crash.
pub fn install(dir: &Path) {
    let _ = fs::create_dir_all(dir);
    let _ = LOG_DIR.set(dir.to_path_buf());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Location and message only: a panic message never carries what the
        // user said.
        log(&format!("panic: {info}"));
        previous(info);
    }));
    #[cfg(windows)]
    windows_side::install();
}

#[cfg(windows)]
mod windows_side {
    use windows::core::PCWSTR;
    use windows::Win32::System::Diagnostics::Debug::{
        SetUnhandledExceptionFilter, EXCEPTION_POINTERS,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Recovery::{
        RegisterApplicationRestart, RESTART_NO_PATCH, RESTART_NO_REBOOT,
    };

    pub fn install() {
        // Restart after a crash or hang (Windows does this only once the app
        // has run for at least a minute, so a crash at start-up cannot loop).
        let flag: Vec<u16> = super::RESTARTED_FLAG
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            let _ = RegisterApplicationRestart(
                PCWSTR(flag.as_ptr()),
                RESTART_NO_PATCH | RESTART_NO_REBOOT,
            );
            SetUnhandledExceptionFilter(Some(on_crash));
        }
    }

    /// A native fault (such as an illegal instruction): note what and where,
    /// then let Windows carry on with its crash handling and the restart.
    unsafe extern "system" fn on_crash(info: *const EXCEPTION_POINTERS) -> i32 {
        unsafe {
            if let Some(record) = info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) {
                let base = GetModuleHandleW(PCWSTR::null())
                    .map(|h| h.0 as usize)
                    .unwrap_or(0);
                let address = record.ExceptionAddress as usize;
                super::log(&format!(
                    "crash: code 0x{:08x} at agent-desk.exe+0x{:x}",
                    record.ExceptionCode.0 as u32,
                    address.wrapping_sub(base)
                ));
            }
        }
        0 // EXCEPTION_CONTINUE_SEARCH
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("agent-desk-recovery-{name}-{}", now()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_pending_request_survives_until_cleared() {
        let dir = temp_dir("pending");
        assert_eq!(load(&dir), None);
        let p = Pending {
            turn: 3,
            prompt: "open my GitHub".into(),
            at: 100,
            retried: false,
        };
        save(&dir, &p);
        assert_eq!(load(&dir), Some(p));
        clear(&dir);
        assert_eq!(load(&dir), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn only_recent_requests_are_offered() {
        let p = Pending {
            turn: 1,
            prompt: "check my repos".into(),
            at: 1_000,
            retried: false,
        };
        assert!(worth_offering(&p, 1_000 + 60));
        assert!(!worth_offering(&p, 1_000 + OFFER_FOR_SECS + 1));
        let empty = Pending {
            prompt: "  ".into(),
            ..p
        };
        assert!(!worth_offering(&empty, 1_000));
    }

    #[test]
    fn resent_requests_say_to_check_first() {
        for text in [resume_prompt("open GitHub"), retry_prompt("open GitHub")] {
            assert!(text.ends_with("\nopen GitHub"));
            assert!(text.contains("do not repeat"));
        }
    }
}
