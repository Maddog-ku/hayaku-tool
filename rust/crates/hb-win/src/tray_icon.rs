//! 對應 C#：`src/HayakuBar/UI/TrayIcon.cs`
//!
//! 系統匣圖示：選單（登入、工作列位置、設定、開機啟動、結束）與通知。工作列上的工時按右鍵也是這個選單。
//! C# 用 WinForms NotifyIcon＋ContextMenuStrip；這裡用 Shell_NotifyIcon＋原生選單（TrackPopupMenu），
//! 選單每次打開時重建（等同 C# 的 Opening 事件更新登入文字與勾選）。
//!
//! 跟 C# 不同：「開機時自動啟動」C# 直接呼叫 `StartupRegistration`（在 hb-backend），hb-win 不依賴其他 crate，
//! 改由 `TrayActions::is_startup_enabled`／`set_startup` 兩個回呼提供（失敗寫紀錄由呼叫端處理）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HGDIOBJ,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    GetSystemMetrics, HICON, HMENU, ICONINFO, LR_DEFAULTCOLOR, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, PostMessageW, RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
    TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP,
};
use windows::core::{PCWSTR, w};

use crate::native::Point;
use crate::taskbar_widget::{TaskbarAnchor, WidgetLayout};
use crate::window::{Handler, OwnedWindow, wide};

static APP_ICO: &[u8] = include_bytes!("../../hayakubar/assets/app.ico");

const TRAY_MSG: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;

/// 選單項目的回呼（對應 C# `record TrayActions`）。
pub struct TrayActions {
    pub toggle_panel: Box<dyn Fn()>,
    pub refresh: Box<dyn Fn()>,
    pub settings: Box<dyn Fn()>,
    pub login_or_logout: Box<dyn Fn()>,
    pub is_logged_in: Box<dyn Fn() -> bool>,
    pub get_anchor: Box<dyn Fn() -> TaskbarAnchor>,
    pub set_anchor: Box<dyn Fn(TaskbarAnchor)>,
    pub reset_offset: Box<dyn Fn()>,
    pub get_layout: Box<dyn Fn() -> WidgetLayout>,
    pub set_layout: Box<dyn Fn(WidgetLayout)>,
    pub time_entries: Box<dyn Fn()>,
    pub workspace: Box<dyn Fn()>,
    pub open_web: Box<dyn Fn()>,
    pub open_log: Box<dyn Fn()>,
    pub exit: Box<dyn Fn()>,
    /// C# `StartupRegistration.IsEnabled()`。
    pub is_startup_enabled: Box<dyn Fn() -> bool>,
    /// C# `StartupRegistration.Set(bool)`（含失敗時寫「設定開機啟動失敗」紀錄）。
    pub set_startup: Box<dyn Fn(bool)>,
}

const ANCHORS: [(TaskbarAnchor, &str); 4] = [
    (TaskbarAnchor::Left, "最左邊"),
    (TaskbarAnchor::BeforeStart, "工作列圖示左側"),
    (TaskbarAnchor::AfterIcons, "工作列圖示右側"),
    (TaskbarAnchor::BeforeTray, "通知區域左側"),
];

const LAYOUTS: [(WidgetLayout, &str); 2] =
    [(WidgetLayout::OneLine, "一排（較寬）"), (WidgetLayout::TwoLine, "兩排（較窄）")];

mod cmd {
    pub const PANEL: usize = 1;
    pub const WORKSPACE: usize = 2;
    pub const TIME_ENTRIES: usize = 3;
    pub const REFRESH: usize = 4;
    pub const ANCHOR0: usize = 10;
    pub const RESET_OFFSET: usize = 20;
    pub const DRAG_HINT: usize = 21;
    pub const LAYOUT0: usize = 30;
    pub const LOGIN: usize = 40;
    pub const SETTINGS: usize = 41;
    pub const STARTUP: usize = 42;
    pub const OPEN_WEB: usize = 50;
    pub const OPEN_LOG: usize = 51;
    pub const EXIT: usize = 60;
}

struct TrayInner {
    window: RefCell<Option<OwnedWindow>>,
    actions: TrayActions,
    icon: HICON,
    owns_icon: bool,
    tip: RefCell<String>,
    added: Cell<bool>,
}

