//! `--shot` 的截圖（C# 用 RenderTargetBitmap 拍 WPF 視窗）：用 PrintWindow 拍這個程式自己的視窗，存成 PNG。
//! 螢幕鎖定時直接截螢幕只會拍到桌布，所以視窗一律用 PrintWindow（PW_RENDERFULLCONTENT）。
//! 註：Slint 用 OpenGL 畫的視窗 PrintWindow 可能拍成白的；要截圖時用 `SLINT_BACKEND=winit-software` 啟動。

use std::path::Path;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId};

/// 要拍哪裡。
#[derive(Debug, Clone, Copy)]
pub enum Target {
    /// 這個視窗
    Window(HWND),
    /// 目前在最前面的視窗（剛開的小視窗；要是這個程式的才拍）
    Foreground,
    /// 螢幕上的一塊（工作列上的工時）
    Screen(hb_win::Rect),
}

const PW_RENDERFULLCONTENT: u32 = 2;

pub fn capture(target: &Target, path: &Path) -> anyhow::Result<()> {
    let (width, height, pixels) = match *target {
        Target::Window(h) => window_pixels(h)?,
        Target::Foreground => {
            let h = unsafe { GetForegroundWindow() };
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
            anyhow::ensure!(pid == std::process::id(), "最前面的視窗不是 HayakuBar 的");
            window_pixels(h)?
        }
        Target::Screen(r) => screen_pixels(r)?,
    };
    let image = image::RgbaImage::from_raw(width, height, pixels).ok_or_else(|| anyhow::anyhow!("圖片大小不對"))?;
    image.save(path)?;
    Ok(())
}

/// BGRA（DIB，上下顛倒前已用負高度處理）→ RGBA，alpha 設成不透明。
fn to_rgba(mut bgra: Vec<u8>) -> Vec<u8> {
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
        px[3] = 255;
    }
    bgra
}

fn window_pixels(h: HWND) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }?;
    let (w, ht) = (r.right - r.left, r.bottom - r.top);
    anyhow::ensure!(w > 0 && ht > 0, "視窗大小是 0");
    with_dib(w, ht, |dc| unsafe { PrintWindow(h, dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool() })
}

fn screen_pixels(r: hb_win::Rect) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let (w, h) = (r.width(), r.height());
    anyhow::ensure!(w > 0 && h > 0, "範圍是 0");
    with_dib(w, h, |dc| unsafe {
        let screen = GetDC(None);
        let ok = BitBlt(dc, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY | CAPTUREBLT).is_ok();
        ReleaseDC(None, screen);
        ok
    })
}

fn with_dib(
    w: i32,
    h: i32,
    draw: impl FnOnce(windows::Win32::Graphics::Gdi::HDC) -> bool,
) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        ReleaseDC(None, screen);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, HGDIOBJ(bitmap.0));
        let ok = draw(dc);
        let len = (w * h * 4) as usize;
        let pixels = if ok && !bits.is_null() {
            std::slice::from_raw_parts(bits as *const u8, len).to_vec()
        } else {
            Vec::new()
        };
        SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
        anyhow::ensure!(ok && !pixels.is_empty(), "拍不到畫面");
        Ok((w as u32, h as u32, to_rgba(pixels)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_轉_rgba() {
        assert_eq!(vec![3, 2, 1, 255, 30, 20, 10, 255], to_rgba(vec![1, 2, 3, 0, 10, 20, 30, 7]));
    }
}
