//! Enumerate visible top-level windows for the canvas.

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassLongPtrW, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
    IsIconic, IsWindowVisible, SendMessageW, GWL_EXSTYLE, GWL_STYLE, ICON_BIG, GCL_HICON,
    WM_GETICON, WS_EX_TOOLWINDOW, WS_EX_NOACTIVATE, WS_CHILD, HICON,
};

/// Extract the program name from a window title.
/// Takes the last 3 words from the title, excluding common separators.
fn extract_program_name(title: &str) -> String {
    // Split on common separators and take the last meaningful part
    let separators = ['-', '–', '—', '|', ':', '·', '[', ']'];
    let parts: Vec<&str> = title
        .split(|c: char| separators.contains(&c))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    let last_part = parts.last().unwrap_or(&title);

    // Extract last 3 words from the remaining part
    let words: Vec<&str> = last_part.split_whitespace().collect();
    if words.len() >= 3 {
        format!("{} {} {}", words[words.len() - 3], words[words.len() - 2], words[words.len() - 1])
    } else if words.len() == 2 {
        format!("{} {}", words[0], words[1])
    } else if words.len() == 1 {
        words[0].to_string()
    } else {
        title.to_string()
    }
}

/// Information about an enumerated window.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub title: String,
    pub icon: HICON,
}

/// Enumerate all visible top-level application windows.
/// Filters out tool windows, invisible windows, cloaked (UWP) windows, etc.
pub fn enumerate_windows() -> Vec<WindowInfo> {
    let mut results: Vec<WindowInfo> = Vec::new();

    unsafe {
        let _ = EnumWindows(
            Some(enum_callback),
            LPARAM(&mut results as *mut Vec<WindowInfo> as isize),
        );
    }

    results
}

unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let results = &mut *(lparam.0 as *mut Vec<WindowInfo>);

    // Must be visible
    if !IsWindowVisible(hwnd).as_bool() {
        return TRUE;
    }

    // Skip child windows
    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    if style & WS_CHILD.0 != 0 {
        return TRUE;
    }

    // Skip tool windows and noactivate windows
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return TRUE;
    }
    if ex_style & WS_EX_NOACTIVATE.0 != 0 {
        return TRUE;
    }

    // Skip minimized windows (we can still thumbnail them, but they show blank)
    // Actually, let's include minimized — DWM can still show last known content
    // if IsIconic(hwnd).as_bool() {
    //     return TRUE;
    // }
    let _ = IsIconic(hwnd); // silence unused warning

    // Must have a title
    let title_len = GetWindowTextLengthW(hwnd);
    if title_len == 0 {
        return TRUE;
    }

    // Check if cloaked (hidden UWP apps, virtual desktop windows on other desktops)
    let mut cloaked: u32 = 0;
    let hr = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut _,
        std::mem::size_of::<u32>() as u32,
    );
    if hr.is_ok() && cloaked != 0 {
        return TRUE;
    }

    // Get the title
    let mut title_buf = vec![0u16; (title_len + 1) as usize];
    let copied = GetWindowTextW(hwnd, &mut title_buf);
    let title = String::from_utf16_lossy(&title_buf[..copied as usize]);

    // Skip empty titles after conversion
    if title.trim().is_empty() {
        return TRUE;
    }

    // Extract only the program name (last 2 words) for cleaner display
    let title = extract_program_name(&title);

    // Get window icon (try application icon first, then fall back to class icon)
    let icon = unsafe {
        let app_icon = SendMessageW(hwnd, WM_GETICON, WPARAM(ICON_BIG as usize), LPARAM(0));
        let app_icon_isize = app_icon.0 as isize;
        if app_icon_isize != 0 {
            HICON(app_icon_isize as *mut _)
        } else {
            let class_icon = GetClassLongPtrW(hwnd, GCL_HICON);
            HICON(class_icon as *mut _)
        }
    };

    results.push(WindowInfo { hwnd, title, icon });

    TRUE
}
