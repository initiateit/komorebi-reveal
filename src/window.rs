//! Win32 window creation and management for the canvas overlay.

use windows::Win32::Foundation::{HWND, COLORREF};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

/// Encode a &str as null-terminated wide string.
pub fn wide_string(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Create the canvas overlay window (fullscreen, layered, topmost).
pub fn create_canvas_window(wndproc: WNDPROC) -> windows::core::Result<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let class_name = wide_string("WinCanvasClass");

        // Dark background brush (fallback if no screenshot)
        let bg_brush: HBRUSH = CreateSolidBrush(COLORREF(0x00201820));

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: wndproc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: HICON::default(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: bg_brush,
            lpszMenuName: PCWSTR::null(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hIconSm: HICON::default(),
        };

        RegisterClassExW(&wc);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);

        let title = wide_string("Win Canvas");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_LAYERED,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            screen_w,
            screen_h,
            None,
            None,
            hinstance,
            None,
        )?;

        // Start fully transparent (animation will fade in)
        set_window_alpha(hwnd, 0);

        let _ = UpdateWindow(hwnd);
        Ok(hwnd)
    }
}

/// Set the layered window alpha value.
pub fn set_window_alpha(hwnd: HWND, alpha: u8) {
    unsafe {
        const LWA_ALPHA: LAYERED_WINDOW_ATTRIBUTES_FLAGS = LAYERED_WINDOW_ATTRIBUTES_FLAGS(2);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
    }
}

/// Capture the desktop wallpaper to an HBITMAP with a heavy blur applied.
/// Two-pass 1/32 downsample + GDI+ bicubic upsample — completely unrecognisable,
/// smooth result with minimal transient memory (~500KB vs ~45MB for D3D11).
pub fn capture_screen(screen_w: i32, screen_h: i32) -> HBITMAP {
    use windows::Win32::Graphics::GdiPlus::{
        GdipCreateBitmapFromHBITMAP, GdipCreateFromHDC, GdipDeleteGraphics, GdipDisposeImage,
        GdipDrawImageRectI, GdipLoadImageFromFile, GdipSetInterpolationMode, GpBitmap,
        GpGraphics, GpImage, InterpolationModeHighQualityBilinear, Status,
    };
    unsafe {
        let hdc_screen = GetDC(None);

        // Output bitmap (full resolution)
        let hdc_out = CreateCompatibleDC(hdc_screen);
        let hbm_out = CreateCompatibleBitmap(hdc_screen, screen_w, screen_h);
        let old_out = SelectObject(hdc_out, hbm_out);

        // Pass 1: downsample to 1/8
        let s1_w = (screen_w / 16).max(1);
        let s1_h = (screen_h / 16).max(1);
        let hdc_s1 = CreateCompatibleDC(hdc_screen);
        let hbm_s1 = CreateCompatibleBitmap(hdc_screen, s1_w, s1_h);
        let old_s1 = SelectObject(hdc_s1, hbm_s1);

        // Pass 2: downsample to 1/32 (1/4 of pass 1)
        let s2_w = (screen_w / 128).max(1);
        let s2_h = (screen_h / 128).max(1);
        let hdc_s2 = CreateCompatibleDC(hdc_screen);
        let hbm_s2 = CreateCompatibleBitmap(hdc_screen, s2_w, s2_h);
        let old_s2 = SelectObject(hdc_s2, hbm_s2);

        // Load wallpaper at pass-1 size directly (no windows on top of it)
        let mut loaded_wallpaper = false;
        let mut path = vec![0u16; 260];
        if SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            260,
            Some(path.as_mut_ptr() as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        ).is_ok() {
            let mut image: *mut GpImage = std::ptr::null_mut();
            if GdipLoadImageFromFile(PCWSTR(path.as_ptr()), &mut image) == Status(0) {
                let mut g: *mut GpGraphics = std::ptr::null_mut();
                if GdipCreateFromHDC(hdc_s1, &mut g) == Status(0) {
                    let _ = GdipSetInterpolationMode(g, InterpolationModeHighQualityBilinear);
                    let _ = GdipDrawImageRectI(g, image, 0, 0, s1_w, s1_h);
                    let _ = GdipDeleteGraphics(g);
                    loaded_wallpaper = true;
                }
                let _ = GdipDisposeImage(image);
            }
        }
        if !loaded_wallpaper {
            SetStretchBltMode(hdc_s1, STRETCH_HALFTONE);
            let _ = StretchBlt(hdc_s1, 0, 0, s1_w, s1_h, hdc_screen, 0, 0, screen_w, screen_h, SRCCOPY);
        }

        // Second downscale pass: 1/8 → 1/32
        SetStretchBltMode(hdc_s2, STRETCH_HALFTONE);
        let _ = StretchBlt(hdc_s2, 0, 0, s2_w, s2_h, hdc_s1, 0, 0, s1_w, s1_h, SRCCOPY);

        // Deselect hbm_s2 before GDI+ touches it
        SelectObject(hdc_s2, old_s2);

        // Upsample with GDI+ bicubic — smooth interpolation hides block artefacts
        let mut gp_bm: *mut GpBitmap = std::ptr::null_mut();
        if GdipCreateBitmapFromHBITMAP(hbm_s2, HPALETTE::default(), &mut gp_bm) == Status(0) {
            let mut g: *mut GpGraphics = std::ptr::null_mut();
            if GdipCreateFromHDC(hdc_out, &mut g) == Status(0) {
                let _ = GdipSetInterpolationMode(g, InterpolationModeHighQualityBilinear);
                // 12.5% overscan on each side — bilinear edge-artifacts are most visible at the edges, so this ensures they
                // gets pushed outside the DC boundary and clipped, regardless of
                // resolution or scale factor.
                let ox = screen_w * 15 / 100;
                let oy = screen_h * 15 / 100;
                let _ = GdipDrawImageRectI(g, gp_bm as *mut GpImage,
                    -ox, -oy, screen_w + ox * 2, screen_h + oy * 2);
                let _ = GdipDeleteGraphics(g);
            }
            let _ = GdipDisposeImage(gp_bm as *mut GpImage);
        } else {
            // GDI+ fallback: plain upsample
            SetStretchBltMode(hdc_out, STRETCH_HALFTONE);
            let _ = StretchBlt(hdc_out, 0, 0, screen_w, screen_h, hdc_s2, 0, 0, s2_w, s2_h, SRCCOPY);
        }

        // Cleanup
        let _ = DeleteObject(hbm_s2);
        let _ = DeleteDC(hdc_s2);
        SelectObject(hdc_s1, old_s1);
        let _ = DeleteObject(hbm_s1);
        let _ = DeleteDC(hdc_s1);
        SelectObject(hdc_out, old_out);
        let _ = DeleteDC(hdc_out);
        let _ = ReleaseDC(None, hdc_screen);

        hbm_out
    }
}

/// Free an HBITMAP.
pub fn free_bitmap(hbm: HBITMAP) {
    if !hbm.0.is_null() {
        unsafe {
            let _ = DeleteObject(hbm);
        }
    }
}

/// Show the canvas window.
pub fn show_canvas(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Hide the canvas window.
pub fn hide_canvas(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// Get screen dimensions.
pub fn get_screen_size() -> (i32, i32) {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
        (w, h)
    }
}

/// Bring a window to the foreground (like Alt+Tab selection).
pub fn activate_window(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}
