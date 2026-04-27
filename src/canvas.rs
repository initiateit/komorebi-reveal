//! Canvas state management — pan, zoom, layout, and coordinate transforms.

use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::HICON;

use crate::state::SavedCanvasState;

/// Represents a window's position and size on the canvas (in canvas-space coordinates).
#[derive(Debug, Clone)]
pub struct CanvasWindow {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub thumb_index: usize,
    pub title: String,
    pub icon: HICON,
    pub dragging: bool,
}

/// Source window info for layout computation.
pub struct SourceInfo {
    pub thumb_index: usize,
    pub width: i32,
    pub height: i32,
    pub title: String,
    pub icon: HICON,
}

/// The canvas state.
pub struct Canvas {
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
    pub windows: Vec<CanvasWindow>,
    pub screen_w: i32,
    pub screen_h: i32,
    pub drag_target: Option<usize>,
    pub drag_start_x: f64,
    pub drag_start_y: f64,
    pub drag_origin_x: f64,
    pub drag_origin_y: f64,
    pub panning: bool,
    pub pan_start_x: f64,
    pub pan_start_y: f64,
    pub pan_origin_x: f64,
    pub pan_origin_y: f64,
    pub active_window: Option<usize>, // Currently active/selected window
    // Carousel scrolling state
    target_pan_x: f64,
    scroll_active: bool,
    scroll_start_pan: f64,
    scroll_target: f64,
    scroll_progress: f64,
}

impl Canvas {
    pub fn new(screen_w: i32, screen_h: i32) -> Self {
        Self {
            pan_x: 0.0,
            pan_y: 0.0,
            zoom: 1.0,
            windows: Vec::new(),
            screen_w,
            screen_h,
            drag_target: None,
            drag_start_x: 0.0,
            drag_start_y: 0.0,
            drag_origin_x: 0.0,
            drag_origin_y: 0.0,
            panning: false,
            pan_start_x: 0.0,
            pan_start_y: 0.0,
            pan_origin_x: 0.0,
            pan_origin_y: 0.0,
            active_window: None,
            target_pan_x: 0.0,
            scroll_active: false,
            scroll_start_pan: 0.0,
            scroll_target: 0.0,
            scroll_progress: 0.0,
        }
    }

    /// Layout windows in a single horizontal row.
    pub fn layout_grid(&mut self, sources: &[SourceInfo], saved: Option<&SavedCanvasState>) {
        self.windows.clear();

        let count = sources.len();
        if count == 0 {
            return;
        }

        // Single horizontal row (like Alt+Tab)
        let cols = count;
        let _rows = 1;
        let thumb_w = 400.0;
        let padding = 120.0; // Increased padding for backing cards
        let grid_w = cols as f64 * (thumb_w + padding) - padding;
        let start_x = -(grid_w / 2.0);
        let start_y = 0.0; // Centered vertically

        for (i, src) in sources.iter().enumerate() {
            let col = i;
            let aspect = if src.height > 0 {
                src.width as f64 / src.height as f64
            } else {
                16.0 / 9.0
            };
            let w = thumb_w;
            let h = w / aspect;
            let x = start_x + col as f64 * (thumb_w + padding) + w / 2.0;
            let y = start_y; // All windows on same vertical line

            self.windows.push(CanvasWindow {
                x,
                y,
                w,
                h,
                thumb_index: src.thumb_index,
                title: src.title.clone(),
                icon: src.icon,
                dragging: false,
            });
        }

        // Apply saved zoom state, always center the canvas
        if let Some(saved) = saved {
            self.zoom = saved.zoom;
            self.pan_x = self.screen_w as f64 / 2.0;
            self.pan_y = self.screen_h as f64 / 2.0;
        } else {
            self.pan_x = self.screen_w as f64 / 2.0;
            self.pan_y = self.screen_h as f64 / 2.0;
            self.zoom = 1.0; // Start at 100% zoom for better visibility
        }

        self.target_pan_x = self.pan_x;

        // Set the middle window as active, or keep previously active if valid
        if self.active_window.is_none() || self.active_window.unwrap() >= self.windows.len() {
            self.active_window = if self.windows.is_empty() {
                None
            } else {
                Some(self.windows.len() / 2) // Middle window
            };
        }
    }

    /// Navigate to the next window (cycling)
    pub fn next_window(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let current = self.active_window.unwrap_or(0);
        self.active_window = Some((current + 1) % self.windows.len());
        self.scroll_to_active_window();
    }

    /// Navigate to the previous window (cycling)
    pub fn prev_window(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let current = self.active_window.unwrap_or(0);
        self.active_window = Some(if current == 0 {
            self.windows.len() - 1
        } else {
            current - 1
        });
        self.scroll_to_active_window();
    }

    /// Get the currently active window index
    pub fn get_active_window(&self) -> Option<usize> {
        self.active_window
    }

    /// Set the active window by index and scroll to it
    pub fn set_active_window(&mut self, index: usize) {
        if index < self.windows.len() {
            self.active_window = Some(index);
            self.scroll_to_active_window();
        }
    }