impl TrayInner {
    fn hwnd(&self) -> Option<HWND> {
        self.window.borrow().as_ref().map(|w| w.hwnd())
    }

    fn data(&self, hwnd: HWND) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: TRAY_ID, ..Default::default() }
    }

    fn add(&self) {
        let Some(hwnd) = self.hwnd() else { return };
        let mut data = self.data(hwnd);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = TRAY_MSG;
        data.hIcon = self.icon;
        copy_wide(&mut data.szTip, &self.tip.borrow());
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &data).as_bool() };
        self.added.set(ok);
        if !ok {
            log::error!("加入系統匣圖示失敗");
        }
    }

    fn show_menu(&self, at: Point) {
        let Some(hwnd) = self.hwnd() else { return };
        let a = &self.actions;
        let chosen = unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let item = |m: HMENU, id: usize, text: &str, checked: bool| {
                let t = wide(text);
                let flags = if checked { MF_STRING | MF_CHECKED } else { MF_STRING };
                let _ = AppendMenuW(m, flags, id, PCWSTR(t.as_ptr()));
            };
            let separator = |m: HMENU| {
                let _ = AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null());
            };
            item(menu, cmd::PANEL, "開啟面板", false);
            item(menu, cmd::WORKSPACE, "工作台…", false);
            item(menu, cmd::TIME_ENTRIES, "工時紀錄…", false);
            item(menu, cmd::REFRESH, "立即同步", false);
            separator(menu);

            if let Ok(position) = CreatePopupMenu() {
                let current = (a.get_anchor)();
                for (i, (anchor, text)) in ANCHORS.iter().enumerate() {
                    item(position, cmd::ANCHOR0 + i, text, *anchor == current);
                }
                separator(position);
                item(position, cmd::RESET_OFFSET, "重設拖曳微調", false);
                let hint = wide("（可直接左右拖曳；空間不夠會自動避開圖示）");
                let _ = AppendMenuW(position, MF_STRING | MF_GRAYED, cmd::DRAG_HINT, PCWSTR(hint.as_ptr()));
                let t = wide("工作列位置");
                let _ = AppendMenuW(menu, MF_POPUP, position.0 as usize, PCWSTR(t.as_ptr()));
            }

            if let Ok(layout) = CreatePopupMenu() {
                let current = (a.get_layout)();
                for (i, (l, text)) in LAYOUTS.iter().enumerate() {
                    item(layout, cmd::LAYOUT0 + i, text, *l == current);
                }
                let t = wide("顯示方式");
                let _ = AppendMenuW(menu, MF_POPUP, layout.0 as usize, PCWSTR(t.as_ptr()));
            }

            item(menu, cmd::LOGIN, if (a.is_logged_in)() { "登出 Hayaku" } else { "登入 Hayaku" }, false);
            item(menu, cmd::SETTINGS, "設定…", false);
            let startup = (a.is_startup_enabled)();
            item(menu, cmd::STARTUP, "開機時自動啟動", startup);
            separator(menu);
            item(menu, cmd::OPEN_WEB, "開啟 Hayaku 網頁", false);
            item(menu, cmd::OPEN_LOG, "開啟紀錄與設定資料夾", false);
            separator(menu);
            item(menu, cmd::EXIT, "結束", false);

            // 選單要能在點到外面時關閉：先把自己的視窗設成前景，結束後送 WM_NULL（Win32 系統匣選單的慣用做法）
            let _ = SetForegroundWindow(hwnd);
            let chosen = TrackPopupMenu(
                menu,
                TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
                at.x,
                at.y,
                None,
                hwnd,
                None,
            );
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu); // 子選單一起摧毀
            (chosen.0 as usize, startup)
        };
        let (id, startup) = chosen;
        match id {
            0 => {}
            cmd::PANEL => (a.toggle_panel)(),
            cmd::WORKSPACE => (a.workspace)(),
            cmd::TIME_ENTRIES => (a.time_entries)(),
            cmd::REFRESH => (a.refresh)(),
            cmd::RESET_OFFSET => (a.reset_offset)(),
            cmd::LOGIN => (a.login_or_logout)(),
            cmd::SETTINGS => (a.settings)(),
            cmd::STARTUP => (a.set_startup)(!startup), // CheckOnClick：先切換勾選再套用
            cmd::OPEN_WEB => (a.open_web)(),
            cmd::OPEN_LOG => (a.open_log)(),
            cmd::EXIT => (a.exit)(),
            id if (cmd::ANCHOR0..cmd::ANCHOR0 + ANCHORS.len()).contains(&id) => {
                (a.set_anchor)(ANCHORS[id - cmd::ANCHOR0].0)
            }
            id if (cmd::LAYOUT0..cmd::LAYOUT0 + LAYOUTS.len()).contains(&id) => {
                (a.set_layout)(LAYOUTS[id - cmd::LAYOUT0].0)
            }
            _ => {}
        }
    }
}

