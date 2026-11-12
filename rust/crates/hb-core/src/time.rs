//! C# `DateTime.Now`／`DateTimeOffset.Now` 的對應（本地時間），以及 `DateTime.MinValue`。

use chrono::{Local, NaiveDate, NaiveDateTime};
use hb_api::DateTimeOffset;

/// `DateTime.MinValue`。
pub const MIN: NaiveDateTime = NaiveDateTime::MIN;

/// `DateTime.Now`（本地時間）。
pub fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// `DateTime.Today`。
pub fn today() -> NaiveDate {
    now().date()
}

/// `DateTimeOffset.Now`。
pub fn now_offset() -> DateTimeOffset {
    Local::now().fixed_offset()
}

/// `dto.LocalDateTime`。
pub fn local(dto: &DateTimeOffset) -> NaiveDateTime {
    dto.with_timezone(&Local).naive_local()
}

/// 本地時間 → `DateTimeOffset`（C# 把 `DateTime` 隱含轉成 `DateTimeOffset`）。
pub fn offset(local_time: NaiveDateTime) -> DateTimeOffset {
    use chrono::TimeZone;
    match Local.from_local_datetime(&local_time) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.fixed_offset(),
        chrono::LocalResult::None => Local.from_utc_datetime(&local_time).fixed_offset(),
    }
}

/// `yyyy-MM-dd`。
pub fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}
