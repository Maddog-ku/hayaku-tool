//! 原生（Win32）彈出選單：面板任務列的 ⋯／右鍵、改狀態、托盤右鍵、通知「稍後」、設定「從最近的 session 加入」
//! （C# 是 WPF ContextMenu）。選單內容在打開前就決定好，選了哪一項回傳索引。
//!
//! TrackPopupMenu 會跑自己的訊息迴圈；為了不在 Slint 處理滑鼠事件的途中重入，呼叫端用 [`show_later`] 延到下一輪事件迴圈再開。

use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, MF_GRAYED, MF_SEPARATOR, MF_STRING, PostMessageW, SetForegroundWindow,
    TPM_HORIZONTAL, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, TPM_VERTICAL, TPMPARAMS,
    TrackPopupMenuEx, WM_NULL,
};
use windows::core::PCWSTR;

/// 選單的一項。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item { text: String, enabled: bool },
    Separator,
}

impl MenuEntry {
    pub fn item(text: impl Into<String>) -> Self {
        MenuEntry::Item { text: text.into(), enabled: true }
    }

    pub fn item_if(text: impl Into<String>, enabled: bool) -> Self {
        MenuEntry::Item { text: text.into(), enabled }
    }
}

/// 選單擺哪裡（螢幕座標，實體像素）。
#[derive(Debug, Clone, Copy)]
pub enum MenuPlace {
    /// 擺在這個矩形（按鈕）右側，右邊放不下改放左側（C# OpenMenu 的 CustomPopupPlacement）。
    RightOf(hb_win::Rect),
    /// 擺在這個矩形正下方（C# PlacementMode.Bottom）。
    Below(hb_win::Rect),
}

/// 顯示選單，回傳選了第幾項（分隔線也算一項）；按選單外面關掉時 None。只能在 UI 執行緒呼叫。
pub fn show(owner: &slint::Window, entries: &[MenuEntry], place: MenuPlace) -> Option<usize> {
    let hwnd = crate::popup::hwnd(owner)?;
    let wide: Vec<Vec<u16>> = entries
        .iter()
        .map(|e| match e {
            MenuEntry::Item { text, .. } => text.encode_utf16().chain(std::iter::once(0)).collect(),
            MenuEntry::Separator => vec![0],
        })
        .collect();
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        for (i, (entry, text)) in entries.iter().zip(&wide).enumerate() {
            let id = i + 1;
            let _ = match entry {
                MenuEntry::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()),
                MenuEntry::Item { enabled, .. } => {
                    let flags = if *enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                    AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()))
                }
            };
        }
        let (x, y, flags, exclude) = match place {
            MenuPlace::RightOf(r) => (r.right, r.top, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_HORIZONTAL, Some(r)),
            MenuPlace::Below(r) => (r.left, r.bottom, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_VERTICAL, Some(r)),
        };
        let params = exclude.map(|r| TPMPARAMS {
            cbSize: size_of::<TPMPARAMS>() as u32,
            rcExclude: RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
        });
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenuEx(
            menu,
            (flags | TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            x,
            y,
            hwnd,
            params.as_ref().map(|p| p as *const TPMPARAMS),
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        let id = chosen.0 as usize;
        (id >= 1 && id <= entries.len()).then(|| id - 1)
    }
}

/// 延到下一輪事件迴圈再顯示選單，選了就呼叫 `chosen(索引)`。
pub fn show_later(
    owner: slint::Weak<impl slint::ComponentHandle + 'static>,
    entries: Vec<MenuEntry>,
    place: MenuPlace,
    chosen: impl FnOnce(usize) + 'static,
) {
    slint::Timer::single_shot(Duration::ZERO, move || {
        let Some(ui) = owner.upgrade() else { return };
        if let Some(i) = show(ui.window(), &entries, place) {
            chosen(i);
        }
    });
}

/// Slint 視窗座標（邏輯像素）的一個矩形 → 螢幕矩形（實體像素）。
pub fn to_screen(window: &slint::Window, x: f32, y: f32, w: f32, h: f32) -> hb_win::Rect {
    let scale = window.scale_factor();
    let pos = window.position();
    let left = pos.x + (x * scale).round() as i32;
    let top = pos.y + (y * scale).round() as i32;
    hb_win::Rect { left, top, right: left + (w * scale).round() as i32, bottom: top + (h * scale).round() as i32 }
}
