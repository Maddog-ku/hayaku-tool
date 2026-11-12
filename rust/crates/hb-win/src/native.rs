//! 對應 C#：`src/HayakuBar/Infra/Native.cs`
//!
//! C# 的 P/Invoke 宣告在 Rust 直接用 `windows` crate；這裡只留有邏輯的函式與共用的座標型別。

use windows::Win32::UI::Shell::SHQueryUserNotificationState;

/// 螢幕座標的矩形（實體像素），對應 C# `Native.RECT`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }
}

impl From<windows::Win32::Foundation::RECT> for Rect {
    fn from(r: windows::Win32::Foundation::RECT) -> Self {
        Self { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

/// 螢幕座標的點（實體像素）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

/// 使用者現在是否不方便被打擾：全螢幕程式、簡報模式、Windows 的安靜時段、螢幕鎖定。
/// （SHQueryUserNotificationState：1 不在電腦前、2 全螢幕、3 全螢幕 D3D、4 簡報、5 可以通知、6 安靜時段、7 全螢幕的市集 app）
pub fn user_busy() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(state) => busy_state(state.0),
        Err(_) => false,
    }
}

fn busy_state(state: i32) -> bool {
    matches!(state, 1 | 2 | 3 | 4 | 6 | 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 忙碌狀態對應() {
        for s in [1, 2, 3, 4, 6, 7] {
            assert!(busy_state(s), "{s}");
        }
        assert!(!busy_state(5));
        assert!(!busy_state(0));
    }

    #[test]
    fn rect_尺寸() {
        let r = Rect { left: 10, top: 20, right: 40, bottom: 20 };
        assert_eq!(r.width(), 30);
        assert_eq!(r.height(), 0);
        assert!(r.is_empty());
    }
}
