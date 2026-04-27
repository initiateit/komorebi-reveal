//! Win-Canvas: An infinite canvas for managing open windows.
//!
//! Press Ctrl+Alt+Space to toggle the canvas overlay.
//! Features: wallpaper background, fade-in animation, persistent layout.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod canvas;
mod dwm;
mod enumerate;
mod hotkey;
mod input;
mod state;
mod window;

use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::ptr::{addr_of, addr_of_mut};

use std::result::Result::Ok;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::GdiPlus::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::PCWSTR;

use canvas::{Canvas, SourceInfo};
use dwm::Thumbnail;

// GDI+ token
static mut GDIPLUS_TOKEN: usize = 0;
static mut SHADOW_IMAGE: *mut GpImage = std::ptr::null_mut();

// Animation constants
const TIMER_FADE_IN: usize = 1;
const TIMER_SCROLL_ANIM: usize = 2;
const TIMER_TEXT_FADE_IN: usize = 3;
const ANIM_INTERVAL_MS: u32 = 16;
const ANIM_STEPS: u32 = 10;
const TEXT_ANIM_STEPS: u32 = 10;
const TARGET_ALPHA: u8 = 255;

fn ease_out(t: f64) -> f64 {
    1.0 - (1.0 - t).powi(3)
}

/// Simple debug logger
fn log_debug(msg: &str) {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    let log_dir = std::path::PathBuf::from(&appdata).join("win-canvas");
    let _ = fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("debug.log");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(f, "{}", msg);
    }
    #[cfg(debug_assertions)]
    eprintln!("{}", msg);
}

struct AppState {
    canvas: Canvas,
    thumbnails: Vec<Thumbnail>,
    visible: bool,
    canvas_hwnd: HWND,
    drag_moved: bool,
    click_target: Option<usize>,
    bg_bitmap: HBITMAP,
    anim_step: u32,
    anim_active: bool,
    text_anim_step: u32,
    text_anim_active: bool,
    current_alpha: u8,
}

impl AppState {
    fn new(screen_w: i32, screen_h: i32) -> Self {
        Self {
            canvas: Canvas::new(screen_w, screen_h),
            thumbnails: Vec::new(),
            visible: false,
            canvas_hwnd: HWND::default(),
            drag_moved: false,
            click_target: None,
            bg_bitmap: HBITMAP::default(),
            anim_step: 0,
            anim_active: false,
            text_anim_step: 0,
            text_anim_active: false,
            current_alpha: 0,
        }
    }

    fn refresh(&mut self) {
        self.thumbnails.clear();
        self.canvas.windows.clear();

        let windows = enumerate::enumerate_windows();
        log_debug(&format!("Enumerated {} windows", windows.len()));

        let mut source_infos = Vec::new();

        for winfo in &windows {
            if winfo.hwnd == self.canvas_hwnd {
                continue;
            }
            match Thumbnail::register(self.canvas_hwnd, winfo.hwnd) {
                Ok(thumb) => {
                    let idx = self.thumbnails.len();
                    source_infos.push(SourceInfo {
                        thumb_index: idx,
                        width: thumb.source_width,
                        height: thumb.source_height,
                        title: winfo.title.clone(),
                        icon: winfo.icon,
                    });
                    self.thumbnails.push(thumb);
                }
                Err(e) => {
                    log_debug(&format!(
                        "Failed to register thumbnail for '{}': {:?}",
                        winfo.title, e
                    ));
                }
            }
        }

        log_debug(&format!("Registered {} thumbnails", self.thumbnails.len()));

        let saved = state::load_state();
        self.canvas.layout_grid(&source_infos, saved.as_ref());
        self.update_all_thumbnails();
    }

    fn update_all_thumbnails(&self) {
        let scale = if self.anim_active {
            let t = self.anim_step as f64 / ANIM_STEPS as f64;
            0.92 + 0.08 * ease_out(t)
        } else {
            1.0
        };

        for cw in &self.canvas.windows {
            if cw.thumb_index < self.thumbnails.len() {
                let rect = self.canvas.canvas_to_screen_rect(cw, scale);
                if rect.right > 0
                    && rect.bottom > 0
                    && rect.left < self.canvas.screen_w
                    && rect.top < self.canvas.screen_h
                {
                    let _ = self.thumbnails[cw.thumb_index]
                        .update(rect, self.current_alpha, false);
                } else {
                    let _ = self.thumbnails[cw.thumb_index].hide();
                }
            }
        }
    }

