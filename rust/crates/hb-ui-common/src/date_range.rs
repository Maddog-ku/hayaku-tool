//! 對應 C# `UI/DateRangePicker.xaml(.cs)` 的 Rust 端：日期與 Slint 整數（yyyymmdd，0 表示未指定）互轉。
//!
//! 元件本身（`@hbc` 的 `DateRangePicker`）整個在 Slint 裡：輸入框「開始 ~ 結束 📅」，點一下跳出並排的兩個月日曆
//! （週一開始），點第一下是開始、第二下是結束，按「套用日期」才生效；只選一天就套用＝開始和結束同一天；
//! 「清除」把兩個日期都清掉；`single` 只選一天，點一下日期就直接套用。
//! 屬性：`in-out start`、`in-out end`（yyyymmdd）、`in single`、`in today`；回呼 `applied()`（按了套用或清除）。

use chrono::{Datelike, NaiveDate};

/// 日期 → yyyymmdd；None → 0。
pub fn to_int(d: Option<NaiveDate>) -> i32 {
    d.map_or(0, |d| d.year() * 10000 + d.month() as i32 * 100 + d.day() as i32)
}

/// yyyymmdd → 日期；0 或不合法 → None。
pub fn from_int(v: i32) -> Option<NaiveDate> {
    if v <= 0 {
        return None;
    }
    NaiveDate::from_ymd_opt(v / 10000, (v / 100 % 100) as u32, (v % 100) as u32)
}

/// 今天的 yyyymmdd（給 `today` 屬性）。
pub fn today_int() -> i32 {
    to_int(Some(chrono::Local::now().date_naive()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日期與整數互轉() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        assert_eq!(20261109, to_int(Some(d)));
        assert_eq!(0, to_int(None));
        assert_eq!(Some(d), from_int(20261109));
        assert_eq!(None, from_int(0));
        assert_eq!(None, from_int(20261332));
    }
}
