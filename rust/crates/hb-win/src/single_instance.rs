//! 對應 C#：`src/HayakuBar/App.xaml.cs` 的 `new Mutex(true, "HayakuBar.SingleInstance", out var created)`。
//! 單一執行個體：同名 Mutex 已存在表示已經有一個 HayakuBar 在執行。

use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};

use crate::window::wide;

pub const MUTEX_NAME: &str = "HayakuBar.SingleInstance";

/// 持有期間其他執行個體 `acquire` 會得到 None；Drop 時釋放。
pub struct SingleInstance {
    handle: HANDLE,
    owned: bool,
}

impl SingleInstance {
    /// 取得單一執行個體；已經有一個在執行時回傳 None。建立 Mutex 失敗時（極少見）視為可以執行。
    pub fn acquire() -> Option<Self> {
        Self::acquire_named(MUTEX_NAME)
    }

    pub fn acquire_named(name: &str) -> Option<Self> {
        let wname = wide(name);
        match unsafe { CreateMutexW(None, true, windows::core::PCWSTR(wname.as_ptr())) } {
            Ok(handle) => {
                if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                    None
                } else {
                    Some(Self { handle, owned: true })
                }
            }
            Err(e) => {
                log::error!("建立單一執行個體 Mutex 失敗：{e}");
                Some(Self { handle: HANDLE::default(), owned: false })
            }
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        if self.owned {
            unsafe {
                let _ = ReleaseMutex(self.handle);
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 同名第二個取得失敗() {
        let name = format!("HayakuBar.SingleInstance.Test.{}", std::process::id());
        let first = SingleInstance::acquire_named(&name);
        assert!(first.is_some());
        assert!(SingleInstance::acquire_named(&name).is_none());
        drop(first);
        assert!(SingleInstance::acquire_named(&name).is_some());
    }
}
