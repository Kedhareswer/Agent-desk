//! Seeing and pointing: what is on screen (through Windows UI Automation,
//! the accessibility interface screen readers use), moving the real mouse
//! cursor, clicking and scrolling, and turning PDF pages into pictures.
//!
//! UI Automation lets the AI find a button, link, tab or text box by its
//! name ("Send", "Type a message", "Address and search bar") in any app,
//! including the pages inside the user's own browser, without guessing
//! pixels. Checked on Windows 11 with Chrome 158, the new Teams and File
//! Explorer. Everything here runs on Windows only; other systems get stubs.

use serde::Serialize;

/// One thing on screen the AI can point at.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Element {
    /// Number the AI refers to it by (1-based, valid until the next look).
    pub id: usize,
    /// "button", "link", "edit", "tab", "item", "menu item", "checkbox"…
    pub kind: &'static str,
    pub name: String,
    /// Current text, for text boxes.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// Centre on screen, in physical pixels.
    pub x: i32,
    pub y: i32,
    #[serde(skip)]
    pub width: i32,
    #[serde(skip)]
    pub height: i32,
}

/// Which UI Automation control types are worth showing, and their names.
pub fn kind_name(control_type: i32) -> Option<&'static str> {
    Some(match control_type {
        50000 => "button",
        50002 => "checkbox",
        50003 => "combo box",
        50004 => "edit",
        50005 => "link",
        50007 => "list item",
        50011 => "menu item",
        50013 => "radio button",
        50015 => "slider",
        50019 => "tab",
        50024 => "tree item",
        50029 => "data item",
        50030 => "document",
        50031 => "split button",
        _ => return None,
    })
}