/// 系統匣圖示。在 UI 執行緒建立；Drop 時移除圖示。
pub struct TrayIcon {
    inner: Rc<TrayInner>,
}

impl TrayIcon {
    pub fn new(actions: TrayActions) -> Self {
        let (icon, owns_icon) = create_icon();
        let inner = Rc::new(TrayInner {
            window: RefCell::new(None),
            actions,
            icon,
            owns_icon,
            tip: RefCell::new("HayakuBar".into()),
            added: Cell::new(false),
        });
        let weak = Rc::downgrade(&inner);
        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        let handler: Rc<Handler> = Rc::new(move |_hwnd, msg, _wparam, lparam| {
            let inner = weak.upgrade()?;
            if msg == TRAY_MSG {
                match lparam.0 as u32 {
                    WM_LBUTTONUP => (inner.actions.toggle_panel)(),
                    WM_RBUTTONUP => {
                        let mut p = windows::Win32::Foundation::POINT::default();
                        if unsafe { GetCursorPos(&mut p) }.is_ok() {
                            inner.show_menu(Point { x: p.x, y: p.y });
                        }
                    }
                    _ => {}
                }
                return Some(LRESULT(0));
            }
            if taskbar_created != 0 && msg == taskbar_created {
                // explorer 重啟：圖示要重新加回去（WinForms NotifyIcon 也這樣做）
                inner.add();
            }
            None
        });
        match OwnedWindow::hidden("HayakuBarTray", "HayakuBarTray", handler) {
            Ok(w) => *inner.window.borrow_mut() = Some(w),
            Err(e) => log::error!("建立系統匣視窗失敗：{e}"),
        }
        inner.add();
        Self { inner }
    }

    /// 在指定的螢幕座標（實體像素）開選單，往右上方展開（C# AboveRight）。
    pub fn show_menu(&self, screen_point: Point) {
        self.inner.show_menu(screen_point);
    }

