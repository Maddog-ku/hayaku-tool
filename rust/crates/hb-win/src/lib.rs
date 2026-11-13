//! 原生 Win32：工作列小工具、系統匣、快捷鍵、系統事件
//!
//! 所有視窗都在 UI 執行緒建立；正式程式的 UI 執行緒跑 Slint（winit）事件迴圈，它會替同執行緒的視窗分派訊息，
//! 這裡只建立隱藏視窗／子視窗，不自己跑訊息迴圈。回呼不需要 `Send`。

pub mod global_hotkey;
pub mod native;
pub mod shell;
pub mod single_instance;
pub mod system_events;
pub mod taskbar_icon_scanner;
pub mod taskbar_widget;
pub mod theme;
pub mod tray_icon;
mod window;

pub use global_hotkey::{GlobalHotkey, ModifierKeys};
pub use native::{Point, Rect, user_busy};
pub use shell::open;
pub use single_instance::SingleInstance;
pub use system_events::{SessionSwitchReason, SystemEvents};
pub use taskbar_icon_scanner::{IconRange, TaskbarIconScanner};
pub use taskbar_widget::{TaskbarAnchor, TaskbarWidget, WidgetContent, WidgetLayout, WidgetStatus};
pub use theme::{apps_use_dark_theme, client_area_animation, taskbar_uses_dark_theme, windows_animations_disabled};
pub use tray_icon::{TrayActions, TrayIcon};
