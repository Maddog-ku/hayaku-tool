//! 對應 C# `Eip/LeavePlanner.cs`：EIP 假單 → Hayaku 請假的規則（純函式，方便測試）。

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Weekday};
use hb_api::{DaySchedule, WorkSegment};
use serde_json::{Map, Value};

use crate::eip_client::EipLeave;

/// 要在 Hayaku 登記的一筆請假：DAY 是整天（可跨天），RANGE 是某一天的指定時段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeavePiece {
    pub span: String,
    pub date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub start: Option<NaiveTime>,
    pub end: Option<NaiveTime>,
}

/// zh-TW 的 `ToString("M/d（ddd）")`：10/30（週五）。
pub(crate) fn zh_month_day(d: NaiveDate) -> String {
    const DAYS: [&str; 7] = ["週一", "週二", "週三", "週四", "週五", "週六", "週日"];
    format!("{}/{}（{}）", d.month(), d.day(), DAYS[d.weekday().num_days_from_monday() as usize])
}

/// C# `s[..n]`（以 UTF-16 單位計）。
fn left_utf16(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

fn hh_mm(t: Option<NaiveTime>) -> String {
    t.map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

impl LeavePiece {
    pub fn new(
        span: impl Into<String>,
        date: NaiveDate,
        end_date: Option<NaiveDate>,
        start: Option<NaiveTime>,
        end: Option<NaiveTime>,
    ) -> Self {
        Self { span: span.into(), date, end_date, start, end }
    }

    /// 整天（end_date 是跨天的最後一天）。
    pub fn day(date: NaiveDate, end_date: Option<NaiveDate>) -> Self {
        Self::new("DAY", date, end_date, None, None)
    }

    /// 某一天的指定時段。
    pub fn range(date: NaiveDate, start: NaiveTime, end: NaiveTime) -> Self {
        Self::new("RANGE", date, None, Some(start), Some(end))
    }

    pub fn from(&self) -> NaiveDateTime {
        if self.span == "DAY" {
            self.date.and_time(NaiveTime::MIN)
        } else {
            self.date.and_time(self.start.unwrap_or(NaiveTime::MIN))
        }
    }

    pub fn to(&self) -> NaiveDateTime {
        if self.span == "DAY" {
            (self.end_date.unwrap_or(self.date) + TimeDelta::days(1)).and_time(NaiveTime::MIN)
        } else {
            self.date.and_time(self.end.unwrap_or(NaiveTime::MIN))
        }
    }

    /// add_leave 的參數。
    pub fn args(&self, note: &str) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert("span".into(), Value::String(self.span.clone()));
        args.insert("date".into(), Value::String(self.date.format("%Y-%m-%d").to_string()));
        if let Some(end) = self.end_date
            && end > self.date
        {
            args.insert("endDate".into(), Value::String(end.format("%Y-%m-%d").to_string()));
        }
        if self.span == "RANGE" {
            args.insert("start".into(), Value::String(hh_mm(self.start)));
            args.insert("end".into(), Value::String(hh_mm(self.end)));
        }
        if !note.is_empty() {
            args.insert("note".into(), Value::String(left_utf16(note, 500).to_owned()));
        }
        args
    }

    /// 11/3（一）整天、11/3（一）到 11/5（三）整天、11/3（一）13:30–17:30。
    pub fn describe(&self) -> String {
        let first = zh_month_day(self.date);
        if self.span == "RANGE" {
            return format!("{first} {}–{}", hh_mm(self.start), hh_mm(self.end));
        }
        match self.end_date {
            Some(end) if end > self.date => format!("{first} 到 {} 整天", zh_month_day(end)),
            _ => format!("{first} 整天"),
        }
    }
}

/// 要同步的性質：請假、補假（銷假用來抵掉請假，黃卡不是請假）。
const LEAVE_KINDS: [&str; 2] = ["請假", "補假"];

/// 上班前那次從第一段上班前多久開始。
pub const MORNING_LEAD_MINUTES: i64 = 15;

/// 最多往回補幾天。
pub const BACKFILL_DAYS: i64 = 7;

/// `DateOnly.TryParseExact(text, "yyyy-MM-dd")`。
fn parse_day(text: &str) -> Option<NaiveDate> {
    let b = text.as_bytes();
    let ok = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if !ok {
        return None;
    }
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub struct LeavePlanner;

impl LeavePlanner {
    /// 有效的請假：只留請假／補假（同一單號只算一次），被銷假整段蓋掉的不算。
    /// 只銷掉一部分的照原本的請假登記（EIP 的格式沒有樣本，不猜）。
    pub fn effective(rows: &[EipLeave]) -> Vec<EipLeave> {
        let cancels: Vec<&EipLeave> = rows.iter().filter(|r| r.kind == "銷假").collect();
        let mut seen: Vec<&str> = Vec::new();
        let mut list: Vec<EipLeave> = Vec::new();
        for r in rows {
            if !LEAVE_KINDS.contains(&r.kind.as_str()) || r.no.is_empty() || seen.contains(&r.no.as_str()) {
                continue;
            }
            seen.push(&r.no);
            if cancels.iter().any(|c| c.start <= r.start && c.end >= r.end) {
                continue;
            }
            list.push(r.clone());
        }
        list.sort_by_key(|r| r.start);
        list
    }

    /// after 之後第一個要上班的日子（有上班時段，或已經請假的工作日）；班表範圍內找不到回 None。
    pub fn next_workday(days: &[DaySchedule], after: NaiveDate) -> Option<NaiveDate> {
        days.iter()
            .filter(|d| d.date > after && d.holiday.is_none() && (!d.working.is_empty() || d.on_leave))
            .map(|d| d.date)
            .min()
    }

    /// 某一天的上班時段：班表有就用班表；已經請假的日子班表可能是空的，改用平常的時段；
    /// 假日、放假是空的；班表範圍外的平日用平常的時段，週末是空的。
    pub fn working(days: &[DaySchedule]) -> impl Fn(NaiveDate) -> Vec<WorkSegment> + 'static {
        let days = days.to_vec();
        let typical: Vec<WorkSegment> =
            days.iter().find(|d| !d.working.is_empty()).map(|d| d.working.clone()).unwrap_or_default();
        move |date| match days.iter().find(|d| d.date == date) {
            None => {
                if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                    Vec::new()
                } else {
                    typical.clone()
                }
            }
            Some(s) if !s.working.is_empty() => s.working.clone(),
            Some(s) => {
                if s.on_leave && s.holiday.is_none() {
                    typical.clone()
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// 把一張假單換成 Hayaku 的請假：涵蓋當天整個上班時段的算整天，連續的整天合併成一筆（中間的假日不打斷）；
    /// 其他用指定時段。不用上班的日子、完全落在上班時段外的部分略過。
    pub fn plan(leave: &EipLeave, working: impl Fn(NaiveDate) -> Vec<WorkSegment>) -> Vec<LeavePiece> {
        let mut pieces = Vec::new();
        let mut run: Option<(NaiveDate, NaiveDate)> = None;

        fn flush(pieces: &mut Vec<LeavePiece>, run: &mut Option<(NaiveDate, NaiveDate)>) {
            if let Some((s, e)) = run.take() {
                pieces.push(LeavePiece::day(s, (e > s).then_some(e)));
            }
        }

        // leave.End.AddTicks(-1)
        let last = (leave.end - TimeDelta::nanoseconds(100)).date();
        let mut day = leave.start.date();
        while day <= last {
            let day_start = day.and_time(NaiveTime::MIN);
            let next_day = day_start + TimeDelta::days(1);
            let from = if leave.start > day_start { leave.start } else { day_start };
            let to = if leave.end < next_day { leave.end } else { next_day };
            let segs = working(day);
            if to > from
                && let (Some(first), Some(last_seg)) = (segs.first(), segs.last())
            {
                let work_start = day.and_time(first.start);
                let work_end = day.and_time(last_seg.end);
                if from <= work_start && to >= work_end {
                    run = Some(match run {
                        Some((s, _)) => (s, day),
                        None => (day, day),
                    });
                } else if !(to <= work_start || from >= work_end) {
                    flush(&mut pieces, &mut run);
                    // 跨天假單的頭尾那天會延伸到午夜，裁到當天的上班時間
                    let start = if from > work_start { from } else { work_start };
                    let end = if to < work_end { to } else { work_end };
                    pieces.push(LeavePiece::range(day, start.time(), end.time()));
                }
            }
            // 不用上班的日子：不打斷連續的整天
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        flush(&mut pieces, &mut run);
        pieces
    }

    pub fn overlaps(a_from: NaiveDateTime, a_to: NaiveDateTime, b_from: NaiveDateTime, b_to: NaiveDateTime) -> bool {
        a_from < b_to && b_from < a_to
    }

    /// 現在要不要做下班後的工作（一天一次）：今天要上班（請假、放假、週末一律不做）、今天還沒做過，
    /// 而且已經下班（班表下班時間到、手動按了下班），或從沒做過、昨天以前就漏了（那天電腦沒開到下班）。
    pub fn is_due(today: &DaySchedule, clocked_out: bool, now: NaiveDateTime, last_sync_day: &str) -> bool {
        let date = now.date();
        let Some(last_seg) = today.working.last() else { return false };
        if last_sync_day == iso(date) {
            return false;
        }
        match parse_day(last_sync_day) {
            Some(last) if last >= date - TimeDelta::days(1) => {}
            _ => return true,
        }
        if clocked_out {
            return true;
        }
        now >= date.and_time(last_seg.end)
    }

    /// 現在要不要做上班前那次（一天一次，抓早上才在 EIP 臨時請的假）：今天要上班、今天還沒做過、還沒下班，
    /// 第一段上班前 15 分鐘起（開機已經過了上班時間就馬上做）；下班後交給下班那次。
    pub fn is_morning_due(today: &DaySchedule, clocked_out: bool, now: NaiveDateTime, morning_day: &str) -> bool {
        let date = now.date();
        let (Some(first), Some(last)) = (today.working.first(), today.working.last()) else { return false };
        if clocked_out || morning_day == iso(date) {
            return false;
        }
        now >= date.and_time(first.start) - TimeDelta::minutes(MORNING_LEAD_MINUTES) && now < date.and_time(last.end)
    }

    /// 這次從哪一天開始找要登記的假單：上次自動同步算到的那天（last_sync_day）的隔天，
    /// 中間漏掉的日子（例如臨時請整天假、那天電腦沒開）一起補登；最多往回 7 天，從沒同步過就從今天。
    pub fn backfill_from(last_sync_day: &str, today: NaiveDate) -> NaiveDate {
        let Some(last) = parse_day(last_sync_day).filter(|l| *l < today) else { return today };
        let from = last + TimeDelta::days(1);
        let earliest = today - TimeDelta::days(BACKFILL_DAYS);
        if from < earliest { earliest } else { from }
    }

    /// 這次做完算哪一天的（存成 last_sync_day）：已經下班就是今天；上班前或上班中補做的只算昨天，
    /// 今天下班時照樣再做一次（白天才在 EIP 請的假）。
    pub fn covered_day(today: &DaySchedule, clocked_out: bool, now: NaiveDateTime) -> NaiveDate {
        let date = now.date();
        let after_work = clocked_out || today.working.last().is_none_or(|last| now >= date.and_time(last.end));
        if after_work { date } else { date - TimeDelta::days(1) }
    }
}