    /// Start smooth scroll animation to center the active window
    /// Only scrolls if the total grid width exceeds screen width
    pub fn scroll_to_active_window(&mut self) {
        if self.windows.is_empty() {
            return;
        }

        // Calculate total grid width in screen space
        // Grid layout uses: thumb_w=400, padding=80
        let thumb_w = 400.0;
        let padding = 80.0;
        let cols = self.windows.len();
        let grid_w = (cols as f64 * (thumb_w + padding) - padding) * self.zoom;

        // Only scroll if grid is wider than screen
        if grid_w <= self.screen_w as f64 {
            return;
        }

        if let Some(idx) = self.active_window {
            if idx < self.windows.len() {
                let window = &self.windows[idx];
                // Calculate target pan_x to center this window on screen
                // We want: window.x * zoom + pan_x = screen_center
                // So: pan_x = screen_center - window.x * zoom
                let screen_center = self.screen_w as f64 / 2.0;
                self.target_pan_x = screen_center - window.x * self.zoom;

                // Start scroll animation
                self.scroll_active = true;
                self.scroll_start_pan = self.pan_x;
                self.scroll_target = self.target_pan_x;
                self.scroll_progress = 0.0;
            }
        }
    }

    /// Update scroll animation, returns true if animation is still active
    pub fn update_scroll_animation(&mut self) -> bool {
        if !self.scroll_active {
            return false;
        }

        // Animation speed: 0.15 per frame (smooth easing)
        const SCROLL_SPEED: f64 = 0.15;
        self.scroll_progress += SCROLL_SPEED;

        if self.scroll_progress >= 1.0 {
            // Animation complete
            self.pan_x = self.scroll_target;
            self.scroll_active = false;
            return false;
        }

        // Ease-out cubic interpolation
        let t = self.scroll_progress;
        let eased = 1.0 - (1.0 - t).powi(3);
        self.pan_x = self.scroll_start_pan + (self.scroll_target - self.scroll_start_pan) * eased;
        true
    }

    /// Export current state for saving (only zoom).
    pub fn to_saved_state(&self) -> SavedCanvasState {
        SavedCanvasState {
            zoom: self.zoom,
        }
    }

    /// Convert canvas-space to screen-space RECT, with an optional scale factor.
    pub fn canvas_to_screen_rect(&self, cw: &CanvasWindow, scale: f64) -> RECT {
        let half_w = cw.w / 2.0 * scale;
        let half_h = cw.h / 2.0 * scale;
        let cx = cw.x * self.zoom + self.pan_x;
        let cy = cw.y * self.zoom + self.pan_y;

        RECT {
            left: (cx - half_w * self.zoom) as i32,
            top: (cy - half_h * self.zoom) as i32,
            right: (cx + half_w * self.zoom) as i32,
            bottom: (cy + half_h * self.zoom) as i32,
        }
    }

    pub fn screen_to_canvas(&self, screen_x: f64, screen_y: f64) -> (f64, f64) {
        let cx = (screen_x - self.pan_x) / self.zoom;
        let cy = (screen_y - self.pan_y) / self.zoom;
        (cx, cy)
    }

    pub fn zoom_at(&mut self, screen_x: f64, screen_y: f64, delta: f64) {
        let old_zoom = self.zoom;
        let zoom_factor = if delta > 0.0 { 1.15 } else { 1.0 / 1.15 };
        self.zoom = (self.zoom * zoom_factor).clamp(0.05, 10.0);
        let ratio = self.zoom / old_zoom;
        self.pan_x = screen_x - ratio * (screen_x - self.pan_x);
        self.pan_y = screen_y - ratio * (screen_y - self.pan_y);
    }

    pub fn hit_test(&self, screen_x: f64, screen_y: f64) -> Option<usize> {
        let (cx, cy) = self.screen_to_canvas(screen_x, screen_y);
        for (i, w) in self.windows.iter().enumerate().rev() {
            let half_w = w.w / 2.0;
            let half_h = w.h / 2.0;
            if cx >= w.x - half_w && cx <= w.x + half_w && cy >= w.y - half_h && cy <= w.y + half_h
            {
                return Some(i);
            }
        }
        None
    }

    pub fn update_drag(&mut self, screen_x: f64, screen_y: f64) {
        if let Some(idx) = self.drag_target {
            let dx = (screen_x - self.drag_start_x) / self.zoom;
            let dy = (screen_y - self.drag_start_y) / self.zoom;
            self.windows[idx].x = self.drag_origin_x + dx;
            self.windows[idx].y = self.drag_origin_y + dy;
        }
    }

    pub fn end_drag(&mut self) {
        if let Some(idx) = self.drag_target {
            self.windows[idx].dragging = false;
        }
        self.drag_target = None;
    }

    pub fn start_pan(&mut self, screen_x: f64, screen_y: f64) {
        self.panning = true;
        self.pan_start_x = screen_x;
        self.pan_start_y = screen_y;
        self.pan_origin_x = self.pan_x;
        self.pan_origin_y = self.pan_y;
    }

    pub fn update_pan(&mut self, screen_x: f64, screen_y: f64) {
        if self.panning {
            self.pan_x = self.pan_origin_x + (screen_x - self.pan_start_x);
            self.pan_y = self.pan_origin_y + (screen_y - self.pan_start_y);
        }
    }

    pub fn end_pan(&mut self) {
        self.panning = false;
    }
}
