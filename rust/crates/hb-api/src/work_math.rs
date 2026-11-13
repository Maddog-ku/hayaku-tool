//! 工時計算（班表時段、計時聯集、空檔、工時文字、上班階段）。對應 C# `Core/WorkMath.cs`。
//!
//! C# 的 `DateTime`（本地時間）在這裡是 `NaiveDateTime`；`TimeEntry` 的 `DateTimeOffset` 先換成本地時間再算。

use chrono::{Local, NaiveDateTime, NaiveTime, TimeDelta};

use crate::json::round_away;
use crate::models::{DaySchedule, TimeEntry, WorkSegment};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DayPhaseKind {
    Off,
    Before,
    Working,
    Break,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayPhase {
    pub kind: DayPhaseKind,
    pub text: String,
}

impl DayPhase {
    fn new(kind: DayPhaseKind, text: impl Into<String>) -> Self {
        Self { kind, text: text.into() }
    }
}

const SEGMENT_SEPARATORS: [char; 4] = ['、', ',', '，', ';'];
const RANGE_SEPARATORS: [char; 4] = ['–', '—', '-', '~'];

/// TimeSpan.TotalMinutes。
pub fn total_minutes(d: TimeDelta) -> f64 {
    match d.num_nanoseconds() {
        Some(ns) => ns as f64 / 60_000_000_000.0,
        None => d.num_milliseconds() as f64 / 60_000.0,
    }
}

pub struct WorkMath;

impl WorkMath {
    /// 解析 Hayaku 的時段文字，例如「08:30–12:00、13:00–17:30」；「不上班」回傳空集合。
    pub fn parse_segments(text: Option<&str>) -> Vec<WorkSegment> {
        let mut list = Vec::new();
        let Some(text) = text.filter(|t| !crate::json::is_blank(t)) else {
            return list;
        };
        for part in text.split(SEGMENT_SEPARATORS).map(str::trim).filter(|p| !p.is_empty()) {
            let range: Vec<&str> = part.split(RANGE_SEPARATORS).map(str::trim).collect();
            if range.len() == 2
                && let (Some(start), Some(end)) = (try_time(range[0]), try_time(range[1]))
                && end > start
            {
                list.push(WorkSegment { start, end });
            }
        }
        list.sort_by_key(|a| a.start);
        list
    }

    /// 某天的實際工時（分鐘）：把所有計時段聯集起來，重疊的時間只算一次。
    /// 進行中的計時算到 now；沒有起訖時間的段落（手動補登）直接加上時數。
    pub fn union_minutes<'a>(
        entries: impl IntoIterator<Item = &'a TimeEntry>,
        day: NaiveDateTime,
        now: NaiveDateTime,
    ) -> f64 {
        let entries: Vec<&TimeEntry> = entries.into_iter().collect();
        let extra: i32 =
            entries.iter().filter(|e| e.started_at.is_none()).map(|e| e.duration_minutes.unwrap_or(0)).sum();
        Self::covered(entries.iter().copied(), day, now).iter().map(|(s, e)| total_minutes(*e - *s)).sum::<f64>()
            + extra as f64
    }

    /// 某天各計時段（裁到當天、進行中算到 now）；沒有起訖時間的段落不算。
    pub fn intervals<'a>(
        entries: impl IntoIterator<Item = &'a TimeEntry>,
        day: NaiveDateTime,
        now: NaiveDateTime,
    ) -> Vec<(NaiveDateTime, NaiveDateTime)> {
        let day_start = day.date().and_time(NaiveTime::MIN);
        let day_end = day_start + TimeDelta::days(1);
        let mut list = Vec::new();
        for e in entries {
            let Some(started) = e.started_at else { continue };
            let mut s = started.with_timezone(&Local).naive_local();
            let mut end = if e.running {
                now
            } else {
                match e.ended_at {
                    Some(ended) => ended.with_timezone(&Local).naive_local(),
                    None => match e.duration_minutes {
                        Some(d) => s + TimeDelta::minutes(d as i64),
                        None => s,
                    },
                }
            };
            if s < day_start {
                s = day_start;
            }
            if end > day_end {
                end = day_end;
            }
            if end > now {
                end = now;
            }
            if end > s {
                list.push((s, end));
            }
        }
        list.sort_by_key(|a| a.0);
        list
    }

    /// 某天有在計時的時段（所有計時段的聯集，依時間排序、不重疊）。
    pub fn covered<'a>(
        entries: impl IntoIterator<Item = &'a TimeEntry>,
        day: NaiveDateTime,
        now: NaiveDateTime,
    ) -> Vec<(NaiveDateTime, NaiveDateTime)> {
        let mut merged: Vec<(NaiveDateTime, NaiveDateTime)> = Vec::new();
        for (s, e) in Self::intervals(entries, day, now) {
            match merged.last_mut() {
                Some(last) if s <= last.1 => {
                    if e > last.1 {
                        last.1 = e;
                    }
                }
                _ => merged.push((s, e)),
            }
        }
        merged
    }

    /// 班表時段裡沒有任何計時的空檔（到 now 為止、至少 min_minutes 分鐘；C# 預設 5）。
    pub fn gaps<'a>(
        segments: &[WorkSegment],
        entries: impl IntoIterator<Item = &'a TimeEntry>,
        day: NaiveDateTime,
        now: NaiveDateTime,
        min_minutes: f64,
    ) -> Vec<(NaiveDateTime, NaiveDateTime)> {
        let covered = Self::covered(entries, day, now);
        let mut gaps = Vec::new();
        for seg in segments {
            let mut cursor = day.date().and_time(seg.start);
            let mut seg_end = day.date().and_time(seg.end);
            if seg_end > now {
                seg_end = now;
            }
            for &(s, e) in &covered {
                if e <= cursor || s >= seg_end {
                    continue;
                }
                if s > cursor {
                    gaps.push((cursor, s));
                }
                cursor = e;
            }
            if cursor < seg_end {
                gaps.push((cursor, seg_end));
            }
        }
        gaps.into_iter().filter(|(s, e)| total_minutes(*e - *s) >= min_minutes).collect()
    }

    /// 已填有沒有滿應出勤（跟 Hayaku 同一套）：都算到分再比，差不到半分鐘就算填滿。
    /// 補工時提醒、月曆、統計、下班提示都用這個，不會出現「還差 0 分」卻照樣提醒。
    pub fn is_filled(worked: f64, expected: f64) -> bool {
        expected <= 0.0 || expected - worked < 0.5
    }

    /// 還差幾分（填滿了是 0）。
    pub fn shortfall(worked: f64, expected: f64) -> f64 {
        if Self::is_filled(worked, expected) { 0.0 } else { expected - worked }
    }

    pub fn task_minutes<'a>(
        entries: impl IntoIterator<Item = &'a TimeEntry>,
        task: &str,
        day: NaiveDateTime,
        now: NaiveDateTime,
    ) -> f64 {
        let task = task.to_uppercase();
        Self::union_minutes(entries.into_iter().filter(|e| e.task.to_uppercase() == task), day, now)
    }

    /// 工時（跟 Hayaku 網頁一樣：先捨去不滿一分鐘的秒數，再換成小時到小數點後兩位）：
    /// 41.6 分 → "0.68h"；364.8 分 → "6.07h"；8 小時 → "8h"。
    pub fn hours_text(minutes: f64) -> String {
        format!("{}h", format_trimmed(round_away(minutes.floor() / 60.0, 2), 2))
    }

    /// 小時數只顯示到小數點後一位（計算時保留原本的精確度）：2.13 → "2.1"、4 → "4"。
    pub fn hours_number(hours: f64) -> String {
        format_trimmed(round_away(hours, 1), 1)
    }

    /// "1:23:45"
    pub fn clock(minutes: f64) -> String {
        // TimeSpan.FromMinutes：換成 tick（100 奈秒）後捨去
        let ticks = (minutes.max(0.0) * 600_000_000.0) as i64;
        let total_seconds = ticks / 10_000_000;
        format!("{}:{:02}:{:02}", total_seconds / 3600, (total_seconds / 60) % 60, total_seconds % 60)
    }

    /// "3 小時 27 分"
    pub fn hours_minutes(minutes: f64) -> String {
        let total = minutes.max(0.0).round_ties_even() as i64;
        let (h, m) = (total / 60, total % 60);
        if h == 0 {
            format!("{m} 分")
        } else if m == 0 {
            format!("{h} 小時")
        } else {
            format!("{h} 小時 {m} 分")
        }
    }

    pub fn phase(schedule: Option<&DaySchedule>, now: NaiveDateTime, clocked_out: bool) -> DayPhase {
        let Some(schedule) = schedule else {
            return DayPhase::new(DayPhaseKind::Off, "");
        };
        if let Some(holiday) = &schedule.holiday {
            return DayPhase::new(DayPhaseKind::Off, format!("今天放假（{holiday}）"));
        }
        let segs = &schedule.working;
        let (Some(first), Some(last)) = (segs.first(), segs.last()) else {
            return DayPhase::new(DayPhaseKind::Off, if schedule.on_leave { "今天請假" } else { "今天不上班" });
        };
        if clocked_out {
            return DayPhase::new(DayPhaseKind::After, "已手動下班");
        }
        let t = now.time();
        if t < first.start {
            return DayPhase::new(DayPhaseKind::Before, format!("{} 上班", first.start.format("%H:%M")));
        }
        if t >= last.end {
            return DayPhase::new(DayPhaseKind::After, "已過下班時間");
        }
        if let Some(current) = segs.iter().find(|s| s.start <= t && t < s.end) {
            return DayPhase::new(DayPhaseKind::Working, format!("上班中 · 到 {}", current.end.format("%H:%M")));
        }
        match segs.iter().find(|s| s.start > t) {
            Some(next) => DayPhase::new(DayPhaseKind::Break, format!("休息中 · {} 繼續", next.start.format("%H:%M"))),
            // C# 用 First()，照理不會走到這裡（t 在第一段開始與最後一段結束之間）
            None => DayPhase::new(DayPhaseKind::After, "已過下班時間"),
        }
    }
}

