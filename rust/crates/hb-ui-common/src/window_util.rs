//! 小視窗共用：保留視窗（Slint 的元件被丟掉視窗就關了）、置中、擺到工作列上方、不佔工作列（WS_EX_TOOLWINDOW）、
//! 標題列深淺色（C# `ThemeManager.ApplyTitleBar`）、拉到前面。

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetCursorPos, GetWindowLongPtrW, SW_HIDE, SW_SHOW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow,
    WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
use windows::core::BOOL;

struct Entry {
    _value: Box<dyn Any>,
    tag: Option<String>,
    activate: Rc<dyn Fn()>,
}

thread_local! {
    static OPEN: RefCell<HashMap<u64, Entry>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u64> = const { Cell::new(1) };
}

/// 放掉時執行的動作（例如視窗關了，把結果交給等著的 future）。
pub(crate) struct OnDrop(Option<Box<dyn FnOnce()>>);

impl OnDrop {
    pub(crate) fn new(f: impl FnOnce() + 'static) -> Self {
        Self(Some(Box::new(f)))
    }
}

impl Drop for OnDrop {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}

/// 留住一個視窗元件（和其他要一起留著的東西）直到 [`release`]；tag 用來找「同一個只開一個」的視窗。回傳編號。
pub(crate) fn keep<T: slint::ComponentHandle + 'static>(ui: &T, tag: Option<String>, extra: impl Any) -> u64 {
    let id = NEXT.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    let weak = ui.as_weak();
    let activate: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.show();
            activate(ui.window());
        }
    });
    let value: Box<dyn Any> = Box::new((ui.clone_strong(), extra));
    OPEN.with(|o| o.borrow_mut().insert(id, Entry { _value: value, tag, activate }));
    id
}

/// 有同一個 tag 的視窗開著就拉到前面，回 true（C# `Windows.OfType<…>().FirstOrDefault(w => w.Tag == tag)?.Activate()`）。
pub(crate) fn activate_tag(tag: &str) -> bool {
    let found = OPEN.with(|o| o.borrow().values().find(|e| e.tag.as_deref() == Some(tag)).map(|e| e.activate.clone()));
    match found {
        Some(f) => {
            f();
            true
        }
        None => false,
    }
}

/// 放掉 [`keep`] 留住的東西；延到下一輪事件迴圈才丟，避免在視窗自己的回呼裡把自己丟掉。
pub(crate) fn release(id: u64) {
    slint::Timer::single_shot(Duration::ZERO, move || {
        let value = OPEN.with(|o| o.borrow_mut().remove(&id));
        drop(value);
    });
}

/// 視窗的 HWND（還沒顯示時拿不到）。
pub fn hwnd(window: &slint::Window) -> Option<HWND> {
    let handle = window.window_handle();
    let raw = handle.window_handle().ok()?.as_raw();
    match raw {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// 不佔工作列（C# `ShowInTaskbar = false`）：加上 WS_EX_TOOLWINDOW。Windows 規定要先藏起來、改樣式、再顯示。
pub fn set_tool_window(window: &slint::Window) {
    let Some(h) = hwnd(window) else { return };
    unsafe {
        let style = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let wanted = (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
        if wanted == style {
            return;
        }
        let _ = ShowWindow(h, SW_HIDE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, wanted);
        let _ = ShowWindow(h, SW_SHOW);
    }
}

/// 標題列跟著深淺色（DWMWA_USE_IMMERSIVE_DARK_MODE）。
pub fn set_dark_title_bar(window: &slint::Window, dark: bool) {
    let Some(h) = hwnd(window) else { return };
    let value = BOOL::from(dark);
    unsafe {
        let _ = DwmSetWindowAttribute(
            h,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &value as *const BOOL as *const core::ffi::c_void,
            size_of::<BOOL>() as u32,
        );
    }
}

/// 拉到前面並取得焦點（C# `Activate()`）。
pub fn activate(window: &slint::Window) {
    if let Some(h) = hwnd(window) {
        unsafe {
            let _ = SetForegroundWindow(h);
        }
    }
}

/// 螢幕可用範圍（扣掉工作列，實體像素）：視窗所在的螢幕；拿不到視窗時用滑鼠所在的螢幕。
pub fn work_area(window: Option<&slint::Window>) -> hb_win::Rect {
    unsafe {
        let monitor = match window.and_then(hwnd) {
            Some(h) => MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST),
            None => {
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)
            }
        };
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            info.rcWork.into()
        } else {
            hb_win::Rect { left: 0, top: 0, right: 1920, bottom: 1040 }
        }
    }
}

/// 系統 DPI 的縮放比例（視窗還沒顯示、Slint 還不知道縮放時用）。
fn system_scale() -> f32 {
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() };
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

/// 視窗大小（實體像素）；還沒顯示（Slint 還沒算出大小）時用 estimate（邏輯像素）乘上系統縮放。
/// 顯示後用 HWND 的實際外框大小（Slint 的 `size()` 要等第一次調整大小的事件處理完才會更新）；
/// 還沒顯示時用 estimate（邏輯像素，不含標題列）乘上系統縮放。
fn size_or(window: &slint::Window, estimate: (f32, f32)) -> slint::PhysicalSize {
    if let Some(h) = hwnd(window) {
        let mut r = windows::Win32::Foundation::RECT::default();
        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(h, &mut r) }.is_ok()
            && r.right > r.left
            && r.bottom > r.top
        {
            return slint::PhysicalSize::new((r.right - r.left) as u32, (r.bottom - r.top) as u32);
        }
    }
    let scale = system_scale();
    slint::PhysicalSize::new((estimate.0 * scale) as u32, ((estimate.1 + 32.0) * scale) as u32)
}

