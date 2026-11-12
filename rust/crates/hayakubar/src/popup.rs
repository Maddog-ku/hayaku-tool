//! 面板、提示、滑鼠停留的摘要這幾個「浮在工作列上方」的視窗共用的原生設定
//! （C# 是 WindowStyle=None、Topmost、ShowInTaskbar=False、ShowActivated=False，加上 SetWindowLong 的 NOACTIVATE／TOOLWINDOW）。
//!
//! Slint 的 winit 後端在**建立元件時**就決定視窗屬性；這裡在建立前用 [`create`] 標記要建哪一種，
//! 由 [`attributes_hook`]（掛在 `BackendSelector`）把那一種的屬性設好：無邊框、最上層、不佔工作列，
//! 提示與摘要第一次顯示時不搶焦點。顯示後再用 [`apply_styles`] 補上 WS_EX_NOACTIVATE 等延伸樣式與圓角。

use std::cell::Cell;

use slint::winit_030::winit;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, SystemParametersInfoW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
};

/// 浮動視窗的種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupKind {
    /// 面板：要接收鍵盤（開始任務的輸入框），會取得焦點。
    Panel,
    /// 提示：點按鈕也不搶走目前視窗的焦點（C# MakeNoActivate）。
    Toast,
    /// 滑鼠停留的摘要：不接收滑鼠、不搶焦點（C# NOACTIVATE｜TOOLWINDOW｜TRANSPARENT）。
    Hover,
}

thread_local! {
    static PENDING: Cell<Option<PopupKind>> = const { Cell::new(None) };
}

/// 建立一個浮動視窗元件（`XxxWindow::new()`）：建立期間標記種類，讓 [`attributes_hook`] 套用對應的屬性。
pub fn create<T>(
    kind: PopupKind,
    f: impl FnOnce() -> Result<T, slint::PlatformError>,
) -> Result<T, slint::PlatformError> {
    PENDING.with(|p| p.set(Some(kind)));
    let result = f();
    PENDING.with(|p| p.set(None));
    result
}

/// 掛在 `slint::BackendSelector::with_winit_window_attributes_hook`：只改 [`create`] 標記的視窗，其他（設定、工作台、小視窗）不動。
pub fn attributes_hook(attrs: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    use winit::platform::windows::WindowAttributesExtWindows;
    // 所有視窗（設定、工作台、小視窗）一建立就用目前的深淺色標題列；不然 winit 會照 Windows 的設定
    let theme = if crate::theme_manager::ThemeManager::is_dark() {
        winit::window::Theme::Dark
    } else {
        winit::window::Theme::Light
    };
    let attrs = attrs.with_theme(Some(theme));
    let Some(kind) = PENDING.with(Cell::get) else {
        return attrs;
    };
    attrs
        .with_decorations(false)
        .with_resizable(false)
        .with_window_level(winit::window::WindowLevel::AlwaysOnTop)
        .with_skip_taskbar(true)
        .with_active(kind == PopupKind::Panel)
}

/// 視窗的 HWND（還沒顯示時拿不到）。
pub fn hwnd(window: &slint::Window) -> Option<HWND> {
    hb_ui_common::window_util::hwnd(window)
}

