//! Everything that talks to the operating system directly: which window is
//! in front, what the screen looks like, and pasting into another app.
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
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, GetMonitorInfoW, MonitorFromWindow, ReleaseDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, MONITORINFO,
        MONITOR_DEFAULTTOPRIMARY, SRCCOPY,
    };
    use windows::Win32::System::Threading::{
        AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_V,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
        SetForegroundWindow,
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
            let mut buf = [0u16; 512];
            let n = GetWindowTextW(h, &mut buf).max(0) as usize;
            let title = String::from_utf16_lossy(&buf[..n]);

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
            Foreground {
                hwnd: h.0 as isize,
                title,
                process,
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
    pub fn capture_monitor(_window: isize) -> Option<RawImage> {
        None
    }
    pub fn force_foreground(_window: isize) {}
    pub fn paste_into(_window: isize) {}
}

pub use imp::*;

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
