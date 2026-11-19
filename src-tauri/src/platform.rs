//! Everything that talks to the operating system directly: which window is
//! in front, what the screen looks like, pasting into another app, and the
//! desktop actions (opening things, switching windows, typing, keys).
//! Windows has the real implementation; other systems get harmless stubs so
//! the app still builds and the UI can be developed anywhere.

/// A screenshot as raw RGBA pixels.
pub struct RawImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What the user was looking at when they woke the assistant.
#[derive(Default)]
pub struct Foreground {
    /// Window handle, kept so the answer can be pasted back into it.
    pub hwnd: isize,
    pub title: String,
    pub process: String,
}

#[cfg(windows)]
mod imp {
    use super::{Foreground, RawImage};
    use std::{ffi::c_void, mem::size_of, thread::sleep, time::Duration};
    use windows::core::PWSTR;
    use windows::core::{BOOL, HSTRING, PCWSTR};
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, GetMonitorInfoW, MonitorFromWindow, ReleaseDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, MONITORINFO,
        MONITOR_DEFAULTTOPRIMARY, SRCCOPY,
    };
    use windows::Win32::System::Com::{
        CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
    };
    use windows::Win32::System::SystemInformation::GetLocalTime;
    use windows::Win32::System::Threading::{
        AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL,
        VK_LWIN, VK_MENU, VK_RETURN, VK_RWIN, VK_SHIFT, VK_TAB, VK_V,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongW,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        SetForegroundWindow, ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_RESTORE, SW_SHOWNORMAL,
        WS_EX_TOOLWINDOW,
    };

    fn hwnd(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    pub fn foreground() -> Foreground {
        unsafe {
            let h = GetForegroundWindow();
            if h.0.is_null() {
                return Foreground::default();
            }
            Foreground {
                hwnd: h.0 as isize,
                title: window_title(h),
                process: process_name(h),
            }
        }
    }

    fn window_title(h: HWND) -> String {
        let mut buf = [0u16; 512];
        let n = unsafe { GetWindowTextW(h, &mut buf) }.max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }

    /// The program behind a window, e.g. "chrome.exe".
    fn process_name(h: HWND) -> String {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            let mut process = String::new();
            if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut path = [0u16; 1024];
                let mut len = path.len() as u32;
                if QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_FORMAT(0),
                    PWSTR(path.as_mut_ptr()),
                    &mut len,
                )
                .is_ok()
                {
                    let full = String::from_utf16_lossy(&path[..len as usize]);
                    process = full.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                }
                let _ = CloseHandle(handle);
            }
            process
        }
    }

    /// Top-left corner of the monitor that holds the given window.
    pub fn monitor_origin(window: isize) -> (i32, i32) {
        unsafe {
            let monitor = MonitorFromWindow(hwnd(window), MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                (info.rcMonitor.left, info.rcMonitor.top)
            } else {
                (0, 0)
            }
        }
    }

    /// Screenshot of the monitor that holds the given window.
    pub fn capture_monitor(window: isize) -> Option<RawImage> {
        unsafe {
            let monitor = MonitorFromWindow(hwnd(window), MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return None;
            }
            let r = info.rcMonitor;
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            if w <= 0 || h <= 0 {
                return None;
            }

            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bitmap.into());
            let copied = BitBlt(
                mem,
                0,
                0,
                w,
                h,
                Some(screen),
                r.left,
                r.top,
                SRCCOPY | CAPTUREBLT,
            )
            .is_ok();

            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // top-down rows
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
            let rows = if copied {
                GetDIBits(
                    mem,
                    bitmap,
                    0,
                    h as u32,
                    Some(pixels.as_mut_ptr() as *mut c_void),
                    &mut bmi,
                    DIB_RGB_COLORS,
                )
            } else {
                0
            };

            SelectObject(mem, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);

            if rows == 0 {
                return None;
            }
            // GDI hands back BGRA with an unused alpha byte.
            for px in pixels.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            Some(RawImage {
                width: w as u32,
                height: h as u32,
                rgba: pixels,
            })
        }
    }

    /// Bring a window to the front even when Windows would normally only
    /// flash its taskbar button.
    pub fn force_foreground(window: isize) {
        unsafe {
            let target = hwnd(window);
            if !IsWindow(Some(target)).as_bool() {
                return;
            }
            let front_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let this_thread = GetCurrentThreadId();
            let attached = front_thread != 0
                && front_thread != this_thread
                && AttachThreadInput(this_thread, front_thread, true).as_bool();
            let _ = BringWindowToTop(target);
            let _ = SetForegroundWindow(target);
            if attached {
                let _ = AttachThreadInput(this_thread, front_thread, false);
            }
        }
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    ..Default::default()
                },
            },
        }
    }

    /// Year, month, day, hour, minute on the wall clock.
    pub fn local_now() -> (i64, u32, u32, u32, u32) {
        let t = unsafe { GetLocalTime() };
        (
            i64::from(t.wYear),
            u32::from(t.wMonth),
            u32::from(t.wDay),
            u32::from(t.wHour),
            u32::from(t.wMinute),
        )
    }

    /// Open a file, link, shortcut or program the way Explorer would.
    /// `params` are passed to a program as its command line.
    pub fn shell_open(target: &str, params: Option<&str>) -> Result<(), String> {
        unsafe {
            // Shell extensions behind ShellExecute expect COM on this thread.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            let file = HSTRING::from(target);
            let args = params.map(HSTRING::from);
            let result = ShellExecuteW(
                None,
                &HSTRING::from("open"),
                &file,
                args.as_ref()
                    .map(|a| PCWSTR(a.as_ptr()))
                    .unwrap_or(PCWSTR::null()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
            // Values above 32 mean success.
            if result.0 as isize > 32 {
                Ok(())
            } else {
                Err(match result.0 as isize {
                    2 | 3 => "it isn't installed or could not be found".to_string(),
                    5 => "Windows refused access".to_string(),
                    code => format!("Windows error {code}"),
                })
            }
        }
    }

    /// Top-level windows a person would switch to, front to back.
    pub fn list_windows() -> Vec<(isize, String, String)> {
        unsafe extern "system" fn each(h: HWND, out: LPARAM) -> BOOL {
            let list = unsafe { &mut *(out.0 as *mut Vec<HWND>) };
            list.push(h);
            BOOL(1)
        }
        let mut handles: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut handles as *mut _ as isize));
        }
        handles
            .into_iter()
            .filter(|&h| unsafe {
                IsWindowVisible(h).as_bool()
                    && GetWindow(h, GW_OWNER).is_err()
                    && (GetWindowLongW(h, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0) == 0
            })
            .map(|h| (h.0 as isize, window_title(h), process_name(h)))
            .filter(|(_, title, _)| !title.trim().is_empty())
            .collect()
    }

    /// Bring a window to the front, restoring it if minimised.
    pub fn focus_window(window: isize) {
        unsafe {
            if IsIconic(hwnd(window)).as_bool() {
                let _ = ShowWindow(hwnd(window), SW_RESTORE);
            }
        }
        force_foreground(window);
    }

    fn unicode(unit: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wScan: unit,
                    dwFlags: if up {
                        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                    } else {
                        KEYEVENTF_UNICODE
                    },
                    ..Default::default()
                },
            },
        }
    }

    /// Let go of modifiers the user may still be holding (the wake keys).
    fn release_modifiers() -> Vec<INPUT> {
        [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN, VK_CONTROL]
            .into_iter()
            .map(|k| key(k, true))
            .collect()
    }

    /// Type text into the window in front, as if from the keyboard. With
    /// `soft_newlines` a new line is Shift+Enter, which starts a new line
    /// without sending in chat apps (Teams, Slack…); otherwise it is Enter.
    pub fn type_text(text: &str, soft_newlines: bool) {
        let mut inputs = release_modifiers();
        for ch in text.chars() {
            match ch {
                '\n' if soft_newlines => inputs.extend([
                    key(VK_SHIFT, false),
                    key(VK_RETURN, false),
                    key(VK_RETURN, true),
                    key(VK_SHIFT, true),
                ]),
                '\n' => inputs.extend([key(VK_RETURN, false), key(VK_RETURN, true)]),
                '\t' => inputs.extend([key(VK_TAB, false), key(VK_TAB, true)]),
                _ => {
                    let mut buf = [0u16; 2];
                    for unit in ch.encode_utf16(&mut buf) {
                        inputs.extend([unicode(*unit, false), unicode(*unit, true)]);
                    }
                }
            }
        }
        // In batches, so a long text does not overflow the input queue.
        for chunk in inputs.chunks(200) {
            unsafe {
                SendInput(chunk, size_of::<INPUT>() as i32);
            }
            sleep(Duration::from_millis(10));
        }
    }

    /// Press a chord such as Ctrl+T in the window in front.
    pub fn press_keys(keys: &crate::actions::Keys) {
        let mods: Vec<VIRTUAL_KEY> = [
            (keys.ctrl, VK_CONTROL),
            (keys.alt, VK_MENU),
            (keys.shift, VK_SHIFT),
            (keys.win, VK_LWIN),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, k)| k)
        .collect();
        // Arrows, Home/End, Page Up/Down, Insert and Delete are "extended"
        // keys; without the flag some apps read them as the number pad.
        let extended = matches!(keys.vk, 0x21..=0x28 | 0x2D | 0x2E);
        let main = |up: bool| {
            let mut i = key(VIRTUAL_KEY(keys.vk), up);
            if extended {
                unsafe { i.Anonymous.ki.dwFlags |= KEYEVENTF_EXTENDEDKEY };
            }
            i
        };
        let mut inputs = release_modifiers();
        inputs.extend(mods.iter().map(|&k| key(k, false)));
        inputs.extend([main(false), main(true)]);
        inputs.extend(mods.iter().rev().map(|&k| key(k, true)));
        unsafe {
            SendInput(&inputs, size_of::<INPUT>() as i32);
        }
    }

    /// Give focus back to `window` and press Ctrl+V there.
    pub fn paste_into(window: isize) {
        force_foreground(window);
        sleep(Duration::from_millis(120));
        // Release any modifier the user may still be holding (Ctrl+Enter
        // triggers the paste), then send a clean Ctrl+V.
        let inputs = [
            key(VK_SHIFT, true),
            key(VK_MENU, true),
            key(VK_LWIN, true),
            key(VK_RWIN, true),
            key(VK_CONTROL, true),
            key(VK_CONTROL, false),
            key(VK_V, false),
            key(VK_V, true),
            key(VK_CONTROL, true),
        ];
        unsafe {
            SendInput(&inputs, size_of::<INPUT>() as i32);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Foreground, RawImage};
    pub fn foreground() -> Foreground {
        Foreground::default()
    }
    /// UTC stands in for local time off Windows (development only).
    pub fn local_now() -> (i64, u32, u32, u32, u32) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (y, m, d) = crate::schedule::civil_from_days(secs.div_euclid(86_400));
        let minute = secs.rem_euclid(86_400) / 60;
        (y, m, d, (minute / 60) as u32, (minute % 60) as u32)
    }
    pub fn shell_open(_target: &str, _params: Option<&str>) -> Result<(), String> {
        Err("this only works on Windows".into())
    }
    pub fn list_windows() -> Vec<(isize, String, String)> {
        Vec::new()
    }
    pub fn focus_window(_window: isize) {}
    pub fn type_text(_text: &str, _soft_newlines: bool) {}
    pub fn press_keys(_keys: &crate::actions::Keys) {}
    pub fn capture_monitor(_window: isize) -> Option<RawImage> {
        None
    }
    pub fn monitor_origin(_window: isize) -> (i32, i32) {
        (0, 0)
    }
    pub fn force_foreground(_window: isize) {}
    pub fn paste_into(_window: isize) {}
}