/// 顯示後補上延伸樣式：不佔工作列（TOOLWINDOW）；提示、摘要加 NOACTIVATE（摘要再加 TRANSPARENT）；
/// Windows 11 的圓角；放到最上層。
pub fn apply_styles(window: &slint::Window, kind: PopupKind) {
    let Some(h) = hwnd(window) else { return };
    unsafe {
        let style = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let mut wanted = (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
        if kind != PopupKind::Panel {
            wanted |= WS_EX_NOACTIVATE.0 as isize;
        }
        if kind == PopupKind::Hover {
            wanted |= WS_EX_TRANSPARENT.0 as isize;
        }
        if wanted != style {
            SetWindowLongPtrW(h, GWL_EXSTYLE, wanted);
        }
        let corner = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            h,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const core::ffi::c_void,
            size_of_val(&corner) as u32,
        );
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

/// 直接用 Win32 藏起來（Slint 仍當它是顯示中；之後用 [`show_no_activate`] 再顯示，不會搶焦點）。
pub fn hide_native(window: &slint::Window) {
    if let Some(h) = hwnd(window) {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

/// 不搶焦點地顯示（SW_SHOWNOACTIVATE）。
pub fn show_no_activate(window: &slint::Window) {
    if let Some(h) = hwnd(window) {
        unsafe {
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
        }
    }
}

/// 視窗目前的外框大小（實體像素）；還沒顯示時 None。
pub fn outer_size(window: &slint::Window) -> Option<(i32, i32)> {
    let h = hwnd(window)?;
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.ok()?;
    (r.right > r.left && r.bottom > r.top).then_some((r.right - r.left, r.bottom - r.top))
}

/// 視窗目前的位置與大小（實體像素）。
pub fn outer_rect(window: &slint::Window) -> Option<hb_win::Rect> {
    let h = hwnd(window)?;
    let mut r = RECT::default();
    unsafe { GetWindowRect(h, &mut r) }.ok()?;
    Some(r.into())
}

/// 點所在螢幕的可用範圍（扣掉工作列，實體像素）。
pub fn work_area_at(x: i32, y: i32) -> hb_win::Rect {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(monitor, &mut info).as_bool() { info.rcWork.into() } else { primary_work_area() }
    }
}

/// 主螢幕的可用範圍（C# `SystemParameters.WorkArea`，實體像素）。
pub fn primary_work_area() -> hb_win::Rect {
    let mut r = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut r as *mut RECT as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_ok() { r.into() } else { hb_win::Rect { left: 0, top: 0, right: 1920, bottom: 1040 } }
}

/// 點所在螢幕的縮放比例（96 DPI = 1.0）；視窗還沒建立、Slint 還不知道縮放時用。
pub fn scale_at(x: i32, y: i32) -> f32 {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (0u32, 0u32);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_ok() && dx > 0 {
            dx as f32 / 96.0
        } else {
            1.0
        }
    }
}

/// 擺到 anchor（工作列上工時的位置，實體像素）的正上方（C# PanelWindow／HoverCard 的 Reposition）：
/// 靠左時左邊對齊 anchor 左邊，否則右邊對齊 anchor 右邊；不超出螢幕；跟 anchor 上緣隔 gap。
/// （C# 視窗外圈有 6 的陰影邊、再隔 2，看得到的卡片離工時 8；Rust 版視窗就是卡片，直接隔 8。）
pub fn place_above(
    anchor: hb_win::Rect,
    align_left: bool,
    size: (i32, i32),
    work: hb_win::Rect,
    gap: i32,
) -> (i32, i32) {
    let (w, h) = size;
    let left = if align_left { anchor.left } else { anchor.right - w };
    let x = left.clamp(work.left, work.left.max(work.right - w));
    let y = work.top.max(anchor.top.min(work.bottom) - h - gap);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> hb_win::Rect {
        hb_win::Rect { left, top, right, bottom }
    }

    #[test]
    fn 面板擺在工時正上方_靠右對齊() {
        let work = rect(0, 0, 1920, 1032);
        let anchor = rect(1700, 1032, 1780, 1080);
        assert_eq!((1780 - 432, 1032 - 600 - 8), place_above(anchor, false, (432, 600), work, 8));
    }

    #[test]
    fn 面板靠左對齊_不超出螢幕() {
        let work = rect(0, 0, 1920, 1032);
        // 工時在最左邊：左邊對齊
        assert_eq!((10, 1032 - 300 - 8), place_above(rect(10, 1032, 90, 1080), true, (432, 300), work, 8));
        // 靠右對齊時會超出左邊：夾在螢幕裡
        assert_eq!((0, 1032 - 300 - 8), place_above(rect(10, 1032, 90, 1080), false, (432, 300), work, 8));
        // 靠左對齊時會超出右邊
        assert_eq!((1920 - 432, 400), place_above(rect(1900, 1032, 1910, 1080), true, (432, 624), work, 8));
    }

    #[test]
    fn 太高時貼齊螢幕上緣() {
        let work = rect(0, 0, 1920, 1032);
        assert_eq!(0, place_above(rect(1700, 1032, 1780, 1080), false, (432, 2000), work, 8).1);
    }

    #[test]
    fn 工作列在上方時用工作區下緣() {
        // anchor 在工作區下面以外的位置（工作列在上方時 anchor.top 比 work.bottom 小或大都以較小者為準）
        let work = rect(0, 48, 1920, 1080);
        assert_eq!(1080 - 200 - 8, place_above(rect(1700, 2000, 1780, 2048), false, (432, 200), work, 8).1);
    }
}
