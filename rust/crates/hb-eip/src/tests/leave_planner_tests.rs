//! 移植自 LeavePlannerTests.cs（CreatedId_支援id或leave物件 已在 hb-api）。

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Weekday};
use hb_api::{DaySchedule, WorkSegment};
use serde_json::{Map, Value, json};

use crate::{EipLeave, LeavePiece, LeavePlanner};

fn t(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn normal() -> Vec<WorkSegment> {
    vec![WorkSegment { start: t(8, 30), end: t(12, 30) }, WorkSegment { start: t(13, 30), end: t(17, 30) }]
}

fn work(d: NaiveDate) -> DaySchedule {
    DaySchedule { date: d, working: normal(), expected_minutes: 480, on_leave: false, holiday: None }
}

fn off(d: NaiveDate, holiday: Option<&str>) -> DaySchedule {
    DaySchedule { date: d, working: vec![], expected_minutes: 0, on_leave: false, holiday: holiday.map(str::to_owned) }
}

fn on_leave(d: NaiveDate) -> DaySchedule {
    DaySchedule { date: d, working: vec![], expected_minutes: 0, on_leave: true, holiday: None }
}

fn dt(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, 0).unwrap()
}

fn leave(no: &str, start: NaiveDateTime, end: NaiveDateTime, kind: &str) -> EipLeave {
    EipLeave::new(no, "Y2798", kind, "特休", "", start, end, "核准", None)
}

// 2026-10-29 是星期四
fn thu() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()
}
fn fri() -> NaiveDate {
    thu() + TimeDelta::days(1)
}
fn mon() -> NaiveDate {
    thu() + TimeDelta::days(4)
}

/// 週四～下下週一的班表：平日上班、週末不上班。
fn two_weeks() -> Vec<DaySchedule> {
    (0..12)
        .map(|i| thu() + TimeDelta::days(i))
        .map(|d| if matches!(d.weekday(), Weekday::Sat | Weekday::Sun) { off(d, None) } else { work(d) })
        .collect()
}

/// C# `DateTime.Parse("2026-10-29 17:00")`。
fn parse(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}

#[test]
fn plan_涵蓋整個上班時段_登記整天() {
    let pieces = LeavePlanner::plan(
        &leave("A", dt(2026, 10, 30, 8, 30), dt(2026, 10, 30, 17, 30), "請假"),
        LeavePlanner::working(&two_weeks()),
    );

    assert_eq!(1, pieces.len());
    let p = &pieces[0];
    assert_eq!(&LeavePiece::day(fri(), None), p);
    assert_eq!(dt(2026, 10, 30, 0, 0), p.from());
    assert_eq!(dt(2026, 10, 31, 0, 0), p.to());
}

#[test]
fn plan_只請幾小時_登記指定時段() {
    let pieces = LeavePlanner::plan(
        &leave("A", dt(2026, 10, 30, 15, 30), dt(2026, 10, 30, 17, 30), "請假"),
        LeavePlanner::working(&two_weeks()),
    );
    assert_eq!(1, pieces.len());
    let p = &pieces[0];

    assert_eq!(&LeavePiece::range(fri(), t(15, 30), t(17, 30)), p);
    assert_eq!(dt(2026, 10, 30, 15, 30), p.from());
    assert_eq!(dt(2026, 10, 30, 17, 30), p.to());
}

#[test]
fn plan_跨週末的整天_合併成一筆() {
    let pieces = LeavePlanner::plan(
        &leave("A", dt(2026, 10, 30, 8, 30), dt(2026, 11, 2, 17, 30), "請假"),
        LeavePlanner::working(&two_weeks()),
    );

    assert_eq!(vec![LeavePiece::day(fri(), Some(mon()))], pieces);
}

