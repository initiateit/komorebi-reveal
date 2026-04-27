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

/// Capture the entire screen to an HBITMAP, and apply a GPU-accelerated Gaussian Blur.
pub fn capture_screen(screen_w: i32, screen_h: i32) -> HBITMAP {
    use std::ffi::c_void;
    use windows::core::{Interface};
    use windows::Win32::Graphics::Direct2D::Common::*;
    use windows::Win32::Graphics::Direct2D::*;
    use windows::Win32::Graphics::Direct3D::*;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::*;
    use windows::Win32::Graphics::Dxgi::*;

    unsafe {
        // 1. Try to load the desktop wallpaper via GDI+
        let mut path = vec![0u16; 260];
        let res = windows::Win32::UI::WindowsAndMessaging::SystemParametersInfoW(
            windows::Win32::UI::WindowsAndMessaging::SPI_GETDESKWALLPAPER,
            260,
            Some(path.as_mut_ptr() as *mut _),
            windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
        
        let hdc_screen = GetDC(HWND::default());
        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbm = CreateCompatibleBitmap(hdc_screen, screen_w, screen_h);
        let old = SelectObject(hdc_mem, hbm);
        
        let mut loaded_wallpaper = false;
        if res.is_ok() {
            use windows::Win32::Graphics::GdiPlus::*;
            use windows::core::PCWSTR;
            let mut image: *mut GpImage = std::ptr::null_mut();
            if GdipLoadImageFromFile(PCWSTR(path.as_ptr()), &mut image as *mut _ as *mut _) == Status(0) {
                let mut graphics: *mut GpGraphics = std::ptr::null_mut();
                if GdipCreateFromHDC(hdc_mem, &mut graphics as *mut _ as *mut _) == Status(0) {
                    GdipDrawImageRectI(graphics, image, 0, 0, screen_w, screen_h);
                    GdipDeleteGraphics(graphics);
                    loaded_wallpaper = true;
                }
                GdipDisposeImage(image);
            }
        }
        
        if !loaded_wallpaper {
            // Fallback to capturing screen
            let _ = BitBlt(hdc_mem, 0, 0, screen_w, screen_h, hdc_screen, 0, 0, SRCCOPY);
        }
        SelectObject(hdc_mem, old);

        // 2. Read pixels
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: screen_w,
                biHeight: -screen_h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut pixels = vec![0u8; (screen_w * screen_h * 4) as usize];
        GetDIBits(
            hdc_mem,
            hbm,
            0,
            screen_h as u32,
            Some(pixels.as_mut_ptr() as *mut c_void),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(hdc_mem);
        let _ = DeleteObject(hbm);

        // 3. Initialize D3D11 & D2D
        let mut d3d_device: Option<ID3D11Device> = None;
        let mut d3d_context: Option<ID3D11DeviceContext> = None;
        let hr = D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            None,
            Some(&mut d3d_context),
        );

        // If D3D11 fails, return a blank black bitmap as fallback
        if hr.is_err() {
            let hbm_fallback = CreateCompatibleBitmap(hdc_screen, screen_w, screen_h);
            ReleaseDC(HWND::default(), hdc_screen);
            return hbm_fallback;
        }
        
        let d3d_device = d3d_device.unwrap();
        let d3d_context = d3d_context.unwrap();
        let dxgi_device: IDXGIDevice = d3d_device.cast().unwrap();
        
        let options = D2D1_FACTORY_OPTIONS::default();
        let factory: ID2D1Factory1 = D2D1CreateFactory(
            D2D1_FACTORY_TYPE_SINGLE_THREADED,
            Some(&options),
        ).unwrap();
        
        let d2d_device = factory.CreateDevice(&dxgi_device).unwrap();
        let d2d_context = d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE).unwrap();

        // 4. Create source D2D bitmap
        let size = D2D_SIZE_U { width: screen_w as u32, height: screen_h as u32 };
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            ..Default::default()
        };
        let src_bitmap = d2d_context.CreateBitmap(
            size,
            Some(pixels.as_ptr() as *const c_void),
            (screen_w * 4) as u32,
            &props as *const _,
        ).unwrap();

        // 5. Create render target texture (D3D11)
        let tex_desc = D3D11_TEXTURE2D_DESC {
            Width: screen_w as u32,
            Height: screen_h as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        
        let mut rt_tex: Option<ID3D11Texture2D> = None;
        d3d_device.CreateTexture2D(&tex_desc, None, Some(&mut rt_tex)).unwrap();
        let rt_tex = rt_tex.unwrap();
        
        let dxgi_surface: IDXGISurface = rt_tex.cast().unwrap();
        
        let target_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET,
            ..Default::default()
        };
        
        let target_bitmap = d2d_context.CreateBitmapFromDxgiSurface(&dxgi_surface, Some(&target_props as *const _)).unwrap();
        d2d_context.SetTarget(&target_bitmap);

        // 6. Apply blur
        let blur_effect: ID2D1Effect = d2d_context.CreateEffect(&CLSID_D2D1GaussianBlur).unwrap();
        blur_effect.SetValue(
            D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION.0 as u32,
            D2D1_PROPERTY_TYPE_FLOAT,
            &120.0f32.to_ne_bytes(),
        ).unwrap();
        blur_effect.SetValue(
            D2D1_GAUSSIANBLUR_PROP_BORDER_MODE.0 as u32,
            D2D1_PROPERTY_TYPE_ENUM,
            &D2D1_BORDER_MODE_HARD.0.to_ne_bytes(),
        ).unwrap();
        blur_effect.SetInput(0, &src_bitmap, None);

        d2d_context.BeginDraw();
        
        // Get output image from effect to draw
        let image = blur_effect.GetOutput().unwrap();
        d2d_context.DrawImage(
            &image,
            None,
            None,
            D2D1_INTERPOLATION_MODE_LINEAR,
            D2D1_COMPOSITE_MODE_SOURCE_OVER,
        );
        
        d2d_context.EndDraw(None, None).unwrap();

        // 7. Read back using staging texture
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: screen_w as u32,
            Height: screen_h as u32,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            ..Default::default()
        };
        let mut staging_tex: Option<ID3D11Texture2D> = None;
        d3d_device.CreateTexture2D(&staging_desc, None, Some(&mut staging_tex)).unwrap();
        let staging_tex = staging_tex.unwrap();
        
        let rt_resource: ID3D11Resource = rt_tex.cast().unwrap();
        let staging_resource: ID3D11Resource = staging_tex.cast().unwrap();
        d3d_context.CopyResource(&staging_resource, &rt_resource);
        
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        d3d_context.Map(&staging_resource, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).unwrap();
        
        // Create new HBITMAP from mapped data
        let final_hbm = CreateCompatibleBitmap(hdc_screen, screen_w, screen_h);
        SetDIBits(
            hdc_screen,
            final_hbm,
            0,
            screen_h as u32,
            mapped.pData,
            &info,
            DIB_RGB_COLORS,
        );
        
        d3d_context.Unmap(&staging_resource, 0);
        ReleaseDC(HWND::default(), hdc_screen);

        final_hbm
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
