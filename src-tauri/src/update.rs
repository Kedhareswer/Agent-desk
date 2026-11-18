//! Updating the app from GitHub Releases.
//!
//! Only a copy installed with the Windows installer updates itself. Scoop,
//! winget, Chocolatey and the portable zip each have their own update
//! command, and two updaters fighting over one folder would break it, so
//! those copies just say how to update.
//!
//! An update is checked for a little after start and then every few hours.
//! When one is found it is downloaded and verified in the background, and
//! the tray offers "Restart to update". Nothing is installed without the
//! user choosing it, because installing closes the app.

use std::sync::Mutex;
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

/// First check after start, then every few hours.
pub const FIRST_CHECK: Duration = Duration::from_secs(60);
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

struct Ready {
    update: Update,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub struct Updates {
    ready: Mutex<Option<Ready>>,
    busy: Mutex<()>,
}

/// How this copy of the app gets updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Installed with our installer: it can replace itself.
    Installer,
    Scoop,
    /// The portable zip, a development build, or something else.
    Manual,
}

/// Work out how this copy was installed from where its exe lives.
pub fn channel() -> Channel {
    let Ok(exe) = std::env::current_exe() else {
        return Channel::Manual;
    };
    channel_for(&exe, cfg!(debug_assertions))
}

/// Pure part of [`channel`], so it can be tested.
pub fn channel_for(exe: &std::path::Path, debug: bool) -> Channel {
    if debug {
        return Channel::Manual;
    }
    let lower = exe.to_string_lossy().to_lowercase().replace('\\', "/");
    if lower.contains("/scoop/apps/") {
        return Channel::Scoop;
    }
    // The installer puts its uninstaller next to the app; the zip does not.
    match exe.parent() {
        Some(dir) if dir.join("uninstall.exe").exists() => Channel::Installer,
        _ => Channel::Manual,
    }
}

/// What to tell someone whose copy cannot update itself.
pub fn manual_hint(channel: Channel) -> &'static str {
    match channel {
        Channel::Scoop => "This copy was installed with Scoop. To update it, run: scoop update agent-desk",
        _ => "This copy cannot update itself. Download the latest version from github.com/Kedhareswer/Agent-desk/releases",
    }
}

/// The version waiting to be installed, if any.
pub fn ready_version(updates: &Updates) -> Option<String> {
    updates
        .ready
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| r.update.version.clone())
}

/// Look for a newer version and download it. Returns the new version when
/// one is ready to install, `None` when this is the latest.
pub fn check(app: &AppHandle, updates: &Updates) -> Result<Option<String>, String> {
    // One check at a time; a second caller waits and then sees the result.
    let _one = updates.busy.lock().unwrap();
    if let Some(v) = ready_version(updates) {
        return Ok(Some(v));
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = tauri::async_runtime::block_on(updater.check()).map_err(explain)?;
    let Some(update) = found else {
        return Ok(None);
    };
    let bytes =
        tauri::async_runtime::block_on(update.download(|_, _| {}, || {})).map_err(explain)?;
    let version = update.version.clone();
    *updates.ready.lock().unwrap() = Some(Ready { update, bytes });
    Ok(Some(version))
}

/// Run the downloaded installer. On Windows this closes the app; the
/// installer starts the new version when it is done.
pub fn install(updates: &Updates) -> Result<(), String> {
    let ready = updates.ready.lock().unwrap().take();
    let Some(ready) = ready else {
        return Err("No update has been downloaded yet.".into());
    };
    ready.update.install(&ready.bytes).map_err(explain)
}

fn explain(e: tauri_plugin_updater::Error) -> String {
    let text = e.to_string();
    if text.to_lowercase().contains("signature") {
        "The update could not be verified, so it was not installed.".into()
    } else {
        format!("Could not check for updates. Check your internet connection. ({text})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn scoop_copies_are_left_to_scoop() {
        let exe = Path::new(r"C:\Users\me\scoop\apps\agent-desk\0.1.1\agent-desk.exe");
        assert_eq!(channel_for(exe, false), Channel::Scoop);
    }

    #[test]
    fn development_builds_never_update() {
        let exe = Path::new(r"C:\Users\me\scoop\apps\agent-desk\0.1.1\agent-desk.exe");
        assert_eq!(channel_for(exe, true), Channel::Manual);
    }

    #[test]
    fn installer_copies_have_an_uninstaller_next_to_them() {
        let dir =
            std::env::temp_dir().join(format!("agent-desk-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("agent-desk.exe");
        assert_eq!(
            channel_for(&exe, false),
            Channel::Manual,
            "a bare exe is the zip"
        );
        std::fs::write(dir.join("uninstall.exe"), b"").unwrap();
        assert_eq!(channel_for(&exe, false), Channel::Installer);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