    pub fn balloon(&self, title: &str, text: &str) {
        let Some(hwnd) = self.inner.hwnd() else { return };
        let mut data = self.inner.data(hwnd);
        data.uFlags = NIF_INFO;
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, text);
        data.dwInfoFlags = NIIF_INFO;
        data.Anonymous.uTimeout = 5000;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    pub fn set_tooltip(&self, text: &str) {
        let text = truncate_utf16(text, 63);
        *self.inner.tip.borrow_mut() = text.clone();
        let Some(hwnd) = self.inner.hwnd() else { return };
        let mut data = self.inner.data(hwnd);
        data.uFlags = NIF_TIP;
        copy_wide(&mut data.szTip, &text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        if let Some(hwnd) = self.inner.hwnd()
            && self.inner.added.get()
        {
            let data = self.inner.data(hwnd);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
        }
        if self.inner.owns_icon {
            unsafe {
                let _ = DestroyIcon(self.inner.icon);
            }
        }
        *self.inner.window.borrow_mut() = None;
    }
}

/// C# `text.Length > 63 ? text[..63] : text`（UTF-16 長度）；不切開代理對。
fn truncate_utf16(text: &str, max: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out
}

fn copy_wide<const N: usize>(dst: &mut [u16; N], text: &str) {
    let mut i = 0;
    for u in text.encode_utf16() {
        if i + 1 >= N {
            break;
        }
        dst[i] = u;
        i += 1;
    }
    dst[i] = 0;
}

/// 選 .ico 裡最接近 `size` 的版本：先找一樣大的，再找比較大的裡面最小的，最後才用最大的；同尺寸取色深最高。
fn pick_ico_entry(ico: &[u8], size: u32) -> Option<&[u8]> {
    let u16_at = |o: usize| ico.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| ico.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u16_at(2)? != 1 {
        return None;
    }
    let count = u16_at(4)? as usize;
    let mut entries = Vec::new();
    for i in 0..count {
        let e = 6 + i * 16;
        let w = match *ico.get(e)? {
            0 => 256,
            v => v as u32,
        };
        let bits = u16_at(e + 6)?;
        let len = u32_at(e + 8)? as usize;
        let offset = u32_at(e + 12)? as usize;
        let data = ico.get(offset..offset.checked_add(len)?)?;
        entries.push((w, bits, data));
    }
    let key = |&(w, bits, _): &(u32, u16, &[u8])| {
        let rank = if w == size {
            0
        } else if w > size {
            1
        } else {
            2
        };
        let dist = w.abs_diff(size);
        (rank, dist, u16::MAX - bits)
    };
    entries.into_iter().min_by_key(key).map(|(_, _, d)| d)
}

/// 系統匣圖示：用 app.ico（crates/hayakubar/assets）裡最接近系統小圖示大小的版本；讀不到才自己畫。回傳（圖示、是否要 DestroyIcon）。
fn create_icon() -> (HICON, bool) {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
    if let Some(data) = pick_ico_entry(APP_ICO, size as u32) {
        match unsafe { CreateIconFromResourceEx(data, true, 0x0003_0000, size, size, LR_DEFAULTCOLOR) } {
            Ok(icon) => return (icon, true),
            Err(e) => log::error!("讀取圖示失敗：{e}"),
        }
    } else {
        log::error!("讀取圖示失敗：app.ico 格式不對");
    }
    match draw_fallback_icon() {
        Some(icon) => (icon, true),
        None => (HICON::default(), false),
    }
}

/// 橘色圓底＋白色時鐘指針（C# 讀不到 app.ico 時畫的那個）。
fn draw_fallback_icon() -> Option<HICON> {
    use tiny_skia::{LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
    let mut pm = Pixmap::new(32, 32)?;
    let mut paint = Paint { anti_alias: true, ..Default::default() };
    paint.set_color_rgba8(217, 98, 43, 255);
    pm.fill_path(
        &PathBuilder::from_circle(16.0, 16.0, 15.0)?,
        &paint,
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
    paint.set_color_rgba8(255, 255, 255, 255);
    let stroke = Stroke { width: 3.2, line_cap: LineCap::Round, ..Default::default() };
    let mut pb = PathBuilder::new();
    pb.move_to(16.0, 16.0);
    pb.line_to(16.0, 7.0);
    pb.move_to(16.0, 16.0);
    pb.line_to(23.0, 20.0);
    pm.stroke_path(&pb.finish()?, &paint, &stroke, Transform::identity(), None);

    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 32,
                biHeight: -32,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        if bits.is_null() {
            let _ = DeleteObject(HGDIOBJ(color.0));
            return None;
        }
        let out = std::slice::from_raw_parts_mut(bits as *mut u8, 32 * 32 * 4);
        // 圖示用非 premultiplied 的 BGRA
        for (p, dst) in pm.pixels().iter().zip(out.as_chunks_mut::<4>().0.iter_mut()) {
            let c = p.demultiply();
            dst.copy_from_slice(&[c.blue(), c.green(), c.red(), c.alpha()]);
        }
        let mask = CreateBitmap(32, 32, 1, 1, None);
        let ii = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii).ok();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_截斷() {
        assert_eq!(truncate_utf16("HayakuBar", 63), "HayakuBar");
        let long = "一".repeat(70);
        assert_eq!(truncate_utf16(&long, 63).chars().count(), 63);
    }

    #[test]
    fn 圖示選最接近的尺寸() {
        let data = pick_ico_entry(APP_ICO, 16).expect("app.ico 有圖");
        assert!(!data.is_empty());
        assert!(pick_ico_entry(&[0, 0, 2, 0, 0, 0], 16).is_none());
    }
}
