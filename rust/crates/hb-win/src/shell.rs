//! 對應 C#：`src/HayakuBar/Infra/Web.cs`（Process.Start + UseShellExecute）
//! 用預設瀏覽器（或檔案總管）開啟網址／資料夾／檔案。

use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

use crate::window::wide;

/// 用預設程式開啟；失敗寫紀錄，不回報錯誤。
pub fn open(target: &str) {
    if let Err(e) = try_open(target) {
        log::error!("開啟 {target} 失敗：{e}");
    }
}

/// 用預設程式開啟；回傳錯誤讓呼叫端自己處理。
pub fn try_open(target: &str) -> Result<(), String> {
    let file = wide(target);
    // 沒指定動作＝預設動作（跟 Process.Start UseShellExecute 一樣）
    let result = unsafe {
        ShellExecuteW(None, PCWSTR::null(), PCWSTR(file.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    let code = result.0 as isize;
    if code > 32 { Ok(()) } else { Err(format!("ShellExecute 錯誤 {code}")) }
}
