//! 對應 C# `UI/DayTimeline.cs`：一天的時間軸。
//!
//! 底色是班表時段，綠色是有計時的時段（並行只算一次），班表內沒計到的空檔標紅，今天會畫出「現在」的位置。
//! 滑鼠移上去會顯示那個時間點在跑哪些任務。
//!
//! C# 是自己畫的 FrameworkElement；這裡 Rust 算資料（[`DayTimelineData`]），畫在 `@hbc` 的 `DayTimeline` 元件。
//! 元件吃一個 `DayTimelineModel` struct（只有基本型別與基本型別陣列）；各 crate 編譯出的 struct 是不同的 Rust 型別，
//! 用 [`day_timeline_model!`](crate::day_timeline_model) 轉成呼叫端自己的型別。

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};
use hb_api::{TimeEntry, WorkMath, WorkSegment};
use slint::{ModelRc, SharedString, VecModel};

type Span = (NaiveDateTime, NaiveDateTime);

/// 一天時間軸的資料（C# `SetData` 算出來的東西）。
#[derive(Debug, Clone, PartialEq)]
pub struct DayTimelineData {
    pub day: NaiveDate,
    pub now: NaiveDateTime,
    /// 顯示範圍：班表與計時段的最早～最晚，取整點；什麼都沒有時 9–18；至少 4 小時。
    pub from: NaiveDateTime,
    pub to: NaiveDateTime,
    pub segments: Vec<Span>,
    pub covered: Vec<Span>,
    pub gaps: Vec<Span>,
    /// 各計時段（任務、那一段的區間），懸停說明用。
    entries: Vec<(String, Vec<Span>)>,
}

/// 給 Slint 的資料（欄位跟 `DayTimelineModel` 一一對應，時間都是「距當天 0 點幾分鐘」）。
pub struct DayTimelineParts {
    pub from_min: f32,
    pub to_min: f32,
    pub from_hour: i32,
    pub hours: i32,
    /// 「現在」的位置；不是今天或不在範圍內是 -1。
    pub now_min: f32,
    pub seg_start: ModelRc<f32>,
    pub seg_end: ModelRc<f32>,
    pub gap_start: ModelRc<f32>,
    pub gap_end: ModelRc<f32>,
    pub run_start: ModelRc<f32>,
    pub run_end: ModelRc<f32>,
    /// 懸停說明（不含前面的「HH:mm」），去重複後的清單。
    pub hover_texts: ModelRc<SharedString>,
    /// 從 from_min 起每一分鐘對到 hover_texts 的第幾個。
    pub hover_index: ModelRc<i32>,
}

fn floor_hours(d: TimeDelta) -> i64 {
    (d.num_seconds() as f64 / 3600.0).floor() as i64
}

fn ceil_hours(d: TimeDelta) -> i64 {
    (d.num_seconds() as f64 / 3600.0).ceil() as i64
}

impl DayTimelineData {
    /// C# `SetData(day, segments, entries, now)`；show_gaps：畫出不足班表的空檔（工時紀錄、懸停摘要用，C# 預設 true）。
    pub fn new(
        day: NaiveDate,
        segments: Option<&[WorkSegment]>,
        entries: &[TimeEntry],
        now: NaiveDateTime,
        show_gaps: bool,
    ) -> Self {
        let day_start = day.and_time(NaiveTime::MIN);
        let segs: Vec<WorkSegment> = segments.map(<[WorkSegment]>::to_vec).unwrap_or_default();
        let covered = WorkMath::covered(entries.iter(), day_start, now);
        let gaps = if show_gaps { WorkMath::gaps(&segs, entries.iter(), day_start, now, 5.0) } else { Vec::new() };
        let segments: Vec<Span> = segs.iter().map(|s| (day.and_time(s.start), day.and_time(s.end))).collect();

        // 範圍：班表與計時段的最早～最晚，取整點；什麼都沒有時顯示 9–18
        let mut points: Vec<NaiveDateTime> = segments.iter().flat_map(|s| [s.0, s.1]).collect();
        points.extend(covered.iter().flat_map(|c| [c.0, c.1]));
        if day == now.date() {
            points.push(now);
        }
        let from_p = points.iter().min().copied().unwrap_or(day_start + TimeDelta::hours(9));
        let to_p = points.iter().max().copied().unwrap_or(day_start + TimeDelta::hours(18));
        let from = day_start + TimeDelta::hours(floor_hours(from_p - day_start));
        let mut to = day_start + TimeDelta::hours(ceil_hours(to_p - day_start).min(24));
        if to - from < TimeDelta::hours(4) {
            to = from + TimeDelta::hours(4);
        }
        let entries =
            entries.iter().map(|e| (e.task.clone(), WorkMath::intervals(std::iter::once(e), day_start, now))).collect();
        DayTimelineData { day, now, from, to, segments, covered, gaps, entries }
    }

