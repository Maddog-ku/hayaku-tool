//! C# `DateTime` 在 System.Text.Json 的格式（eip-sync.json 要跟 C# 版互通）。
//!
//! - `DateTime.Now` 這種本機時間（Kind=Local）寫成 `2026-11-12T09:00:00.1234567+08:00`；
//! - 從字串解析出來的時間（Kind=Unspecified，例如假單的起訖）寫成 `2026-10-30T08:30:00`。
//!
//! 小數秒照 .NET 去掉結尾的 0；讀的時候有時區就換成本機時間，沒有就照原樣。

use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use serde::{Deserialize, Deserializer, Serializer};

/// `DateTime.MinValue`（0001-01-01T00:00:00）。
pub fn min_value() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(1, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0)).unwrap_or(NaiveDateTime::MIN)
}

fn base(t: &NaiveDateTime) -> String {
    let mut s = t.format("%Y-%m-%dT%H:%M:%S").to_string();
    let ticks = t.nanosecond() % 1_000_000_000 / 100;
    if ticks > 0 {
        let frac = format!("{ticks:07}");
        s.push('.');
        s.push_str(frac.trim_end_matches('0'));
    }
    s
}

/// Kind=Unspecified 的寫法（沒有時區）。
pub fn format_unspecified(t: &NaiveDateTime) -> String {
    base(t)
}

/// Kind=Local 的寫法（加上本機時區）。
pub fn format_local(t: &NaiveDateTime) -> String {
    let offset = Local
        .from_local_datetime(t)
        .earliest()
        .map(|d| d.format("%:z").to_string())
        .unwrap_or_else(|| "+00:00".to_owned());
    format!("{}{offset}", base(t))
}

/// 讀：有時區（Z、+08:00）換成本機時間，沒有照原樣。
pub fn parse(text: &str) -> Option<NaiveDateTime> {
    // 沒有時區的會當成本機時間解析，換回本機時間後就是原樣
    hb_api::json::parse_date_time_offset(text).map(|t| t.with_timezone(&Local).naive_local())
}

fn de_text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

/// `DateTime`（Kind=Unspecified）。
pub mod unspecified {
    use super::*;

    pub fn serialize<S: Serializer>(t: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format_unspecified(t))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDateTime, D::Error> {
        let text = de_text(d)?.ok_or_else(|| serde::de::Error::custom("時間不能是 null"))?;
        parse(&text).ok_or_else(|| serde::de::Error::custom(format!("無法解析的時間：{text}")))
    }
}

/// `DateTime?`（Kind=Local，例如 `DateTime.Now`）。
pub mod local_opt {
    use super::*;

    pub fn serialize<S: Serializer>(t: &Option<NaiveDateTime>, s: S) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => s.serialize_str(&format_local(t)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<NaiveDateTime>, D::Error> {
        match de_text(d)? {
            None => Ok(None),
            Some(text) => {
                parse(&text).map(Some).ok_or_else(|| serde::de::Error::custom(format!("無法解析的時間：{text}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 小數秒去掉結尾的0() {
        let t = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap().and_hms_nano_opt(9, 0, 0, 123_400_000).unwrap();
        assert_eq!("2026-11-12T09:00:00.1234", format_unspecified(&t));
        let t = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(8, 30, 0).unwrap();
        assert_eq!("2026-10-30T08:30:00", format_unspecified(&t));
        assert_eq!(Some(t), parse("2026-10-30T08:30:00"));
        let local = format_local(&t);
        assert!(local.starts_with("2026-10-30T08:30:00+") || local.starts_with("2026-10-30T08:30:00-"), "{local}");
        assert_eq!(Some(t), parse(&local));
    }
}
