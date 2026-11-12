//! 對應 C# `Auth/TokenStore.cs`：登入資料用 Windows DPAPI（目前使用者）加密後存檔，別的帳號或電腦解不開。

use std::path::Path;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::infra::app_paths::AppPaths;
use crate::infra::{dpapi, json_file};

/// DPAPI 的 entropy（跟 C# 一樣）。
pub const ENTROPY: &[u8] = b"HayakuBar.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AuthData {
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    #[serde(with = "date_time_offset")]
    pub expires_at: DateTime<FixedOffset>,
}

impl Default for AuthData {
    fn default() -> Self {
        Self {
            client_id: None,
            redirect_uri: None,
            access_token: None,
            refresh_token: None,
            expires_at: date_time_offset::min_value(),
        }
    }
}

/// 讀 %APPDATA%\HayakuBar\auth.bin；沒有檔案回傳空的；解不開寫紀錄後回傳空的。
pub fn load() -> AuthData {
    load_from(&AppPaths::auth())
}

/// 寫 %APPDATA%\HayakuBar\auth.bin（失敗回傳錯誤，跟 C# 一樣往外丟）。
pub fn save(data: &AuthData) -> anyhow::Result<()> {
    save_to(&AppPaths::auth(), data)
}

pub fn load_from(path: &Path) -> AuthData {
    if !path.is_file() {
        return AuthData::default();
    }
    let result = (|| -> anyhow::Result<AuthData> {
        let raw = dpapi::unprotect(&std::fs::read(path)?, ENTROPY)?;
        Ok(json_file::from_slice::<AuthData>(&raw)?.unwrap_or_default())
    })();
    result.unwrap_or_else(|err| {
        log::error!("讀取登入資料失敗: {err}");
        AuthData::default()
    })
}

pub fn save_to(path: &Path, data: &AuthData) -> anyhow::Result<()> {
    let raw = json_file::to_vec(data)?;
    std::fs::write(path, dpapi::protect(&raw, ENTROPY)?)?;
    Ok(())
}

/// C# `DateTimeOffset` 的 JSON 格式：`2026-11-12T08:30:15.1234567+00:00`（小數最多 7 位、去掉尾端的 0）。
pub mod date_time_offset {
    use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
    use serde::{Deserialize, Deserializer, Serializer};

    /// `DateTimeOffset.MinValue`（0001-01-01T00:00:00+00:00）。
    pub fn min_value() -> DateTime<FixedOffset> {
        let naive = NaiveDate::from_ymd_opt(1, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0)).unwrap_or_default();
        FixedOffset::east_opt(0).map(|z| z.from_utc_datetime(&naive)).unwrap_or_else(|| naive.and_utc().fixed_offset())
    }

    pub fn format(value: &DateTime<FixedOffset>) -> String {
        let mut text = value.format("%Y-%m-%dT%H:%M:%S").to_string();
        let ticks = value.nanosecond() % 1_000_000_000 / 100;
        if ticks > 0 {
            let fraction = format!("{ticks:07}");
            text.push('.');
            text.push_str(fraction.trim_end_matches('0'));
        }
        text.push_str(&value.format("%:z").to_string());
        text
    }

    pub fn parse(text: &str) -> Option<DateTime<FixedOffset>> {
        if let Ok(v) = DateTime::parse_from_rfc3339(text) {
            return Some(v);
        }
        // 沒有時區：當成本機時間（跟 .NET 一樣）
        let naive = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
            .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M"))
            .or_else(|_| {
                NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|d| d.and_hms_opt(0, 0, 0).unwrap_or_default())
            })
            .ok()?;
        Local.from_local_datetime(&naive).earliest().map(|v| v.fixed_offset())
    }

    pub fn serialize<S: Serializer>(value: &DateTime<FixedOffset>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<FixedOffset>, D::Error> {
        let text = String::deserialize(d)?;
        parse(&text).ok_or_else(|| serde::de::Error::custom(format!("無法解析的時間：{text}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C# 版（.NET 10）實際寫出的 AuthData JSON。
    const CS_AUTH: &str = "{\r\n  \"clientId\": \"cid\",\r\n  \"redirectUri\": \"http://127.0.0.1:47824/callback\",\r\n  \"accessToken\": \"at\",\r\n  \"refreshToken\": null,\r\n  \"expiresAt\": \"2026-11-12T08:30:15.1234567+00:00\"\r\n}";
    const CS_AUTH_DEFAULT: &str = "{\r\n  \"clientId\": null,\r\n  \"redirectUri\": null,\r\n  \"accessToken\": null,\r\n  \"refreshToken\": null,\r\n  \"expiresAt\": \"0001-01-01T00:00:00+00:00\"\r\n}";

    #[test]
    fn 跟c_sharp格式相容() {
        let data: AuthData = serde_json::from_str(CS_AUTH).unwrap();
        assert_eq!(Some("cid"), data.client_id.as_deref());
        assert_eq!(None, data.refresh_token);
        assert_eq!(CS_AUTH, json_file::to_string(&data).unwrap());
        assert_eq!(CS_AUTH_DEFAULT, json_file::to_string(&AuthData::default()).unwrap());
        assert_eq!(AuthData::default(), serde_json::from_str::<AuthData>(CS_AUTH_DEFAULT).unwrap());
        assert_eq!(AuthData::default(), serde_json::from_str::<AuthData>("{}").unwrap());
    }

    #[test]
    fn dpapi存檔再讀回() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("auth.bin");
        assert_eq!(AuthData::default(), load_from(&path));
        let data: AuthData = serde_json::from_str(CS_AUTH).unwrap();
        save_to(&path, &data).unwrap();
        assert!(!std::fs::read(&path).unwrap().windows(3).any(|w| w == b"cid"));
        assert_eq!(data, load_from(&path));
        // 壞掉的檔案：回傳空的
        std::fs::write(&path, b"garbage").unwrap();
        assert_eq!(AuthData::default(), load_from(&path));
    }

    /// 手動跟 C# 版互通測試（DPAPI 檔只有同一個 Windows 帳號解得開，所以不放進一般測試）：
    /// 先用 C# 的 `ProtectedData` 產生 `cs_auth.bin`（內容同 CS_AUTH），設定 `HB_CS_FIXTURE_DIR` 指向那個資料夾，
    /// 執行 `cargo test -p hb-backend -- --ignored`；會寫出 `rust_auth.bin` 讓 C# 端解密驗證。
    #[test]
    #[ignore]
    fn 手動_跟c_sharp的dpapi檔互通() {
        let dir = std::path::PathBuf::from(std::env::var("HB_CS_FIXTURE_DIR").unwrap());
        let data = load_from(&dir.join("cs_auth.bin"));
        assert_eq!(serde_json::from_str::<AuthData>(CS_AUTH).unwrap(), data);
        save_to(&dir.join("rust_auth.bin"), &data).unwrap();
    }
}
