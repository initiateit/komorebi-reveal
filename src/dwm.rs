//! DWM Thumbnail management — register, update, and unregister live window thumbnails.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail,
    DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY,
    DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};

/// A managed DWM thumbnail.
pub struct Thumbnail {
    pub handle: isize,
    pub source_hwnd: HWND,
    pub source_width: i32,
    pub source_height: i32,
}

impl Thumbnail {
    /// Register a DWM thumbnail from `source` onto `destination` window.
    pub fn register(destination: HWND, source: HWND) -> windows::core::Result<Self> {
        unsafe {
            let handle = DwmRegisterThumbnail(destination, source)?;
            let source_size = DwmQueryThumbnailSourceSize(handle)?;

            Ok(Self {
                handle,
                source_hwnd: source,
                source_width: source_size.cx,
                source_height: source_size.cy,
            })
        }
    }

    /// Update the thumbnail display properties (position/size on the destination window).
    /// The rect is inset to create negative space for rounded corners.
    pub fn update(
        &self,
        dest_rect: RECT,
        opacity: u8,
        client_area_only: bool,
    ) -> windows::core::Result<()> {
        unsafe {
            let mut props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_VISIBLE | DWM_TNP_RECTDESTINATION | DWM_TNP_OPACITY,
                fVisible: true.into(),
                rcDestination: dest_rect,
                opacity,
                ..Default::default()
            };

            if client_area_only {
                props.dwFlags |= DWM_TNP_SOURCECLIENTAREAONLY;
                props.fSourceClientAreaOnly = true.into();
            }

            DwmUpdateThumbnailProperties(self.handle, &props)?;
            Ok(())
        }
    }

    /// Hide this thumbnail (set invisible).
    pub fn hide(&self) -> windows::core::Result<()> {
        unsafe {
            let props = DWM_THUMBNAIL_PROPERTIES {
                dwFlags: DWM_TNP_VISIBLE,
                fVisible: false.into(),
                ..Default::default()
            };
            DwmUpdateThumbnailProperties(self.handle, &props)?;
            Ok(())
        }
    }
}

impl Drop for Thumbnail {
    fn drop(&mut self) {
        unsafe {
            let _ = DwmUnregisterThumbnail(self.handle);
        }
    }
}