pub use imp::*;

/// The current local time as a schedule minute stamp.
pub fn local_stamp() -> i64 {
    let (y, mo, d, h, mi) = local_now();
    crate::schedule::stamp(y, mo, d, h, mi)
}

/// Start-menu shortcuts: (name without ".lnk", full path). Both the
/// user's own Start menu and the one shared by all users.
pub fn start_menu_shortcuts() -> Vec<(String, std::path::PathBuf)> {
    use std::path::PathBuf;
    let roots: Vec<PathBuf> = [("APPDATA", ""), ("ProgramData", "")]
        .iter()
        .filter_map(|(var, _)| std::env::var(var).ok())
        .map(|base| PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs"))
        .collect();
    let mut out = Vec::new();
    fn walk(dir: &std::path::Path, depth: u32, out: &mut Vec<(String, std::path::PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 3 {
                    walk(&path, depth + 1, out);
                }
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
            {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    out.push((stem.to_string(), path.clone()));
                }
            }
        }
    }
    for root in roots {
        walk(&root, 0, &mut out);
    }
    out
}

/// Run a command the user saved, hidden, for at most `timeout`. Returns
/// its exit status and the end of what it printed.
pub fn run_saved_command(command: &str, timeout: std::time::Duration) -> Result<String, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd.exe");
        // The user wrote this command for cmd.exe, so it is passed as is.
        c.raw_arg("/D /S /C \"").raw_arg(command).raw_arg("\"");
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        cmd.current_dir(home);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start it: {e}"))?;
    let read = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out = read(Box::new(child.stdout.take().ok_or("no output")?));
    let err = read(Box::new(child.stderr.take().ok_or("no output")?));
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(status);
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let text = format!(
        "{}{}",
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    let tail: String = {
        let chars: Vec<char> = text.trim().chars().collect();
        chars[chars.len().saturating_sub(1500)..].iter().collect()
    };
    Ok(match status {
        Some(s) if s.success() => format!("Finished.\n{tail}"),
        Some(s) => format!("Exited with code {}.\n{tail}", s.code().unwrap_or(-1)),
        None => format!("Stopped after {} seconds.\n{tail}", timeout.as_secs()),
    })
}