/// Turn the raw list into what the AI sees: on-screen, named (or a text
/// box), matching the optional query, at most `limit`, numbered.
pub fn shape(raw: Vec<(i32, String, String, [i32; 4])>, query: &str, limit: usize) -> Vec<Element> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<Element> = Vec::new();
    for (ct, name, value, [l, t, r, b]) in raw {
        let Some(kind) = kind_name(ct) else { continue };
        let name = name.trim().replace(['\n', '\r'], " ");
        if (name.is_empty() && kind != "edit") || r - l < 2 || b - t < 2 {
            continue;
        }
        // Off-screen or minimised windows report coordinates near -32000.
        if l < -10_000 || t < -10_000 {
            continue;
        }
        if !q.is_empty() && !name.to_lowercase().contains(&q) && !value.to_lowercase().contains(&q)
        {
            continue;
        }
        if out.iter().any(|e| {
            e.name == name
                && e.kind == kind
                && (e.x - (l + r) / 2).abs() < 4
                && (e.y - (t + b) / 2).abs() < 4
        }) {
            continue; // the same control reported twice (outer and inner)
        }
        out.push(Element {
            id: out.len() + 1,
            kind,
            name: name.chars().take(120).collect(),
            value: value.chars().take(200).collect(),
            x: (l + r) / 2,
            y: (t + b) / 2,
            width: r - l,
            height: b - t,
        });
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// The control that attaches a file in chat and mail apps.
pub fn is_attach_label(label: &str) -> bool {
    let l = label.to_lowercase();
    [
        "attach",
        "paperclip",
        "add files",
        "add a file",
        "upload a file",
        "insert file",
    ]
    .iter()
    .any(|w| l.contains(w))
}

/// The menu choice that picks a file from this computer.
pub fn is_local_upload_label(label: &str) -> bool {
    let l = label.to_lowercase();
    [
        "this device",
        "your computer",
        "this computer",
        "from computer",
        "this pc",
        "browse",
    ]
    .iter()
    .any(|w| l.contains(w))
        && !l.contains("cloud")
}

/// Labels on a button or link that mean "this sends, deletes, buys or
/// otherwise commits something". Clicking one waits for the user's OK.
pub fn is_consequential_label(label: &str) -> bool {
    const WORDS: [&str; 51] = [
        "upload",
        "allow",
        "authorize",
        "authorise",
        "grant",
        "agree",
        "yes",
        "ok",
        "continue",
        "commit",
        "join",
        "call",
        "update",
        "send",
        "post",
        "publish",
        "submit",
        "reply",
        "forward",
        "share",
        "delete",
        "remove",
        "discard",
        "erase",
        "trash",
        "archive",
        "buy",
        "purchase",
        "pay",
        "checkout",
        "check out",
        "order",
        "place order",
        "subscribe",
        "unsubscribe",
        "donate",
        "transfer",
        "confirm",
        "approve",
        "accept",
        "decline",
        "sign",
        "install",
        "uninstall",
        "run",
        "deploy",
        "merge",
        "close account",
        "log out",
        "sign out",
        "save",
    ];
    let l = label.to_lowercase();
    let words: Vec<&str> = l
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    WORDS.iter().any(|w| {
        if w.contains(' ') {
            l.contains(w)
        } else {
            words.iter().any(|x| x == w)
        }
    })
}

#[cfg(windows)]
mod imp {
    use super::{shape, Element};
    use std::{mem::size_of, thread::sleep, time::Duration};
    use windows::core::{Interface, HSTRING};
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomationInvokePattern,
        IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationValuePattern,
        TreeScope_Descendants, UIA_InvokePatternId, UIA_SelectionItemPatternId,
        UIA_TogglePatternId, UIA_ValuePatternId,
    };
    use windows::Win32::UI::HiDpi::{
        SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL,
        MOUSEINPUT, MOUSE_EVENT_FLAGS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetForegroundWindow, SetCursorPos,
    };

    /// UI Automation, cursor positions and screenshots all in physical
    /// pixels, whatever the display scaling.
    fn physical_pixels() {
        unsafe {
            let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    fn automation() -> Result<IUIAutomation, String> {
        unsafe {
            // Either apartment works for these calls; keep whatever the
            // thread already has.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("Windows UI Automation is not available: {e}"))
        }
    }

    /// The named elements in a window (the one in front when `window` is 0).
    pub fn list_elements(window: isize, query: &str, limit: usize) -> Result<Vec<Element>, String> {
        physical_pixels();
        let uia = automation()?;
        unsafe {
            let hwnd = if window == 0 {
                GetForegroundWindow()
            } else {
                HWND(window as *mut _)
            };
            let root = uia
                .ElementFromHandle(hwnd)
                .map_err(|e| format!("Could not read that window: {e}"))?;
            let all = root
                .FindAll(
                    TreeScope_Descendants,
                    &uia.CreateTrueCondition().map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
            let n = all.Length().unwrap_or(0).min(4000);
            let mut raw = Vec::with_capacity(n as usize);
            for i in 0..n {
                let Ok(e) = all.GetElement(i) else { continue };
                let ct = e.CurrentControlType().map(|c| c.0).unwrap_or(0);
                if super::kind_name(ct).is_none() {
                    continue;
                }
                let name = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                let value = if ct == 50004 {
                    e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                        .and_then(|v| v.CurrentValue())
                        .map(|b| b.to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let Ok(r) = e.CurrentBoundingRectangle() else {
                    continue;
                };
                raw.push((ct, name, value, [r.left, r.top, r.right, r.bottom]));
            }
            Ok(shape(raw, query, limit))
        }
    }

    /// The name and kind of whatever is under a screen point.
    pub fn element_at(x: i32, y: i32) -> Option<(String, &'static str)> {
        physical_pixels();
        let uia = automation().ok()?;
        unsafe {
            let e = uia.ElementFromPoint(POINT { x, y }).ok()?;
            let name = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
            let kind = super::kind_name(e.CurrentControlType().ok()?.0).unwrap_or("area");
            Some((name, kind))
        }
    }

    fn mouse(flags: MOUSE_EVENT_FLAGS, data: i32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    mouseData: data as u32,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        }
    }

    /// Where the user's cursor is (physical pixels).
    pub fn cursor_pos() -> (i32, i32) {
        physical_pixels();
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
        }
        (p.x, p.y)
    }

    /// Press what is at a screen point the way assistive tools do, without
    /// the mouse: invoke a button or link, toggle a check box, select a tab
    /// or list item. Looks at the element and up to two parents (a label
    /// inside a button). False when nothing there can be pressed this way.
    pub fn press_at(x: i32, y: i32) -> bool {
        physical_pixels();
        let Ok(uia) = automation() else { return false };
        unsafe {
            let Ok(mut e) = uia.ElementFromPoint(POINT { x, y }) else {
                return false;
            };
            let walker = uia.ControlViewWalker().ok();
            for _ in 0..3 {
                if let Ok(p) =
                    e.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                {
                    return p.Invoke().is_ok();
                }
                if let Ok(p) =
                    e.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId)
                {
                    return p.Toggle().is_ok();
                }
                if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(
                    UIA_SelectionItemPatternId,
                ) {
                    return p.Select().is_ok();
                }
                let Some(w) = &walker else { break };
                match w.GetParentElement(&e) {
                    Ok(parent) => e = parent,
                    Err(_) => break,
                }
            }
            false
        }
    }

    /// Give keyboard focus to the element at a point (a text box), without
    /// the mouse. False when it did not take focus.
    pub fn focus_at(x: i32, y: i32) -> bool {
        physical_pixels();
        let Ok(uia) = automation() else { return false };
        unsafe {
            let Ok(e) = uia.ElementFromPoint(POINT { x, y }) else {
                return false;
            };
            if e.SetFocus().is_err() {
                return false;
            }
            sleep(Duration::from_millis(60));
            e.CurrentHasKeyboardFocus().is_ok_and(|b| b.as_bool())
        }
    }

    /// Do something that needs the real cursor at a point (a click on a
    /// spot in a screenshot, scrolling), then put the user's cursor back
    /// where it was. It is away for well under a tenth of a second.
    pub fn at_point(x: i32, y: i32, act: impl FnOnce()) {
        physical_pixels();
        unsafe {
            let mut home = POINT::default();
            let _ = GetCursorPos(&mut home);
            let _ = SetCursorPos(x, y);
            sleep(Duration::from_millis(15));
            act();
            sleep(Duration::from_millis(35));
            let _ = SetCursorPos(home.x, home.y);
        }
    }

    pub fn click(right: bool, double: bool) {
        let (down, up) = if right {
            (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
        } else {
            (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
        };
        let mut inputs = vec![mouse(down, 0), mouse(up, 0)];
        if double {
            inputs.extend([mouse(down, 0), mouse(up, 0)]);
        }
        unsafe {
            SendInput(&inputs, size_of::<INPUT>() as i32);
        }
    }

    /// Scroll where the cursor is: positive `lines` goes down / right.
    pub fn scroll(lines: i32, horizontal: bool) {
        let input = if horizontal {
            mouse(MOUSEEVENTF_HWHEEL, lines * 120)
        } else {
            mouse(MOUSEEVENTF_WHEEL, -lines * 120)
        };
        unsafe {
            SendInput(&[input], size_of::<INPUT>() as i32);
        }
    }

    /// Every app in the Start menu, desktop and Store alike, as
    /// (name, "shell:AppsFolder\<id>") ready to open. Listing takes about a
    /// second, so the list is kept for ten minutes.
    pub fn start_apps() -> Vec<(String, std::path::PathBuf)> {
        use std::sync::{Mutex, OnceLock};
        use std::time::{Duration, Instant};
        type Cache = Mutex<Option<(Instant, Vec<(String, std::path::PathBuf)>)>>;
        static CACHE: OnceLock<Cache> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(None));
        if let Some((at, list)) = &*cache.lock().unwrap() {
            if at.elapsed() < Duration::from_secs(600) {
                return list.clone();
            }
        }
        let list = list_apps_folder().unwrap_or_default();
        *cache.lock().unwrap() = Some((Instant::now(), list.clone()));
        list
    }

    fn list_apps_folder() -> windows::core::Result<Vec<(String, std::path::PathBuf)>> {
        use windows::core::w;
        use windows::Win32::System::Com::{
            CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{
            BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName,
            SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
        };
        let mut out = Vec::new();
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let folder: IShellItem = SHCreateItemFromParsingName(w!("shell:AppsFolder"), None)?;
            let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
            loop {
                let mut one = [None];
                let mut got = 0u32;
                if items.Next(&mut one, Some(&mut got)).is_err() || got == 0 {
                    break;
                }
                let Some(item) = one[0].take() else { break };
                let (Ok(name), Ok(id)) = (
                    item.GetDisplayName(SIGDN_NORMALDISPLAY),
                    item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING),
                ) else {
                    continue;
                };
                if let (Ok(n), Ok(i)) = (name.to_string(), id.to_string()) {
                    out.push((
                        n,
                        std::path::PathBuf::from(format!("shell:AppsFolder\\{i}")),
                    ));
                }
                CoTaskMemFree(Some(name.0 as _));
                CoTaskMemFree(Some(id.0 as _));
            }
        }
        Ok(out)
    }

    /// Render the first pages of a PDF to PNG images with Windows' own PDF
    /// engine (no PDF library needed).
    pub fn pdf_pages(path: &str, max_pages: u32) -> Result<(u32, Vec<Vec<u8>>), String> {
        use windows::Data::Pdf::PdfDocument;
        use windows::Storage::StorageFile;
        use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
        let err = |e: windows::core::Error| format!("Could not read the PDF: {}", e.message());
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))
            .and_then(|op| op.join())
            .map_err(err)?;
        let doc = PdfDocument::LoadFromFileAsync(&file)
            .and_then(|op| op.join())
            .map_err(err)?;
        let count = doc.PageCount().map_err(err)?;
        let mut pages = Vec::new();
        for i in 0..count.min(max_pages) {
            let page = doc.GetPage(i).map_err(err)?;
            let stream = InMemoryRandomAccessStream::new().map_err(err)?;
            page.RenderToStreamAsync(&stream)
                .and_then(|op| op.join())
                .map_err(err)?;
            let size = stream.Size().map_err(err)? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).map_err(err)?)
                .map_err(err)?;
            reader
                .LoadAsync(size)
                .and_then(|op| op.join())
                .map_err(err)?;
            let mut bytes = vec![0u8; size as usize];
            reader.ReadBytes(&mut bytes).map_err(err)?;
            pages.push(bytes);
            let _ = stream
                .cast::<windows::Foundation::IClosable>()
                .map(|c| c.Close());
        }
        Ok((count, pages))
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Element;
    pub fn list_elements(
        _window: isize,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<Element>, String> {
        Err("Reading the screen works on Windows only.".into())
    }
    pub fn element_at(_x: i32, _y: i32) -> Option<(String, &'static str)> {
        None
    }
    pub fn cursor_pos() -> (i32, i32) {
        (0, 0)
    }
    pub fn press_at(_x: i32, _y: i32) -> bool {
        false
    }
    pub fn focus_at(_x: i32, _y: i32) -> bool {
        false
    }
    pub fn at_point(_x: i32, _y: i32, act: impl FnOnce()) {
        act()
    }
    pub fn click(_right: bool, _double: bool) {}
    pub fn scroll(_lines: i32, _horizontal: bool) {}
    pub fn pdf_pages(_path: &str, _max_pages: u32) -> Result<(u32, Vec<Vec<u8>>), String> {
        Err("Reading PDFs works on Windows only.".into())
    }
    pub fn start_apps() -> Vec<(String, std::path::PathBuf)> {
        Vec::new()
    }
}

pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(ct: i32, name: &str, l: i32, t: i32) -> (i32, String, String, [i32; 4]) {
        (ct, name.into(), String::new(), [l, t, l + 40, t + 20])
    }

    #[test]
    fn only_useful_on_screen_elements_are_listed() {
        let list = shape(
            vec![
                raw(50000, "Send", 100, 100),
                raw(50000, "Send", 102, 101), // inner copy of the same button
                raw(50020, "Some text", 0, 0), // plain text: not clickable
                raw(50000, "", 10, 10),       // unnamed button
                raw(50004, "", 10, 50),       // unnamed text box: still useful
                raw(50000, "Minimize", -32000, -32000),
                raw(50005, "Learn more", 300, 300),
            ],
            "",
            50,
        );
        let names: Vec<(&str, &str)> = list.iter().map(|e| (e.kind, e.name.as_str())).collect();
        assert_eq!(
            names,
            [("button", "Send"), ("edit", ""), ("link", "Learn more")]
        );
        assert_eq!(list.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!((list[0].x, list[0].y), (120, 110));
    }

    #[test]
    fn query_and_limit() {
        let many: Vec<_> = (0..30)
            .map(|i| raw(50000, &format!("Item {i}"), i * 50, 0))
            .collect();
        assert_eq!(shape(many.clone(), "", 10).len(), 10);
        assert_eq!(
            shape(many, "item 2", 50)
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            [
                "Item 2", "Item 20", "Item 21", "Item 22", "Item 23", "Item 24", "Item 25",
                "Item 26", "Item 27", "Item 28", "Item 29"
            ]
        );
    }

    #[test]
    fn attach_labels() {
        assert!(is_attach_label("Attach files"));
        assert!(is_attach_label("Attach"));
        assert!(!is_attach_label("Send"));
        assert!(is_local_upload_label("Upload from this device"));
        assert!(is_local_upload_label("Upload from your computer"));
        assert!(!is_local_upload_label("Attach cloud files"));
        assert!(!is_local_upload_label("Browse cloud files"));
    }

    #[test]
    fn consequential_labels() {
        for yes in [
            "Upload",
            "Allow",
            "Authorize app",
            "Grant access",
            "I agree",
            "Yes",
            "OK",
            "Continue",
            "Commit changes",
            "Join meeting",
            "Call",
            "Update now",
        ] {
            assert!(is_consequential_label(yes), "{yes}");
        }
        for yes in [
            "Send",
            "Send now",
            "Delete conversation",
            "Place order",
            "Buy now",
            "Pay £20",
            "Submit",
            "Reply all",
            "Merge pull request",
            "Sign out",
            "Save",
        ] {
            assert!(is_consequential_label(yes), "{yes}");
        }
        for no in [
            "Attach files",
            "Type a message",
            "Learn more",
            "New tab",
            "Chat Kedhareswer Naidu (You)",
            "Search docs",
            "Sending tips",
            "Settings",
            "Settings",
        ] {
            assert!(!is_consequential_label(no), "{no}");
        }
    }
}
