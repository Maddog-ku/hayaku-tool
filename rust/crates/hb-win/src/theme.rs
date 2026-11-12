//! 系統外觀查詢：深淺色（對應 C# `UI/ThemeManager.cs` 的 SystemAppsDark、`UI/TaskbarWidget.cs` 的 DarkTaskbar）
//! 與「顯示動畫」設定（WPF `SystemParameters.ClientAreaAnimation`）。
//! 變更通知見 [`crate::SystemEvents`]。

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, w};

fn personalize_dword(name: &str) -> Option<u32> {
    let name = crate::window::wide(name);
    let mut value: u32 = 0;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            windows::core::PCWSTR(name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(value)
}

/// 應用程式是否用深色（AppsUseLightTheme = 0）；讀不到算淺色。
pub fn apps_use_dark_theme() -> bool {
    personalize_dword("AppsUseLightTheme") == Some(0)
}

/// 工作列（系統）是否用深色（SystemUsesLightTheme = 0）；讀不到算深色（跟 C# DarkTaskbar 一樣）。
pub fn taskbar_uses_dark_theme() -> bool {
    personalize_dword("SystemUsesLightTheme").is_none_or(|v| v == 0)
}

/// Windows「顯示動畫」是否開啟（SPI_GETCLIENTAREAANIMATION）；查不到算開啟。
pub fn client_area_animation() -> bool {
    let mut enabled = BOOL(1);
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut enabled as *mut BOOL as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || enabled.as_bool()
}

/// Windows 是否關閉動畫（Slint `Theme.reduce-motion` 用）。
pub fn windows_animations_disabled() -> bool {
    !client_area_animation()
}