    fn day_start(&self) -> NaiveDateTime {
        self.day.and_time(NaiveTime::MIN)
    }

    /// 懸停說明的後半段（「  計時中：A、B」／「  班表時段，沒有計時」／「  沒有計時」）。
    fn describe_suffix(&self, t: NaiveDateTime) -> String {
        let mut tasks: Vec<&str> = Vec::new();
        for (task, spans) in &self.entries {
            if spans.iter().any(|(s, e)| *s <= t && t < *e) && !tasks.iter().any(|k| k.eq_ignore_ascii_case(task)) {
                tasks.push(task);
            }
        }
        if !tasks.is_empty() {
            return format!("  計時中：{}", tasks.join("、"));
        }
        if self.gaps.iter().any(|(s, e)| *s <= t && t < *e) {
            return "  班表時段，沒有計時".to_owned();
        }
        "  沒有計時".to_owned()
    }

    /// 滑鼠在 t 時的說明（C# `Describe`）。
    pub fn describe(&self, t: NaiveDateTime) -> String {
        format!("{}{}", t.format("%H:%M"), self.describe_suffix(t))
    }

    fn minutes(&self, t: NaiveDateTime) -> f32 {
        ((t - self.day_start()).num_milliseconds() as f64 / 60000.0) as f32
    }

    pub fn parts(&self) -> DayTimelineParts {
        let floats = |spans: &[Span], end: bool| -> ModelRc<f32> {
            ModelRc::new(VecModel::from(
                spans.iter().map(|s| self.minutes(if end { s.1 } else { s.0 })).collect::<Vec<_>>(),
            ))
        };
        let total = ((self.to - self.from).num_minutes()).max(0);
        let mut texts: Vec<String> = Vec::new();
        let mut index: Vec<i32> = Vec::with_capacity(total as usize + 1);
        for k in 0..=total {
            let suffix = self.describe_suffix(self.from + TimeDelta::minutes(k));
            let i = match texts.iter().position(|s| *s == suffix) {
                Some(i) => i,
                None => {
                    texts.push(suffix);
                    texts.len() - 1
                }
            };
            index.push(i as i32);
        }
        let today = self.day == self.now.date() && self.now >= self.from && self.now <= self.to;
        DayTimelineParts {
            from_min: self.minutes(self.from),
            to_min: self.minutes(self.to),
            from_hour: floor_hours(self.from - self.day_start()) as i32,
            hours: ((self.to - self.from).num_seconds() as f64 / 3600.0).round() as i32,
            now_min: if today { self.minutes(self.now) } else { -1.0 },
            seg_start: floats(&self.segments, false),
            seg_end: floats(&self.segments, true),
            gap_start: floats(&self.gaps, false),
            gap_end: floats(&self.gaps, true),
            run_start: floats(&self.covered, false),
            run_end: floats(&self.covered, true),
            hover_texts: ModelRc::new(VecModel::from(texts.into_iter().map(SharedString::from).collect::<Vec<_>>())),
            hover_index: ModelRc::new(VecModel::from(index)),
        }
    }

    /// 刻度的小時文字（C#：0 點且不是當天開頭時寫 24）。給測試用；Slint 端同一套算法。
    pub fn tick_label(&self, hour_offset: i32) -> String {
        let t = self.from + TimeDelta::hours(hour_offset as i64);
        if t.hour() == 0 && t > self.day_start() { "24".to_owned() } else { t.hour().to_string() }
    }
}