#[test]
fn plan_第一天下午開始_拆成時段加整天() {
    let pieces = LeavePlanner::plan(
        &leave("A", dt(2026, 11, 2, 13, 30), dt(2026, 11, 4, 17, 30), "請假"),
        LeavePlanner::working(&two_weeks()),
    );

    assert_eq!(
        vec![
            LeavePiece::range(mon(), t(13, 30), t(17, 30)),
            LeavePiece::day(mon() + TimeDelta::days(1), Some(mon() + TimeDelta::days(2))),
        ],
        pieces
    );
}

#[test]
fn plan_只落在週末_不登記() {
    assert!(
        LeavePlanner::plan(
            &leave("A", dt(2026, 10, 31, 8, 30), dt(2026, 10, 31, 17, 30), "請假"),
            LeavePlanner::working(&two_weeks())
        )
        .is_empty()
    );
}

#[test]
fn working_已請假的日子用平常時段_假日與範圍外週末是空的() {
    let days = vec![work(thu()), on_leave(fri()), off(mon(), Some("國慶日"))];
    let working = LeavePlanner::working(&days);

    assert_eq!(normal(), working(fri()));
    assert!(working(mon()).is_empty());
    assert_eq!(normal(), working(mon() + TimeDelta::days(1))); // 範圍外的平日
    assert!(working(thu() + TimeDelta::days(2)).is_empty()); // 範圍外的週六
}

#[test]
fn effective_只留請假與補假_被銷假蓋掉的不算_同單號一次() {
    let rows = [
        leave("A", dt(2026, 10, 30, 8, 30), dt(2026, 10, 30, 17, 30), "請假"),
        leave("A", dt(2026, 10, 30, 8, 30), dt(2026, 10, 30, 17, 30), "請假"),
        leave("B", dt(2026, 11, 2, 8, 30), dt(2026, 11, 2, 17, 30), "請假"),
        leave("C", dt(2026, 11, 2, 8, 0), dt(2026, 11, 2, 18, 0), "銷假"),
        leave("D", dt(2026, 11, 3, 8, 30), dt(2026, 11, 3, 10, 30), "補假"),
        leave("E", dt(2026, 11, 4, 8, 30), dt(2026, 11, 4, 9, 0), "黃卡"),
    ];

    assert_eq!(vec!["A", "D"], LeavePlanner::effective(&rows).iter().map(|l| l.no.as_str()).collect::<Vec<_>>());
}

#[test]
fn next_workday_週四是週五_週五跳過週末是週一_國定假日也跳過() {
    let mut days = two_weeks();
    assert_eq!(Some(fri()), LeavePlanner::next_workday(&days, thu()));
    assert_eq!(Some(mon()), LeavePlanner::next_workday(&days, fri()));

    let i = days.iter().position(|d| d.date == mon()).unwrap();
    days[i] = off(mon(), Some("補假"));
    assert_eq!(Some(mon() + TimeDelta::days(1)), LeavePlanner::next_workday(&days, fri()));
}

#[test]
fn next_workday_已經請假的工作日也算() {
    assert_eq!(Some(fri()), LeavePlanner::next_workday(&[work(thu()), on_leave(fri())], thu()));
}

#[test]
fn is_due_一天一次_下班時() {
    for (now, last, clocked_out, due) in [
        ("2026-10-29 17:00", "2026-10-28", false, false), // 還沒下班
        ("2026-10-29 17:30", "2026-10-28", false, true),  // 下班時間到
        ("2026-10-29 16:00", "2026-10-28", true, true),   // 提早按了下班
        ("2026-10-29 18:00", "2026-10-29", false, false), // 今天已經同步過
        ("2026-10-29 09:00", "2026-10-27", false, true),  // 前天以前就漏了：開機補做
        ("2026-10-29 09:00", "", false, true),            // 從沒同步過
    ] {
        assert_eq!(
            due,
            LeavePlanner::is_due(&work(thu()), clocked_out, parse(now), last),
            "{now} {last} {clocked_out}"
        );
    }
}

