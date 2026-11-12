//! 對應 C# `Infra/AppPaths.cs`：%APPDATA%\HayakuBar 底下各檔案的路徑。
//!
//! 測試可以用 [`AppPaths::set_dir`] 改成暫存資料夾，不碰真正的設定檔。

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

pub struct AppPaths;

static DEFAULT_DIR: OnceLock<PathBuf> = OnceLock::new();
static OVERRIDE_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

impl AppPaths {
    /// %APPDATA%\HayakuBar（第一次用到時建立）。
    pub fn dir() -> PathBuf {
        if let Ok(guard) = OVERRIDE_DIR.read()
            && let Some(dir) = guard.as_ref()
        {
            return dir.clone();
        }
        DEFAULT_DIR.get_or_init(create_dir).clone()
    }

    /// 改用別的根目錄（測試用；`None` 還原成 %APPDATA%\HayakuBar）。會建立資料夾。
    pub fn set_dir(dir: Option<&Path>) {
        if let Some(d) = dir {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(mut guard) = OVERRIDE_DIR.write() {
            *guard = dir.map(Path::to_path_buf);
        }
    }

    pub fn config() -> PathBuf {
        Self::dir().join("config.json")
    }

    pub fn state() -> PathBuf {
        Self::dir().join("state.json")
    }

    pub fn auth() -> PathBuf {
        Self::dir().join("auth.bin")
    }

    /// EIP 帳密（DPAPI 加密）。
    pub fn eip_auth() -> PathBuf {
        Self::dir().join("eip.bin")
    }

    /// EIP 請假同步的狀態（上次同步、由同步建立的請假）。
    pub fn eip_sync() -> PathBuf {
        Self::dir().join("eip-sync.json")
    }

    /// Slack App 的金鑰與要傳給誰（DPAPI 加密）。
    pub fn slack_auth() -> PathBuf {
        Self::dir().join("slack.bin")
    }

    /// 傳到 Slack 的訊息（訊息編號對應哪個通知、處理了沒）。
    pub fn slack_state() -> PathBuf {
        Self::dir().join("slack.json")
    }

    /// 紀錄資料夾，一天一個檔（20261029093301.log）。
    pub fn logs() -> PathBuf {
        Self::dir().join("logs")
    }
}

fn create_dir() -> PathBuf {
    let dir = roaming_app_data().join("HayakuBar");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// `Environment.GetFolderPath(SpecialFolder.ApplicationData)`：SHGetKnownFolderPath(FOLDERID_RoamingAppData)。
fn roaming_app_data() -> PathBuf {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath};
    // SAFETY: SHGetKnownFolderPath 回傳的字串要用 CoTaskMemFree 釋放
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KNOWN_FOLDER_FLAG(0), None) {
            let s = p.to_string();
            CoTaskMemFree(Some(p.0 as *const _));
            if let Ok(s) = s
                && !s.is_empty()
            {
                return PathBuf::from(s);
            }
        }
    }
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Mutex, MutexGuard};

    static LOCK: Mutex<()> = Mutex::new(());

    /// 會改 AppPaths 根目錄的測試要先拿這個鎖，避免平行執行互相干擾。
    pub(crate) fn lock() -> MutexGuard<'static, ()> {
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }
}