/// 把 [`DayTimelineData`] 轉成呼叫端 crate 自己編譯出的 `DayTimelineModel`（欄位一致）。
///
/// ```ignore
/// // 呼叫端的 .slint：import { DayTimeline, DayTimelineModel } from "@hbc/index.slint";
/// //                   export { DayTimelineModel }   // 讓 Rust 拿得到型別（或當成某個 in property 的型別）
/// let data = hb_ui_common::DayTimelineData::new(day, segments, &entries, now, true);
/// ui.set_timeline(hb_ui_common::day_timeline_model!(DayTimelineModel, data));
/// ```
#[macro_export]
macro_rules! day_timeline_model {
    ($ty:path, $data:expr) => {{
        let p = $crate::day_timeline::DayTimelineData::parts(&$data);
        $ty {
            from_min: p.from_min,
            to_min: p.to_min,
            from_hour: p.from_hour,
            hours: p.hours,
            now_min: p.now_min,
            seg_start: p.seg_start,
            seg_end: p.seg_end,
            gap_start: p.gap_start,
            gap_end: p.gap_end,
            run_start: p.run_start,
            run_end: p.run_end,
            hover_texts: p.hover_texts,
            hover_index: p.hover_index,
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use slint::Model;

    fn at(day: NaiveDate, h: u32, m: u32) -> NaiveDateTime {
        day.and_hms_opt(h, m, 0).unwrap()
    }

    fn entry(task: &str, start: NaiveDateTime, end: Option<NaiveDateTime>) -> TimeEntry {
        let off = |t: NaiveDateTime| Local.from_local_datetime(&t).unwrap().fixed_offset();
        TimeEntry {
            id: task.to_owned(),
            task: task.to_owned(),
            title: String::new(),
            started_at: Some(off(start)),
            ended_at: end.map(off),
            duration_minutes: None,
            running: end.is_none(),
            stop_reason: None,
            note: None,
            user: None,
            source: None,
        }
    }

    fn seg(h1: u32, h2: u32) -> WorkSegment {
        WorkSegment {
            start: NaiveTime::from_hms_opt(h1, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(h2, 0, 0).unwrap(),
        }
    }

    #[test]
    fn 範圍_沒有資料時_9到18() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        let d = DayTimelineData::new(day, None, &[], at(NaiveDate::from_ymd_opt(2026, 11, 12).unwrap(), 10, 0), true);
        assert_eq!((at(day, 9, 0), at(day, 18, 0)), (d.from, d.to));
        let p = d.parts();
        assert_eq!((540.0, 1080.0, 9, 9, -1.0), (p.from_min, p.to_min, p.from_hour, p.hours, p.now_min));
    }

    #[test]
    fn 範圍_取整點且至少4小時_今天含現在() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let d = DayTimelineData::new(
            day,
            None,
            &[entry("A-1", at(day, 9, 20), Some(at(day, 9, 40)))],
            at(day, 10, 15),
            true,
        );
        assert_eq!((at(day, 9, 0), at(day, 13, 0)), (d.from, d.to));
        assert_eq!(615.0, d.parts().now_min);
    }

    #[test]
    fn 懸停說明_計時中_空檔_沒有計時() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let entries = [
            entry("A-1", at(day, 9, 0), Some(at(day, 10, 0))),
            entry("a-1", at(day, 9, 30), Some(at(day, 9, 45))),
            entry("B-2", at(day, 9, 30), Some(at(day, 11, 0))),
        ];
        let d = DayTimelineData::new(day, Some(&[seg(9, 12), seg(13, 18)]), &entries, at(day, 17, 0), true);
        assert_eq!("09:35  計時中：A-1、B-2", d.describe(at(day, 9, 35)));
        assert_eq!("11:30  班表時段，沒有計時", d.describe(at(day, 11, 30)));
        assert_eq!("12:30  沒有計時", d.describe(at(day, 12, 30)));
        let p = d.parts();
        // 9–18 每分鐘一筆
        assert_eq!(9 * 60 + 1, p.hover_index.row_count());
        let text = |m: usize| p.hover_texts.row_data(p.hover_index.row_data(m).unwrap() as usize).unwrap();
        assert_eq!("  計時中：A-1、B-2", text(35).as_str());
        assert_eq!(vec![540.0, 780.0], p.seg_start.iter().collect::<Vec<_>>());
        assert_eq!(vec![540.0], p.run_start.iter().collect::<Vec<_>>());
        assert_eq!(vec![660.0], p.run_end.iter().collect::<Vec<_>>());
    }

    #[test]
    fn 刻度_午夜寫24() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let d = DayTimelineData::new(
            day,
            None,
            &[entry("A-1", at(day, 21, 0), Some(at(day, 23, 30)))],
            at(day + TimeDelta::days(1), 9, 0),
            false,
        );
        // 23:30 取整到 24 點，不到 4 小時再延到 25 點（隔天 1 點）
        assert_eq!((at(day, 21, 0), at(day, 21, 0) + TimeDelta::hours(4)), (d.from, d.to));
        assert_eq!("21", d.tick_label(0));
        assert_eq!("24", d.tick_label(3));
        assert_eq!("1", d.tick_label(4));
    }
}