    fn toggle(&mut self) {
        log_debug(&format!("Toggle called, visible={}", self.visible));
        if self.visible {
            self.hide();
        } else {
            self.show();
        }
    }

    fn show(&mut self) {
        log_debug("show() called");
        self.visible = true;

        // Capture the current screen as background
        if !self.bg_bitmap.0.is_null() {
            window::free_bitmap(self.bg_bitmap);
            self.bg_bitmap = HBITMAP::default();
        }
        self.bg_bitmap =
            window::capture_screen(self.canvas.screen_w, self.canvas.screen_h);
        log_debug(&format!("Screen captured: bitmap={:?}", self.bg_bitmap.0));

        self.current_alpha = 0;
        window::set_window_alpha(self.canvas_hwnd, 0);

        self.refresh();
        window::show_canvas(self.canvas_hwnd);

        // Start fade-in animation
        self.anim_step = 0;
        self.anim_active = true;
        unsafe {
            SetTimer(self.canvas_hwnd, TIMER_FADE_IN, ANIM_INTERVAL_MS, None);
        }
        log_debug("show() complete, animation started");
    }

    fn hide(&mut self) {
        log_debug("hide() called");
        let saved = self.canvas.to_saved_state();
        state::save_state(&saved);

        self.visible = false;
        self.anim_active = false;
        self.text_anim_active = false;
        unsafe {
            let _ = KillTimer(self.canvas_hwnd, TIMER_FADE_IN);
            let _ = KillTimer(self.canvas_hwnd, TIMER_TEXT_FADE_IN);
        }

        for thumb in &self.thumbnails {
            let _ = thumb.hide();
        }
        window::hide_canvas(self.canvas_hwnd);
    }

    fn tick_animation(&mut self) {
        self.anim_step += 1;
        if self.anim_step >= ANIM_STEPS {
            self.anim_step = ANIM_STEPS;
            self.anim_active = false;
            unsafe {
                let _ = KillTimer(self.canvas_hwnd, TIMER_FADE_IN);
                
                // Start text fade in
                self.text_anim_active = true;
                self.text_anim_step = 0;
                let _ = SetTimer(self.canvas_hwnd, TIMER_TEXT_FADE_IN, ANIM_INTERVAL_MS, None);
            }
        }

        let t = self.anim_step as f64 / ANIM_STEPS as f64;
        let eased = ease_out(t);
        self.current_alpha = (TARGET_ALPHA as f64 * eased) as u8;

        window::set_window_alpha(self.canvas_hwnd, self.current_alpha);
        self.update_all_thumbnails();

        unsafe {
            let _ = InvalidateRect(self.canvas_hwnd, None, true);
        }
    }

    fn tick_text_animation(&mut self) {
        self.text_anim_step += 1;
        if self.text_anim_step >= TEXT_ANIM_STEPS {
            self.text_anim_step = TEXT_ANIM_STEPS;
            self.text_anim_active = false;
            unsafe {
                let _ = KillTimer(self.canvas_hwnd, TIMER_TEXT_FADE_IN);
            }
        }

        unsafe {
            let _ = InvalidateRect(self.canvas_hwnd, None, true);
        }
    }
}

thread_local! {
    static APP_STATE: RefCell<Option<AppState>> = const { RefCell::new(None) };
}

