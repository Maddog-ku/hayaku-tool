//! 對應 C# `Infra/Web.cs`：用預設瀏覽器（或檔案總管）開啟。

/// 用預設瀏覽器（或檔案總管）開啟；失敗寫紀錄。
pub fn open(target: &str) {
    if let Err(err) = shell_open(target) {
        log::error!("開啟 {target} 失敗: {err}");
    }
}

/// `Process.Start(new ProcessStartInfo(target) { UseShellExecute = true })`；失敗回傳錯誤。
///
/// 跟 .NET 一樣在 STA 執行緒上呼叫 ShellExecuteEx（呼叫端可能是 tokio 的執行緒，沒有初始化 COM）。
pub fn shell_open(target: &str) -> windows::core::Result<()> {
    let target = target.to_owned();
    std::thread::spawn(move || shell_execute_sta(&target))
        .join()
        .unwrap_or_else(|_| Err(windows::core::Error::from(windows::Win32::Foundation::E_FAIL)))
}

fn shell_execute_sta(target: &str) -> windows::core::Result<()> {
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR};

    let file = HSTRING::from(target);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: 這條執行緒自己初始化／解除 COM；info 的字串指標在呼叫期間有效
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let result = ShellExecuteExW(&mut info);
        if com.is_ok() {
            CoUninitialize();
        }
        result
    }
}
