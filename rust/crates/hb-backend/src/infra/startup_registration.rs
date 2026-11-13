//! 對應 C# `Infra/StartupRegistration.cs`：開機自動啟動，寫在 HKCU\...\Run，不需要系統管理員權限。

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{HSTRING, PCWSTR};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const NAME: &str = "HayakuBar";

pub fn is_enabled() -> bool {
    is_enabled_at(RUN_KEY, NAME)
}

/// 開啟或關閉開機自動啟動（值是加上引號的執行檔路徑）。
pub fn set(enabled: bool) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    set_at(RUN_KEY, NAME, enabled, &format!("\"{}\"", exe.display()))
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: handle 由 RegOpenKeyExW／RegCreateKeyExW 開啟
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn check(code: WIN32_ERROR) -> windows::core::Result<()> {
    code.ok()
}

/// 值存在而且是字串（C# `key.GetValue(Name) is string`：REG_SZ、REG_EXPAND_SZ）。
pub(crate) fn is_enabled_at(sub_key: &str, name: &str) -> bool {
    let sub_key = HSTRING::from(sub_key);
    let name = HSTRING::from(name);
    let mut key = HKEY::default();
    // SAFETY: 參數都是有效的 null 結尾字串與輸出指標
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, &sub_key, None, KEY_READ, &mut key).is_err() {
            return false;
        }
        let key = Key(key);
        let mut kind = REG_VALUE_TYPE::default();
        RegQueryValueExW(key.0, &name, None, Some(&mut kind), None, None).is_ok()
            && (kind == REG_SZ || kind == REG_EXPAND_SZ)
    }
}

pub(crate) fn set_at(sub_key: &str, name: &str, enabled: bool, value: &str) -> anyhow::Result<()> {
    let sub_key = HSTRING::from(sub_key);
    let name = HSTRING::from(name);
    let mut key = HKEY::default();
    // SAFETY: 參數都是有效的 null 結尾字串與輸出指標；寫入的資料是 UTF-16 含結尾 0
    unsafe {
        check(RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &sub_key,
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut key,
            None,
        ))?;
        let key = Key(key);
        if enabled {
            let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            check(RegSetValueExW(key.0, &name, None, REG_SZ, Some(bytes)))?;
        } else {
            let code = RegDeleteValueW(key.0, &name);
            if code != ERROR_FILE_NOT_FOUND {
                check(code)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 寫入讀取刪除_用測試專用的機碼() {
        use windows::Win32::System::Registry::RegDeleteTreeW;
        let sub_key = format!(r"Software\HayakuBar.Tests\{}", std::process::id());
        assert!(!is_enabled_at(&sub_key, NAME));
        set_at(&sub_key, NAME, true, "\"C:\\x\\HayakuBar.exe\"").unwrap();
        assert!(is_enabled_at(&sub_key, NAME));
        set_at(&sub_key, NAME, false, "").unwrap();
        assert!(!is_enabled_at(&sub_key, NAME));
        set_at(&sub_key, NAME, false, "").unwrap(); // 不存在也不會出錯
        // SAFETY: 刪掉測試建立的機碼
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(r"Software\HayakuBar.Tests"));
        }
    }
}