fn with_state<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut AppState) -> R,
{
    APP_STATE.with(|cell| {
        if let Ok(mut opt) = cell.try_borrow_mut() {
            opt.as_mut().map(f)
        } else {
            // State is currently borrowed (re-entrant call), skip
            None
        }
    })
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            if wparam.0 as i32 == hotkey::HOTKEY_TOGGLE_CANVAS {
                with_state(|s| s.toggle());
            }
            LRESULT(0)
        }

        WM_TIMER => {
            if wparam.0 == TIMER_FADE_IN {
                with_state(|s| s.tick_animation());
            } else if wparam.0 == TIMER_TEXT_FADE_IN {
                with_state(|s| s.tick_text_animation());
            } else if wparam.0 == TIMER_SCROLL_ANIM {
                with_state(|s| {
                    if s.canvas.update_scroll_animation() {
                        s.update_all_thumbnails();
                        let _ = InvalidateRect(hwnd, None, true);
                    } else {
                        // Animation complete, kill timer
                        unsafe {
                            let _ = KillTimer(hwnd, TIMER_SCROLL_ANIM);
                        }
                    }
                });
            }
            LRESULT(0)
        }

        WM_KEYDOWN => {
            let vk = wparam.0 as u32;
            if vk == 0x1B {
                // ESC - close the window
                with_state(|s| s.hide());
            } else if vk == 0x25 || vk == 0x26 {
                // Left or Up arrow - previous window
                with_state(|s| {
                    s.canvas.prev_window();
                    unsafe {
                        let _ = SetTimer(hwnd, TIMER_SCROLL_ANIM, ANIM_INTERVAL_MS, None);
                    }
                    s.update_all_thumbnails();
                    let _ = InvalidateRect(hwnd, None, true);
                });
            } else if vk == 0x27 || vk == 0x28 {
                // Right or Down arrow - next window
                with_state(|s| {
                    s.canvas.next_window();
                    unsafe {
                        let _ = SetTimer(hwnd, TIMER_SCROLL_ANIM, ANIM_INTERVAL_MS, None);
                    }
                    s.update_all_thumbnails();
                    let _ = InvalidateRect(hwnd, None, true);
                });
            } else if vk == 0x0D {
                // Enter key - activate the selected window
                with_state(|s| {
                    if let Some(idx) = s.canvas.get_active_window() {
                        if idx < s.canvas.windows.len() {
                            let ti = s.canvas.windows[idx].thumb_index;
                            if ti < s.thumbnails.len() {
                                let target = s.thumbnails[ti].source_hwnd;
                                s.hide();
                                window::activate_window(target);
                            }
                        }
                    }
                });
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            let (x, y) = input::mouse_coords(lparam.0);
            with_state(|s| {
                let hit = s.canvas.hit_test(x, y);
                s.click_target = hit;
                s.drag_moved = false;
                // Set active window and scroll to it
                if let Some(idx) = hit {
                    s.canvas.set_active_window(idx);
                    unsafe {
                        let _ = SetTimer(hwnd, TIMER_SCROLL_ANIM, ANIM_INTERVAL_MS, None);
                    }
                }
                // Window dragging disabled - only canvas panning with right-click
            });
            LRESULT(0)
        }

        WM_LBUTTONUP => {
            with_state(|s| {
                if !s.drag_moved {
                    if let Some(idx) = s.click_target {
                        if idx < s.canvas.windows.len() {
                            let ti = s.canvas.windows[idx].thumb_index;
                            if ti < s.thumbnails.len() {
                                let target = s.thumbnails[ti].source_hwnd;
                                s.hide();
                                window::activate_window(target);
                            }
                        }
                    }
                }
                s.canvas.end_drag();
                s.click_target = None;
                let _ = ReleaseCapture();
                s.update_all_thumbnails();
                let _ = InvalidateRect(hwnd, None, true);
            });
            LRESULT(0)
        }

        WM_RBUTTONDOWN => {
            let (x, y) = input::mouse_coords(lparam.0);
            with_state(|s| {
                s.canvas.start_pan(x, y);
                SetCapture(hwnd);
            });
            LRESULT(0)
        }

        WM_RBUTTONUP => {
            with_state(|s| {
                s.canvas.end_pan();
                let _ = ReleaseCapture();
                s.update_all_thumbnails();
                let _ = InvalidateRect(hwnd, None, true);
            });
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            let (x, y) = input::mouse_coords(lparam.0);
            with_state(|s| {
                if s.canvas.drag_target.is_some() {
                    s.drag_moved = true;
                    s.canvas.update_drag(x, y);
                    s.update_all_thumbnails();
                    let _ = InvalidateRect(hwnd, None, true);
                } else if s.canvas.panning {
                    s.canvas.update_pan(x, y);
                    s.update_all_thumbnails();
                    let _ = InvalidateRect(hwnd, None, true);
                }
            });
            LRESULT(0)
        }

        WM_MOUSEWHEEL => {
            let (x, y) = input::mouse_coords(lparam.0);
            let delta = input::wheel_delta(wparam.0);
            let ctrl_pressed = (wparam.0 & 0x0008) != 0; // MK_CONTROL

            with_state(|s| {
                if ctrl_pressed {
                    // Ctrl+Wheel = zoom
                    let mut pt = POINT {
                        x: x as i32,
                        y: y as i32,
                    };
                    let _ = ScreenToClient(hwnd, &mut pt);
                    s.canvas.zoom_at(pt.x as f64, pt.y as f64, delta);
                } else {
                    // Wheel without Ctrl = navigate through windows
                    if delta > 0.0 {
                        s.canvas.prev_window();
                    } else {
                        s.canvas.next_window();
                    }
                    unsafe {
                        let _ = SetTimer(hwnd, TIMER_SCROLL_ANIM, ANIM_INTERVAL_MS, None);
                    }
                }
                s.update_all_thumbnails();
                let _ = InvalidateRect(hwnd, None, true);
            });
            LRESULT(0)
        }

        WM_ERASEBKGND => {
            // Don't erase - we'll handle all painting in WM_PAINT to prevent flicker
            LRESULT(1)
        }

        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            with_state(|s| {
                // Use double buffering: draw to off-screen bitmap first
                let hdc_buffer = CreateCompatibleDC(hdc);
                let hbm_buffer = CreateCompatibleBitmap(hdc, s.canvas.screen_w, s.canvas.screen_h);
                let _old_buffer = SelectObject(hdc_buffer, hbm_buffer);

                // Draw background first (captured screen)
                if !s.bg_bitmap.0.is_null() {
                    let hdc_mem = CreateCompatibleDC(hdc_buffer);
                    let old = SelectObject(hdc_mem, s.bg_bitmap);

                    // Draw the captured screen to buffer
                    let _ = BitBlt(
                        hdc_buffer, 0, 0,
                        s.canvas.screen_w, s.canvas.screen_h,
                        hdc_mem, 0, 0, SRCCOPY,
                    );
                    SelectObject(hdc_mem, old);
                    let _ = DeleteDC(hdc_mem);
                }

                // Now draw borders and text to hdc_buffer
                SetBkMode(hdc_buffer, TRANSPARENT);
                SetTextColor(hdc_buffer, COLORREF(0x00E0E0E0));

                let scale = if s.anim_active {
                    let t = s.anim_step as f64 / ANIM_STEPS as f64;
                    0.92 + 0.08 * ease_out(t)
                } else {
                    1.0
                };

                // Draw borders to hdc_buffer
                for (idx, cw) in s.canvas.windows.iter().enumerate() {
                    let rect = s.canvas.canvas_to_screen_rect(cw, scale);
                    let is_active = s.canvas.get_active_window() == Some(idx);

                    // Use GDI+ for anti-aliased rounded corners
                    unsafe {
                        let mut graphics: *mut GpGraphics = std::ptr::null_mut();
                        if GdipCreateFromHDC(hdc_buffer, &mut graphics as *mut _ as *mut _) == Status(0) {
                            let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);

                            let mut pen: *mut GpPen = std::ptr::null_mut();
                            let (color, width) = (0x00, 5.0); // Transparent to hide the blue border
                            if GdipCreatePen1(color, width, UnitPixel, &mut pen as *mut _ as *mut _) == Status(0) {
                                let x = rect.left as f32;
                                let y = rect.top as f32;
                                let w = (rect.right - rect.left) as f32;
                                let h = (rect.bottom - rect.top) as f32;

                                let mut path: *mut GpPath = std::ptr::null_mut();
                                if GdipCreatePath(FillModeAlternate, &mut path as *mut _ as *mut _) == Status(0) {
                                    
                                    // Card Geometry
                                    let card_pad_sides = 32.0f32; // 32px padding sides/top
                                    let card_pad_bottom = 90.0f32; // 80px padding bottom for the label
                                    let cx = x - card_pad_sides;
                                    let cy = y - card_pad_sides;
                                    let cw = w + card_pad_sides * 2.0;
                                    let ch = h + card_pad_sides + card_pad_bottom;
                                    let r = 12.0f32; // 12px border radius
                                    let d = r * 2.0;

                                    let _ = GdipAddPathArc(path, cx, cy, d, d, 180.0, 90.0);
                                    let _ = GdipAddPathArc(path, cx + cw - d, cy, d, d, 270.0, 90.0);
                                    let _ = GdipAddPathArc(path, cx + cw - d, cy + ch - d, d, d, 0.0, 90.0);
                                    let _ = GdipAddPathArc(path, cx, cy + ch - d, d, d, 90.0, 90.0);
                                    let _ = GdipClosePathFigure(path);

                                    // Draw the White Backing Card (No Shadow!)
                                    if is_active {
                                        let mut card_fill: *mut GpSolidFill = std::ptr::null_mut();
                                        let opacity = 0x4D; // ~29% opacity white
                                        if GdipCreateSolidFill((opacity << 24) | 0xFFFFFF, &mut card_fill as *mut _ as *mut _) == Status(0) {
                                            let _ = GdipFillPath(graphics, card_fill as *mut _ as *mut GpBrush, path);
                                            let _ = GdipDeleteBrush(card_fill as *mut _ as *mut GpBrush);
                                        }

                                        let mut card_pen: *mut GpPen = std::ptr::null_mut();
                                        if GdipCreatePen1((0xBF << 24) | 0xFFFFFF, 4.0, UnitPixel, &mut card_pen as *mut _ as *mut _) == Status(0) {
                                            let _ = GdipDrawPath(graphics, card_pen, path);
                                            let _ = GdipDeletePen(card_pen);
                                        }
                                    }

                                    // 9-slice drop shadow using pre-rendered PNG (anchored to the DWM Thumbnail)
                                    if is_active && !(*addr_of!(SHADOW_IMAGE)).is_null() {
                                        let shadow = *addr_of!(SHADOW_IMAGE);
                                        let mt = 50.0f32;
                                        let ml = 50.0f32;
                                        let mr = 150.0f32;
                                        let mb = 150.0f32;
                                        
                                        // Scale factor to make the shadow visually larger
                                        let scale = 1.6f32; // Reverted back to a softer scale for the DWM window
                                        let d_mt = mt * scale;
                                        let d_ml = ml * scale;
                                        let d_mr = mr * scale;
                                        let d_mb = mb * scale;
                                        
                                        // Source dimensions
                                        let src_cx = 100.0f32;
                                        let src_cy = 100.0f32;
                                        
                                        // Destination dimensions mapped to the DWM THUMBNAIL bounds
                                        let dx0 = x - d_ml;
                                        let dx1 = x;
                                        let dx2 = x + w;
                                        
                                        let dy0 = y - d_mt;
                                        let dy1 = y;
                                        let dy2 = y + h;
                                        
                                        let attr: *mut GpImageAttributes = std::ptr::null_mut();
                                        
                                        let draw_slice = |dx: f32, dy: f32, dw: f32, dh: f32, sx: f32, sy: f32, sw: f32, sh: f32| {
                                            let _ = GdipDrawImageRectRect(graphics, shadow, dx, dy, dw, dh, sx, sy, sw, sh, UnitPixel, attr, 0isize as _, std::ptr::null_mut());
                                        };

                                        // Top row
                                        draw_slice(dx0, dy0, d_ml, d_mt, 0.0, 0.0, ml, mt); // Top-left
                                        draw_slice(dx1, dy0, w, d_mt, ml, 0.0, src_cx, mt); // Top-center
                                        draw_slice(dx2, dy0, d_mr, d_mt, ml + src_cx, 0.0, mr, mt); // Top-right

                                        // Middle row
                                        draw_slice(dx0, dy1, d_ml, h, 0.0, mt, ml, src_cy); // Mid-left
                                        draw_slice(dx1, dy1, w, h, ml, mt, src_cx, src_cy); // Center
                                        draw_slice(dx2, dy1, d_mr, h, ml + src_cx, mt, mr, src_cy); // Mid-right

                                        // Bottom row
                                        draw_slice(dx0, dy2, d_ml, d_mb, 0.0, mt + src_cy, ml, mb); // Bottom-left
                                        draw_slice(dx1, dy2, w, d_mb, ml, mt + src_cy, src_cx, mb); // Bottom-center
                                        draw_slice(dx2, dy2, d_mr, d_mb, ml + src_cx, mt + src_cy, mr, mb); // Bottom-right
                                    }


                                    let _ = GdipDeletePath(path);
                                }
                                let _ = GdipDeletePen(pen);
                            }
                            let _ = GdipDeleteGraphics(graphics);
                        }
                    }
                }

                // Draw text and icons
                let mut text_dc = hdc_buffer;
                let mut hdc_text = HDC::default();
                let mut hbm_text = HBITMAP::default();
                let mut old_text = HGDIOBJ::default();

                if !s.anim_active {
                    if s.text_anim_active {
                        hdc_text = CreateCompatibleDC(hdc_buffer);
                        hbm_text = CreateCompatibleBitmap(hdc_buffer, s.canvas.screen_w, s.canvas.screen_h);
                        old_text = SelectObject(hdc_text, hbm_text);
                        let _ = BitBlt(hdc_text, 0, 0, s.canvas.screen_w, s.canvas.screen_h, hdc_buffer, 0, 0, SRCCOPY);
                        text_dc = hdc_text;
                        SetBkMode(text_dc, TRANSPARENT);
                        SetTextColor(text_dc, COLORREF(0x00E0E0E0));
                    }

                    unsafe {
                        let mut graphics: *mut GpGraphics = std::ptr::null_mut();
                        if GdipCreateFromHDC(text_dc, &mut graphics as *mut _ as *mut _) == Status(0) {
                            let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
                            let _ = GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
                            let _ = GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);

                            // Using standard "Segoe UI" family prevents fallback to MS Sans Serif
                            let font_name = window::wide_string("Segoe UI");
                            let mut font_family: *mut GpFontFamily = std::ptr::null_mut();
                            let _ = GdipCreateFontFamilyFromName(PCWSTR(font_name.as_ptr()), std::ptr::null_mut(), &mut font_family);
                            
                            let mut font: *mut GpFont = std::ptr::null_mut();
                            // 1 = Bold, UnitPixel
                            let _ = GdipCreateFont(font_family, 15.0, 1, UnitPixel, &mut font);
                            
                            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
                            let _ = GdipCreateSolidFill(0xE6242430, &mut brush as *mut _ as *mut _);
                            
                            let mut format: *mut GpStringFormat = std::ptr::null_mut();
                            let _ = GdipCreateStringFormat(0, 0, &mut format);
                            let _ = GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter);
                            let _ = GdipSetStringFormatAlign(format, StringAlignmentNear);
                            let _ = GdipSetStringFormatLineAlign(format, StringAlignmentCenter);

                            for cw in s.canvas.windows.iter() {
                                let rect = s.canvas.canvas_to_screen_rect(cw, scale);
                                let icon_size = 20;
                                let icon_spacing = 6;

                                let tw: Vec<u16> = cw.title.encode_utf16().chain(std::iter::once(0)).collect();
                                
                                let mut bounding_box = RectF::default();
                                let _ = GdipMeasureString(
                                    graphics,
                                    PCWSTR(tw.as_ptr()),
                                    (tw.len() - 1) as i32,
                                    font,
                                    &RectF { X: 0.0, Y: 0.0, Width: 10000.0, Height: 10000.0 },
                                    format,
                                    &mut bounding_box,
                                    std::ptr::null_mut(),
                                    std::ptr::null_mut(),
                                );
                                
                                let text_width = bounding_box.Width as i32;
                                
                                let total_width = if !cw.icon.is_invalid() {
                                    text_width + icon_size + icon_spacing
                                } else {
                                    text_width
                                };
                                let start_x = rect.left + (rect.right - rect.left - total_width) / 2;

                                // Draw pill background
                                let pad_x = 14;
                                let pill_h = 32;
                                let pill_w = total_width + pad_x * 2;
                                let pill_x = start_x - pad_x;
                                let pill_y = rect.bottom + 30; // Distance below thumbnail

                                let mut pill_path: *mut GpPath = std::ptr::null_mut();
                                if GdipCreatePath(FillModeAlternate, &mut pill_path as *mut _ as *mut _) == Status(0) {
                                    let r = 12.0f32; // border radius
                                    let x = pill_x as f32;
                                    let y = pill_y as f32;
                                    let w = pill_w as f32;
                                    let h = pill_h as f32;
                                    let x2 = x + w;
                                    let y2 = y + h;

                                    let _ = GdipAddPathArc(pill_path, x2 - 2.0 * r, y, 2.0 * r, 2.0 * r, 270.0, 90.0);
                                    let _ = GdipAddPathLine(pill_path, x2, y + r, x2, y2 - r);
                                    let _ = GdipAddPathArc(pill_path, x2 - 2.0 * r, y2 - 2.0 * r, 2.0 * r, 2.0 * r, 0.0, 90.0);
                                    let _ = GdipAddPathLine(pill_path, x2 - r, y2, x + r, y2);
                                    let _ = GdipAddPathArc(pill_path, x, y2 - 2.0 * r, 2.0 * r, 2.0 * r, 90.0, 90.0);
                                    let _ = GdipAddPathLine(pill_path, x, y2 - r, x, y + r);
                                    let _ = GdipAddPathArc(pill_path, x, y, 2.0 * r, 2.0 * r, 180.0, 90.0);
                                    let _ = GdipAddPathLine(pill_path, x + r, y, x2 - r, y);
                                    let _ = GdipClosePathFigure(pill_path);

                                    let mut pill_brush: *mut GpSolidFill = std::ptr::null_mut();
                                    if GdipCreateSolidFill(0xFFE5E7EB, &mut pill_brush as *mut _ as *mut _) == Status(0) {
                                        let _ = GdipFillPath(graphics, pill_brush as *mut _ as *mut GpBrush, pill_path);
                                        let _ = GdipDeleteBrush(pill_brush as *mut _ as *mut GpBrush);
                                    }
                                    let _ = GdipDeletePath(pill_path);
                                }

                                if !cw.icon.is_invalid() {
                                    let mut gp_icon: *mut GpBitmap = std::ptr::null_mut();
                                    if GdipCreateBitmapFromHICON(cw.icon, &mut gp_icon) == Status(0) {
                                        let icon_y = pill_y + (pill_h - icon_size) / 2;
                                        let _ = GdipDrawImageRectI(graphics, gp_icon as *mut _ as *mut GpImage, start_x, icon_y, icon_size, icon_size);
                                        let _ = GdipDisposeImage(gp_icon as *mut _ as *mut GpImage);
                                    }
                                }

                                let text_x = if !cw.icon.is_invalid() {
                                    start_x + icon_size + icon_spacing
                                } else {
                                    start_x
                                };
                                
                                let rectf = RectF {
                                    X: text_x as f32,
                                    Y: pill_y as f32 + 2.0, // 2px offset for optical centering
                                    Width: (rect.right - text_x).max(1) as f32,
                                    Height: pill_h as f32,
                                };
                                
                                let _ = GdipDrawString(
                                    graphics,
                                    PCWSTR(tw.as_ptr()),
                                    (tw.len() - 1) as i32,
                                    font,
                                    &rectf,
                                    format,
                                    brush as *mut _ as *mut GpBrush,
                                );
                            }
                            
                            let _ = GdipDeleteStringFormat(format);
                            let _ = GdipDeleteBrush(brush as *mut _ as *mut GpBrush);
                            let _ = GdipDeleteFont(font);
                            let _ = GdipDeleteFontFamily(font_family);
                            let _ = GdipDeleteGraphics(graphics);
                        }
                    }

                    if s.text_anim_active {
                        let text_t = s.text_anim_step as f64 / TEXT_ANIM_STEPS as f64;
                        let text_alpha = (255.0 * ease_out(text_t)) as u8;
                        let bf = BLENDFUNCTION {
                            BlendOp: AC_SRC_OVER as u8,
                            BlendFlags: 0,
                            SourceConstantAlpha: text_alpha,
                            AlphaFormat: 0,
                        };
                        let _ = AlphaBlend(
                            hdc_buffer, 0, 0, s.canvas.screen_w, s.canvas.screen_h,
                            hdc_text, 0, 0, s.canvas.screen_w, s.canvas.screen_h,
                            bf
                        );
                        SelectObject(hdc_text, old_text);
                        let _ = DeleteObject(hbm_text);
                        let _ = DeleteDC(hdc_text);
                    }
                }

                // Zoom indicator
                let zoom_text = format!("{:.0}%", s.canvas.zoom * 100.0);
                let mut zw: Vec<u16> = zoom_text.encode_utf16().collect();
                let font_name = window::wide_string("Segoe UI");
                let bf_font = CreateFontW(
                    24, 0, 0, 0, 300, 0, 0, 0, 0, 0, 0, 0, 0,
                    PCWSTR(font_name.as_ptr()),
                );
                let of2 = SelectObject(hdc_buffer, bf_font);
                SetTextColor(hdc_buffer, COLORREF(0x00808080));
                let mut zr = RECT {
                    left: s.canvas.screen_w - 120, top: s.canvas.screen_h - 40,
                    right: s.canvas.screen_w - 10, bottom: s.canvas.screen_h - 10,
                };
                DrawTextW(hdc_buffer, &mut zw, &mut zr, DT_RIGHT | DT_SINGLELINE | DT_NOPREFIX);
                SelectObject(hdc_buffer, of2);
                let _ = DeleteObject(bf_font);

                // Finally, copy the fully composed buffer to the screen ONCE
                let _ = BitBlt(
                    hdc, 0, 0,
                    s.canvas.screen_w, s.canvas.screen_h,
                    hdc_buffer, 0, 0, SRCCOPY,
                );

                // Clean up buffer
                SelectObject(hdc_buffer, _old_buffer);
                let _ = DeleteObject(hbm_buffer);
                let _ = DeleteDC(hdc_buffer);
            });

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_DESTROY => {
            hotkey::unregister_hotkey(hwnd);
            with_state(|s| {
                if !s.bg_bitmap.0.is_null() {
                    window::free_bitmap(s.bg_bitmap);
                    s.bg_bitmap = HBITMAP::default();
                }
            });
            unsafe {
                let _ = KillTimer(hwnd, TIMER_FADE_IN);
                let _ = KillTimer(hwnd, TIMER_SCROLL_ANIM);
                if !(*addr_of!(SHADOW_IMAGE)).is_null() {
                    let _ = GdipDisposeImage(*addr_of!(SHADOW_IMAGE));
                    *addr_of_mut!(SHADOW_IMAGE) = std::ptr::null_mut();
                }
                if *addr_of!(GDIPLUS_TOKEN) != 0 {
                    GdiplusShutdown(*addr_of!(GDIPLUS_TOKEN));
                    *addr_of_mut!(GDIPLUS_TOKEN) = 0;
                }
            }
            PostQuitMessage(0);
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() {
    // Set up panic hook to log panics
    std::panic::set_hook(Box::new(|info| {
        log_debug(&format!("PANIC: {}", info));
    }));

    log_debug("=== Win-Canvas starting ===");

    // Initialize GDI+
    unsafe {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            DebugEventCallback: 0,
            SuppressBackgroundThread: false.into(),
            SuppressExternalCodecs: false.into(),
        };
        let mut token = 0usize;
        let result = GdiplusStartup(&mut token, &input, std::ptr::null_mut());
        if result == Status(0) {
            *addr_of_mut!(GDIPLUS_TOKEN) = token;
            log_debug("GDI+ initialized successfully");

            // Pre-warm the GDI+ font cache to eliminate the 4-second delay on first overlay load
            log_debug("Pre-warming GDI+ font cache...");
            let font_name = window::wide_string("Segoe UI");
            let mut font_family: *mut GpFontFamily = std::ptr::null_mut();
            if GdipCreateFontFamilyFromName(PCWSTR(font_name.as_ptr()), std::ptr::null_mut(), &mut font_family) == Status(0) {
                let _ = GdipDeleteFontFamily(font_family);
            }
            log_debug("GDI+ font cache pre-warmed.");

            // Load the 9-slice shadow image
            let shadow_path = window::wide_string("src\\assets\\drop_shadow.png");
            if GdipLoadImageFromFile(PCWSTR(shadow_path.as_ptr()), addr_of_mut!(SHADOW_IMAGE)) == Status(0) {
                log_debug("Loaded drop_shadow.png successfully");
            } else {
                log_debug("Failed to load drop_shadow.png");
            }
        } else {
            log_debug(&format!("Failed to initialize GDI+: {:?}", result));
        }
    }

    let (screen_w, screen_h) = window::get_screen_size();
    log_debug(&format!("Screen: {}x{}", screen_w, screen_h));

    let mut app_state = AppState::new(screen_w, screen_h);

    let hwnd = match window::create_canvas_window(Some(wndproc)) {
        Ok(h) => {
            log_debug(&format!("Window created: {:?}", h.0));
            h
        }
        Err(e) => {
            log_debug(&format!("Failed to create window: {:?}", e));
            return;
        }
    };
    app_state.canvas_hwnd = hwnd;

    match hotkey::register_hotkey(hwnd) {
        Ok(_) => log_debug("Hotkey Ctrl+Alt+Space registered successfully"),
        Err(e) => {
            log_debug(&format!("Failed to register hotkey: {:?}", e));
            // Try alternative: Ctrl+Alt+Tab
            log_debug("Hotkey registration failed! Another app may have Ctrl+Alt+Space.");
            return;
        }
    }

    APP_STATE.with(|cell| {
        *cell.borrow_mut() = Some(app_state);
    });

    log_debug("Entering message loop...");

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    log_debug("=== Win-Canvas exiting ===");
}