#[test]
fn covered_day_下班後才算今天() {
    for (now, clocked_out, day) in [
        ("2026-10-29 09:00", false, "2026-10-28"), // 上班前補做：算昨天的，下班時還會再做
        ("2026-10-29 17:30", false, "2026-10-29"),
        ("2026-10-29 15:00", true, "2026-10-29"), // 提早按了下班
    ] {
        assert_eq!(
            NaiveDate::parse_from_str(day, "%Y-%m-%d").unwrap(),
            LeavePlanner::covered_day(&work(thu()), clocked_out, parse(now)),
            "{now}"
        );
    }
}

#[test]
fn is_morning_due_上班前15分鐘起一天一次() {
    for (now, morning_day, clocked_out, due) in [
        ("2026-10-29 08:14", "", false, false),           // 上班前 15 分鐘以前還不做
        ("2026-10-29 08:15", "", false, true),            // 上班前 15 分鐘起
        ("2026-10-29 10:00", "2026-10-28", false, true),  // 開機已經過了上班時間：馬上做
        ("2026-10-29 09:00", "2026-10-29", false, false), // 今天做過了
        ("2026-10-29 17:30", "", false, false),           // 下班了：交給下班那次
        ("2026-10-29 10:00", "", true, false),            // 已經按了下班
    ] {
        assert_eq!(due, LeavePlanner::is_morning_due(&work(thu()), clocked_out, parse(now), morning_day), "{now}");
    }
}

#[test]
fn is_morning_due_今天不用上班不做() {
    assert!(!LeavePlanner::is_morning_due(&on_leave(thu()), false, dt(2026, 10, 29, 9, 0), ""));
    assert!(!LeavePlanner::is_morning_due(&off(thu() + TimeDelta::days(2), None), false, dt(2026, 10, 31, 9, 0), ""));
}

#[test]
fn backfill_from_上次同步之後漏掉的日子() {
    for (last, from) in [
        ("2026-11-01", "2026-11-02"), // 昨天做過下班那次：不用補
        ("2026-11-02", "2026-11-02"),
        ("2026-10-29", "2026-10-30"), // 週五沒開電腦：從週五補
        ("2026-10-01", "2026-10-26"), // 太久以前：最多往回 7 天
        ("", "2026-11-02"),           // 從沒同步過：從今天
    ] {
        assert_eq!(
            NaiveDate::parse_from_str(from, "%Y-%m-%d").unwrap(),
            LeavePlanner::backfill_from(last, mon()),
            "{last}"
        );
    }
}

#[test]
fn is_due_今天不用上班_請假放假週末都不做() {
    assert!(!LeavePlanner::is_due(
        &off(thu() + TimeDelta::days(2), None),
        false,
        dt(2026, 10, 31, 18, 0),
        "2026-10-30"
    ));
    assert!(!LeavePlanner::is_due(&on_leave(mon()), true, dt(2026, 11, 2, 18, 0), "2026-10-23")); // 漏了很久也一樣
    assert!(!LeavePlanner::is_due(&off(fri(), Some("國慶日")), false, dt(2026, 10, 30, 18, 0), ""));
}

fn map(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

#[test]
fn leave_piece_參數與說明() {
    let day = LeavePiece::day(fri(), Some(mon()));
    assert_eq!(
        map(json!({ "span": "DAY", "date": "2026-10-30", "endDate": "2026-11-02", "note": "EIP A 特休" })),
        day.args("EIP A 特休")
    );
    assert_eq!("10/30（週五） 到 11/2（週一） 整天", day.describe());

    let range = LeavePiece::range(fri(), t(15, 30), t(17, 30));
    assert_eq!(map(json!({ "span": "RANGE", "date": "2026-10-30", "start": "15:30", "end": "17:30" })), range.args(""));
    assert_eq!("10/30（週五） 15:30–17:30", range.describe());
    assert_eq!(500, range.args(&"x".repeat(600))["note"].as_str().unwrap().chars().count());
}
