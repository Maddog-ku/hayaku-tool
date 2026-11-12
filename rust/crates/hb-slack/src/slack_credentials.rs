//! 對應 C# `Slack/SlackCredentials.cs`：Slack App 的兩組金鑰和要把私訊傳給誰（成員 ID），
//! 用 Windows DPAPI（目前使用者）加密後存成 %APPDATA%\HayakuBar\slack.bin，別的帳號或電腦解不開。

use std::path::Path;

use hb_backend::infra::{AppPaths, dpapi, json_file};
use serde::{Deserialize, Serialize};

/// Slack App 的兩組金鑰，和要把私訊傳給誰（成員 ID）。team 只是顯示用。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlackCredentials {
    /// Bot User OAuth Token（xoxb-），用來傳訊息、改訊息。
    pub bot_token: String,

    /// App-Level Token（xapp-，connections:write），用來連 Socket Mode 收按鈕。
    pub app_token: String,

    /// 自己的 Slack 成員 ID（U 開頭）。
    pub user_id: String,

    pub team: String,
}

impl SlackCredentials {
    pub fn is_empty(&self) -> bool {
        self.bot_token.is_empty() || self.app_token.is_empty() || self.user_id.is_empty()
    }
}

/// Slack 金鑰的存取（C# `SlackCredentialStore`）。
pub struct SlackCredentialStore;

/// DPAPI 的 entropy，跟 C# 版一樣。
const ENTROPY: &[u8] = b"HayakuBar.Slack.v1";

impl SlackCredentialStore {
    /// 讀 slack.bin；沒有或解不開回空的。
    pub fn load() -> SlackCredentials {
        Self::load_from(&AppPaths::slack_auth())
    }

    pub fn load_from(path: &Path) -> SlackCredentials {
        if !path.is_file() {
            return SlackCredentials::default();
        }
        let result = (|| -> anyhow::Result<SlackCredentials> {
            let raw = dpapi::unprotect(&std::fs::read(path)?, ENTROPY)?;
            Ok(json_file::from_slice::<SlackCredentials>(&raw)?.unwrap_or_default())
        })();
        result.unwrap_or_else(|err| {
            log::error!("讀取 Slack 金鑰失敗: {err}");
            SlackCredentials::default()
        })
    }

    /// 加密後存檔；失敗回錯誤訊息（C# 丟例外）。
    pub fn save(data: &SlackCredentials) -> Result<(), String> {
        Self::save_to(&AppPaths::slack_auth(), data)
    }

    pub fn save_to(path: &Path, data: &SlackCredentials) -> Result<(), String> {
        let raw = json_file::to_vec(data).map_err(|e| e.to_string())?;
        let enc = dpapi::protect(&raw, ENTROPY).map_err(|e| e.to_string())?;
        std::fs::write(path, enc).map_err(|e| e.to_string())
    }

    pub fn clear() {
        Self::clear_at(&AppPaths::slack_auth());
    }

    pub fn clear_at(path: &Path) {
        // File.Delete：檔案不存在不算錯
        if let Err(err) = std::fs::remove_file(path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("刪除 Slack 金鑰失敗: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 少一樣就是空的() {
        let full = SlackCredentials {
            bot_token: "xoxb-1".into(),
            app_token: "xapp-1".into(),
            user_id: "U1".into(),
            team: String::new(),
        };
        assert!(!full.is_empty());
        assert!(SlackCredentials { user_id: String::new(), ..full.clone() }.is_empty());
        assert!(SlackCredentials { bot_token: String::new(), ..full.clone() }.is_empty());
        assert!(SlackCredentials { app_token: String::new(), ..full }.is_empty());
    }

    #[test]
    fn 讀得懂c_sharp存的slack_bin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slack.bin");
        // C# JsonSerializer.SerializeToUtf8Bytes(data, JsonFile.Options)；isEmpty 是 JsonIgnore
        let cs = "{\r\n  \"botToken\": \"xoxb-1\",\r\n  \"appToken\": \"xapp-1\",\r\n  \"userId\": \"U1\",\r\n  \"team\": \"公司\"\r\n}";
        std::fs::write(&path, dpapi::protect(cs.as_bytes(), b"HayakuBar.Slack.v1").unwrap()).unwrap();
        let c = SlackCredentialStore::load_from(&path);
        assert_eq!(
            ("xoxb-1", "xapp-1", "U1", "公司"),
            (c.bot_token.as_str(), c.app_token.as_str(), c.user_id.as_str(), c.team.as_str())
        );

        // Rust 存的：C# 用同樣的 entropy 解得開，內容格式一樣
        SlackCredentialStore::save_to(&path, &c).unwrap();
        let raw = dpapi::unprotect(&std::fs::read(&path).unwrap(), b"HayakuBar.Slack.v1").unwrap();
        assert_eq!(cs, String::from_utf8(raw).unwrap());
    }

    #[test]
    fn 解不開或沒檔案_回空的_刪除不存在的檔案不算錯() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slack.bin");
        assert!(SlackCredentialStore::load_from(&path).is_empty());
        std::fs::write(&path, dpapi::protect(b"{}", b"HayakuBar.v1").unwrap()).unwrap();
        assert!(SlackCredentialStore::load_from(&path).is_empty());
        SlackCredentialStore::clear_at(&path);
        assert!(!path.exists());
        SlackCredentialStore::clear_at(&path);
    }
}
