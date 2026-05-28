//! Native window capture via Win32 PrintWindow + GDI.
//!
//! Returns a PNG-encoded byte vector of the target window's contents.
//! Works even when the window is partially occluded (PW_RENDERFULLCONTENT).
//!
//! Safety: all GDI calls are wrapped in `catch_unwind` so an invalid HWND
//! (window destroyed mid-capture) returns `Err` instead of crashing the
//! process through undefined behavior at the WebView2 COM boundary.

#[cfg(windows)]
pub fn capture_window_png(hwnd: isize) -> Result<Vec<u8>, String> {
    std::panic::catch_unwind(|| capture_inner(hwnd))
        .unwrap_or_else(|_| Err("capture panicked (HWND likely invalid)".into()))
}

#[cfg(windows)]
fn capture_inner(hwnd: isize) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::Storage::Xps::PrintWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, IsWindow};

    const PW_RENDERFULLCONTENT: u32 = 0x00000002;

    unsafe {
        let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;

        if IsWindow(hwnd) == 0 {
            return Err("HWND is no longer valid".into());
        }

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        if GetClientRect(hwnd, &mut rect) == 0 {
            return Err("GetClientRect failed".into());
        }

        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return Err(format!("invalid window dimensions: {width}x{height}"));
        }

        let hdc_window = GetDC(hwnd);
        if hdc_window.is_null() {
            return Err("GetDC failed".into());
        }

        let hdc_mem = CreateCompatibleDC(hdc_window);
        if hdc_mem.is_null() {
            ReleaseDC(hwnd, hdc_window);
            return Err("CreateCompatibleDC failed".into());
        }

        let hbm = CreateCompatibleBitmap(hdc_window, width, height);
        if hbm.is_null() {
            DeleteDC(hdc_mem);
            ReleaseDC(hwnd, hdc_window);
            return Err("CreateCompatibleBitmap failed".into());
        }

        let old_bm = SelectObject(hdc_mem, hbm);

        // PrintWindow with PW_RENDERFULLCONTENT asks the window to paint
        // itself into our DC, which correctly captures DirectComposition
        // content (WebView2 uses GPU compositing — BitBlt reads a stale
        // GDI surface and misses overlay/popup layers).
        let ok = PrintWindow(hwnd, hdc_mem, PW_RENDERFULLCONTENT);
        if ok == 0 {
            SelectObject(hdc_mem, old_bm);
            DeleteObject(hbm);
            DeleteDC(hdc_mem);
            ReleaseDC(hwnd, hdc_window);
            return Err("PrintWindow failed".into());
        }

        // Read bitmap bits as BGRA
        let w = width as u32;
        let h = height as u32;
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [std::mem::zeroed()],
        };

        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let rows = GetDIBits(
            hdc_mem,
            hbm,
            0,
            h,
            pixels.as_mut_ptr().cast(),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(hdc_mem, old_bm);
        DeleteObject(hbm);
        DeleteDC(hdc_mem);
        ReleaseDC(hwnd, hdc_window);

        if rows == 0 {
            return Err("GetDIBits failed".into());
        }

        // Convert BGRA → RGB for PNG encoding
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for pixel in pixels.chunks_exact(4) {
            rgb.push(pixel[2]); // R
            rgb.push(pixel[1]); // G
            rgb.push(pixel[0]); // B
        }

        encode_png(&rgb, w, h)
    }
}

#[cfg(windows)]
fn encode_png(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgb).map_err(|e| e.to_string())?;
    }
    Ok(buf)
}