/// 置中在滑鼠所在的螢幕（C# `CenterScreen`）。
pub fn center_on_screen(window: &slint::Window) {
    center_on_screen_est(window, (440.0, 300.0));
}

fn center_on_screen_est(window: &slint::Window, estimate: (f32, f32)) {
    let area = work_area(None);
    let size = size_or(window, estimate);
    let x = area.left + (area.width() - size.width as i32) / 2;
    let y = area.top + (area.height() - size.height as i32) / 2;
    window.set_position(slint::PhysicalPosition::new(x.max(area.left), y.max(area.top)));
}

/// 置中在另一個視窗上（C# `CenterOwner`）；不超出螢幕。
pub fn center_on_owner(window: &slint::Window, owner: &slint::Window) {
    center_on_owner_est(window, owner, (440.0, 300.0));
}

fn center_on_owner_est(window: &slint::Window, owner: &slint::Window, estimate: (f32, f32)) {
    let size = size_or(window, estimate);
    let pos = owner.position();
    let osize = owner.size();
    let area = work_area(Some(owner));
    let x = pos.x + (osize.width as i32 - size.width as i32) / 2;
    let y = pos.y + (osize.height as i32 - size.height as i32) / 2;
    let x = x.clamp(area.left, (area.right - size.width as i32).max(area.left));
    let y = y.clamp(area.top, (area.bottom - size.height as i32).max(area.top));
    window.set_position(slint::PhysicalPosition::new(x, y));
}

/// 擺在 anchor（工作列上的工時，實體像素）的正上方，不超出螢幕（C# `QuickCreateWindow.PlaceNear`）：
/// alignLeft 時左邊對齊 anchor 左邊，否則右邊對齊 anchor 右邊；離螢幕邊 8（DIP）。
pub fn place_near(window: &slint::Window, anchor: hb_win::Rect, align_left: bool) {
    place_near_est(window, anchor, align_left, (780.0, 600.0));
}

pub(crate) fn place_near_est(window: &slint::Window, anchor: hb_win::Rect, align_left: bool, estimate: (f32, f32)) {
    let area = work_area(None);
    let size = size_or(window, estimate);
    let scale = if hwnd(window).is_some() { window.scale_factor() } else { system_scale() };
    let margin = (8.0 * scale).round() as i32;
    let (w, h) = (size.width as i32, size.height as i32);
    let left = if align_left { anchor.left } else { anchor.right - w };
    let x = left.clamp(area.left + margin, (area.right - w - margin).max(area.left + margin));
    let y = (area.top + margin).max(area.bottom - h - margin);
    window.set_position(slint::PhysicalPosition::new(x, y));
}

/// 小視窗怎麼放。
#[derive(Clone, Copy)]
pub enum Placement<'a> {
    /// 滑鼠所在螢幕的正中間（C# CenterScreen）
    Screen,
    /// 另一個視窗的正中間（C# CenterOwner）；None 時同 Screen
    Owner(Option<&'a slint::Window>),
    /// 不動（呼叫端自己放）
    Manual,
}

/// 顯示一個小視窗：套用深淺色（之後也跟著換）、顯示、不佔工作列、標題列深淺色、擺好位置。
/// 視窗被關掉（使用者按 ×）時放掉 [`keep`] 的 id。
/// estimate：視窗大約的大小（邏輯像素）；顯示前先用它擺好位置（一出現就在螢幕內、不會跳），顯示後再用實際大小擺一次。
pub(crate) fn present<T: slint::ComponentHandle + 'static>(
    ui: &T,
    id: u64,
    placement: Placement<'_>,
    tool_window: bool,
    estimate: (f32, f32),
    apply_theme: fn(&T, bool, bool),
) {
    let weak = ui.as_weak();
    crate::theme::track(move |dark, reduce| match weak.upgrade() {
        Some(ui) => {
            apply_theme(&ui, dark, reduce);
            set_dark_title_bar(ui.window(), dark);
            true
        }
        None => false,
    });
    ui.window().on_close_requested(move || {
        release(id);
        slint::CloseRequestResponse::HideWindow
    });
    let place = |window: &slint::Window| match placement {
        Placement::Screen | Placement::Owner(None) => center_on_screen_est(window, estimate),
        Placement::Owner(Some(owner)) => center_on_owner_est(window, owner, estimate),
        Placement::Manual => {}
    };
    place(ui.window());
    if let Err(e) = ui.show() {
        log::error!("顯示視窗失敗: {e}");
        release(id);
        return;
    }
    if tool_window {
        set_tool_window(ui.window());
    }
    set_dark_title_bar(ui.window(), crate::theme::current().0);
    place(ui.window());
    ui.window().request_redraw();
}

/// 從程式關掉小視窗（C# `Close()`）：藏起來、放掉。
pub(crate) fn close<T: slint::ComponentHandle>(ui: &T, id: u64) {
    let _ = ui.hide();
    release(id);
}