/// Shrink a screenshot so its long edge is at most `max_edge` pixels and
/// encode it as JPEG. Smaller images upload faster and cost fewer tokens.
pub fn to_jpeg(img: RawImage, max_edge: u32) -> Option<Vec<u8>> {
    use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, RgbaImage};
    let rgba = RgbaImage::from_raw(img.width, img.height, img.rgba)?;
    let mut rgb = DynamicImage::ImageRgba8(rgba).into_rgb8();
    let long = img.width.max(img.height);
    if long > max_edge {
        let scale = max_edge as f32 / long as f32;
        let (w, h) = (
            (img.width as f32 * scale) as u32,
            (img.height as f32 * scale) as u32,
        );
        rgb = image::imageops::resize(&rgb, w.max(1), h.max(1), FilterType::Triangle);
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 80)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_commands_run_and_report() {
        let out = run_saved_command("echo hello", std::time::Duration::from_secs(20)).unwrap();
        assert!(out.starts_with("Finished."), "{out}");
        assert!(out.contains("hello"), "{out}");
        let failed = run_saved_command("exit 3", std::time::Duration::from_secs(20)).unwrap();
        assert!(failed.starts_with("Exited with code 3"), "{failed}");
    }

    #[test]
    fn local_time_is_sane() {
        let (y, m, d, h, mi) = local_now();
        assert!(y >= 2024 && (1..=12).contains(&m) && (1..=31).contains(&d) && h < 24 && mi < 60);
    }

    #[test]
    fn jpeg_is_downscaled() {
        let img = RawImage {
            width: 400,
            height: 200,
            rgba: vec![128; 400 * 200 * 4],
        };
        let bytes = to_jpeg(img, 100).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (100, 50));
    }
}