/// `TimeOnly.TryParseExact(s, ["H:mm", "HH:mm"])`：時 1～2 位、分 2 位。
fn try_time(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.split_once(':')?;
    if h.is_empty()
        || h.len() > 2
        || m.len() != 2
        || !h.bytes().all(|b| b.is_ascii_digit())
        || !m.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    NaiveTime::from_hms_opt(h.parse().ok()?, m.parse().ok()?, 0)
}

/// `ToString("0.##")` 這類格式：最多 digits 位小數，去掉尾巴的 0。
fn format_trimmed(x: f64, digits: usize) -> String {
    let s = format!("{x:.digits$}");
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    if s == "-0" { "0".to_owned() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DateTimeOffset;
    use chrono::{NaiveDate, TimeZone};

    // 對應 WorkMathTests 與 TestingTests.WorkMath_IsFilled
    fn local(t: NaiveDateTime) -> DateTimeOffset {
        Local.from_local_datetime(&t).earliest().expect("local time").fixed_offset()
    }

    fn entry(task: &str, start: NaiveDateTime, end: Option<NaiveDateTime>, running: bool) -> TimeEntry {
        TimeEntry::new(crate::json::new_guid(), task, task, Some(local(start)), end.map(local), None, running, None)
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).expect("date").and_time(NaiveTime::MIN)
    }

    fn hours(d: NaiveDateTime, h: f64) -> NaiveDateTime {
        d + TimeDelta::milliseconds((h * 3_600_000.0) as i64)
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).expect("time")
    }

    fn assert_close(expected: f64, actual: f64) {
        assert!((expected - actual).abs() < 0.0005, "expected {expected}, got {actual}");
    }

    #[test]
    fn parse_segments_讀得懂hayaku的時段格式() {
        let segs = WorkMath::parse_segments(Some("08:30–12:00、13:00–17:30"));
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].start, t(8, 30));
        assert_eq!(segs[1].end, t(17, 30));
    }

    #[test]
    fn parse_segments_不上班是空的() {
        for text in [Some("不上班"), Some(""), None] {
            assert!(WorkMath::parse_segments(text).is_empty());
        }
    }

    #[test]
    fn union_minutes_並行計時只算一次() {
        let d = day(2026, 10, 28);
        let now = hours(d, 10.0);
        let entries: Vec<TimeEntry> =
            (0..6).map(|i| entry(&format!("F2P-{i}"), hours(d, 8.0) + TimeDelta::minutes(30), None, true)).collect();
        assert_close(90.0, WorkMath::union_minutes(&entries, d, now));
    }

    #[test]
    fn union_minutes_午休空檔不算() {
        let d = day(2026, 10, 27);
        let now = hours(d, 20.0);
        let entries = [
            entry("A", hours(d, 8.5), Some(hours(d, 12.0)), false),
            entry("B", hours(d, 8.5), Some(hours(d, 12.0)), false),
            entry("A", hours(d, 13.0), Some(hours(d, 17.5)), false),
            entry("C", hours(d, 16.0), Some(hours(d, 18.0)), false),
        ];
        // 08:30–12:00 (210) + 13:00–18:00 (300)
        assert_close(510.0, WorkMath::union_minutes(&entries, d, now));
        assert_close(480.0, WorkMath::task_minutes(&entries, "A", d, now));
    }

    #[test]
    fn union_minutes_跨日的計時只算今天的部分() {
        let d = day(2026, 10, 28);
        let entries = [entry("A", hours(d, -1.0), None, true)];
        assert_close(60.0, WorkMath::union_minutes(&entries, d, hours(d, 1.0)));
    }

    #[test]
    fn hours_text_格式() {
        for (minutes, expected) in
            [(480.0, "8h"), (208.0, "3.47h"), (364.8, "6.07h"), (41.6, "0.68h"), (366.0, "6.1h"), (0.0, "0h")]
        {
            assert_eq!(WorkMath::hours_text(minutes), expected, "{minutes}");
        }
    }

    #[test]
    fn phase_判斷上班午休下班() {
        let schedule = DaySchedule {
            date: NaiveDate::from_ymd_opt(2026, 10, 28).expect("date"),
            working: WorkMath::parse_segments(Some("08:30–12:00、13:00–17:30")),
            expected_minutes: 480,
            on_leave: false,
            holiday: None,
        };
        let d = day(2026, 10, 28);
        assert_eq!(WorkMath::phase(Some(&schedule), hours(d, 8.0), false).kind, DayPhaseKind::Before);
        assert_eq!(WorkMath::phase(Some(&schedule), hours(d, 9.0), false).kind, DayPhaseKind::Working);
        assert_eq!(WorkMath::phase(Some(&schedule), hours(d, 12.5), false).kind, DayPhaseKind::Break);
        assert_eq!(WorkMath::phase(Some(&schedule), hours(d, 18.0), false).kind, DayPhaseKind::After);
        assert_eq!(WorkMath::phase(Some(&schedule), hours(d, 9.0), true).kind, DayPhaseKind::After);
    }

    #[test]
    fn hours_number_只顯示到小數點後一位() {
        for (h, expected) in [(2.13, "2.1"), (3.58, "3.6"), (4.0, "4"), (2.25, "2.3"), (0.04, "0")] {
            assert_eq!(WorkMath::hours_number(h), expected, "{h}");
        }
    }

    #[test]
    fn covered_重疊的計時合併成一段() {
        let d = day(2026, 10, 28);
        let entries = [
            entry("A", hours(d, 9.0), Some(hours(d, 11.0)), false),
            entry("B", hours(d, 10.0), Some(hours(d, 12.0)), false),
            entry("C", hours(d, 13.0), Some(hours(d, 14.0)), false),
        ];
        let covered = WorkMath::covered(&entries, d, hours(d, 20.0));
        assert_eq!(covered, vec![(hours(d, 9.0), hours(d, 12.0)), (hours(d, 13.0), hours(d, 14.0))]);
    }

    #[test]
    fn gaps_只列班表內沒計到的空檔且算到現在() {
        let d = day(2026, 10, 28);
        let segs = WorkMath::parse_segments(Some("08:30–12:00、13:00–17:30"));
        let entries = [
            entry("A", hours(d, 9.0), Some(hours(d, 11.5)), false),
            entry("B", hours(d, 11.5) + TimeDelta::minutes(2), Some(hours(d, 12.0)), false), // 2 分鐘的縫不算
            entry("C", hours(d, 12.0), Some(hours(d, 13.5)), false),                         // 午休的計時不影響
        ];
        let gaps = WorkMath::gaps(&segs, &entries, d, hours(d, 15.0), 5.0);
        assert_eq!(gaps, vec![(hours(d, 8.5), hours(d, 9.0)), (hours(d, 13.5), hours(d, 15.0))]);
    }

    #[test]
    fn is_filled_差不到半分鐘就算填滿() {
        for (worked, expected, filled) in [
            (479.6, 480.0, true),
            (479.5, 480.0, false),
            (479.2, 480.0, false),
            (478.8, 480.0, false),
            (0.0, 0.0, true),
        ] {
            assert_eq!(WorkMath::is_filled(worked, expected), filled, "{worked}");
            let want = if filled { 0.0 } else { expected - worked };
            assert!((WorkMath::shortfall(worked, expected) - want).abs() < 0.0005);
        }
    }

    #[test]
    fn 時鐘與時分文字() {
        assert_eq!(WorkMath::clock(83.75), "1:23:45");
        assert_eq!(WorkMath::clock(-5.0), "0:00:00");
        assert_eq!(WorkMath::hours_minutes(207.0), "3 小時 27 分");
        assert_eq!(WorkMath::hours_minutes(120.0), "2 小時");
        assert_eq!(WorkMath::hours_minutes(45.0), "45 分");
        assert_eq!(WorkMath::hours_minutes(0.5), "0 分"); // 銀行家捨入
    }
}
