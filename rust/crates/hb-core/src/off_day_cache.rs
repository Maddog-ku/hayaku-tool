//! 對應 C# `Core/OffDayCache.cs`：接下來不用上班的日子（請假、放假、週末）存在本機，那幾天不問 Hayaku 就知道今天不用上班。

use chrono::NaiveDate;
use hb_api::DaySchedule;
use indexmap::IndexMap;

pub struct OffDayCache;

const LEAVE: &str = "請假";

impl OffDayCache {
    /// 從班表挑出 today 之後沒有上班時段的日子：日期 → 「請假」、假日名稱或空字串（同一天只取第一筆）。
    pub fn from<'a>(days: impl IntoIterator<Item = &'a DaySchedule>, today: NaiveDate) -> IndexMap<String, String> {
        let mut map = IndexMap::new();
        for d in days {
            if d.date <= today || !d.working.is_empty() {
                continue;
            }
            let key = d.date.format("%Y-%m-%d").to_string();
            if map.contains_key(&key) {
                continue;
            }
            let label = d.holiday.clone().unwrap_or_else(|| if d.on_leave { LEAVE.to_owned() } else { String::new() });
            map.insert(key, label);
        }
        map
    }

    /// today 是快取裡不用上班的日子：回傳當作今天班表的 DaySchedule（沒有上班時段）；否則 None。
    pub fn today(off_days: &IndexMap<String, String>, today: NaiveDate) -> Option<DaySchedule> {
        let label = off_days.get(&today.format("%Y-%m-%d").to_string())?;
        Some(DaySchedule {
            date: today,
            working: Vec::new(),
            expected_minutes: 0,
            on_leave: label == LEAVE,
            holiday: if label.is_empty() || label == LEAVE { None } else { Some(label.clone()) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveTime, TimeDelta};
    use hb_api::WorkSegment;

    // 對應 OffDayCacheTests

    fn thu() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()
    }

    fn normal() -> Vec<WorkSegment> {
        vec![WorkSegment {
            start: NaiveTime::from_hms_opt(8, 30, 0).unwrap(),
            end: NaiveTime::from_hms_opt(17, 30, 0).unwrap(),
        }]
    }

    fn day(offset: i64, working: Vec<WorkSegment>, minutes: i32, on_leave: bool, holiday: Option<&str>) -> DaySchedule {
        DaySchedule {
            date: thu() + TimeDelta::days(offset),
            working,
            expected_minutes: minutes,
            on_leave,
            holiday: holiday.map(str::to_owned),
        }
    }

    #[test]
    fn from_只留今天之後沒有上班時段的日子() {
        let days = [
            day(0, vec![], 0, true, None),      // 今天：不記
            day(1, normal(), 480, false, None), // 上班
            day(2, vec![], 0, false, None),     // 週六
            day(4, vec![], 0, true, None),      // 請假
            day(5, vec![], 0, false, Some("國慶日")),
            day(6, normal(), 240, true, None), // 請半天：還是要上班
        ];
        let expected: IndexMap<String, String> = [
            ("2026-10-31".to_owned(), String::new()),
            ("2026-11-02".to_owned(), "請假".to_owned()),
            ("2026-11-03".to_owned(), "國慶日".to_owned()),
        ]
        .into_iter()
        .collect();
        assert_eq!(expected, OffDayCache::from(&days, thu()));
    }

    #[test]
    fn today_還原成沒有上班時段的班表() {
        let off: IndexMap<String, String> = [
            ("2026-10-29".to_owned(), "請假".to_owned()),
            ("2026-10-30".to_owned(), "國慶日".to_owned()),
            ("2026-10-31".to_owned(), String::new()),
        ]
        .into_iter()
        .collect();

        let leave = OffDayCache::today(&off, thu()).unwrap();
        assert_eq!(thu(), leave.date);
        assert!(leave.working.is_empty());
        assert!(leave.on_leave);
        assert_eq!(Some("國慶日"), OffDayCache::today(&off, thu() + TimeDelta::days(1)).unwrap().holiday.as_deref());
        let weekend = OffDayCache::today(&off, thu() + TimeDelta::days(2)).unwrap();
        assert!(weekend.working.is_empty());
        assert!(!weekend.on_leave);
        assert_eq!(None, weekend.holiday);
        assert!(OffDayCache::today(&off, thu() + TimeDelta::days(3)).is_none());
    }
}
