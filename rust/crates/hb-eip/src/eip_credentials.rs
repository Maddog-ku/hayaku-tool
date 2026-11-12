//! 對應 C# `Eip/EipCredentials.cs`：EIP 帳密，用 Windows DPAPI（目前使用者）加密後存成 eip.bin，別的帳號或電腦解不開。

use std::path::Path;

use hb_backend::infra::{AppPaths, dpapi, json_file};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EipCredentials {
    pub account: String,
    pub password: String,
}

impl EipCredentials {
    pub fn new(account: impl Into<String>, password: impl Into<String>) -> Self {
        Self { account: account.into(), password: password.into() }
    }

    pub fn is_empty(&self) -> bool {
        self.account.is_empty() || self.password.is_empty()
    }
}

/// EIP 帳密的存取（C# `EipCredentialStore`）。
pub struct EipCredentialStore;

/// DPAPI 的 entropy，跟 C# 版一樣。
const ENTROPY: &[u8] = b"HayakuBar.Eip.v1";

impl EipCredentialStore {
    /// 讀 %APPDATA%\HayakuBar\eip.bin；沒有或解不開回空的。
    pub fn load() -> EipCredentials {
        Self::load_from(&AppPaths::eip_auth())
    }

    pub fn load_from(path: &Path) -> EipCredentials {
        if !path.is_file() {
            return EipCredentials::default();
        }
        let result = (|| -> anyhow::Result<EipCredentials> {
            let raw = dpapi::unprotect(&std::fs::read(path)?, ENTROPY)?;
            Ok(json_file::from_slice::<EipCredentials>(&raw)?.unwrap_or_default())
        })();
        result.unwrap_or_else(|err| {
            log::error!("讀取 EIP 帳密失敗: {err}");
            EipCredentials::default()
        })
    }

    /// 加密後存檔；失敗回錯誤（C# 丟例外）。
    pub fn save(data: &EipCredentials) -> anyhow::Result<()> {
        Self::save_to(&AppPaths::eip_auth(), data)
    }

    pub fn save_to(path: &Path, data: &EipCredentials) -> anyhow::Result<()> {
        let raw = json_file::to_vec(data)?;
        std::fs::write(path, dpapi::protect(&raw, ENTROPY)?)?;
        Ok(())
    }

    pub fn clear() {
        Self::clear_at(&AppPaths::eip_auth());
    }

    pub fn clear_at(path: &Path) {
        // File.Delete：檔案不存在不算錯
        if let Err(err) = std::fs::remove_file(path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("刪除 EIP 帳密失敗: {err}");
        }
    }
}
