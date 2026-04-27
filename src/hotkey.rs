//! Global hotkey registration and management.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_CONTROL, MOD_ALT, MOD_NOREPEAT,
};

/// Hotkey identifier constant.
pub const HOTKEY_TOGGLE_CANVAS: i32 = 1;

/// Register the global hotkey (Ctrl+Alt+Space by default).
pub fn register_hotkey(hwnd: HWND) -> windows::core::Result<()> {
    unsafe {
        // Ctrl + Alt + Space
        let modifiers = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_NOREPEAT.0);
        let vk_space = 0x20u32; // VK_SPACE
        RegisterHotKey(hwnd, HOTKEY_TOGGLE_CANVAS, modifiers, vk_space)?;
        Ok(())
    }
}

/// Unregister the global hotkey.
pub fn unregister_hotkey(hwnd: HWND) {
    unsafe {
        let _ = UnregisterHotKey(hwnd, HOTKEY_TOGGLE_CANVAS);
    }
}
