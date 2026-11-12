//! 工作台「我的工時」頁（對應 C# `UI/Work/HoursPage.xaml(.cs)`）：我的工時月曆，每天填了多少、應出勤多少；
//! 點一天開工時紀錄補登。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

#[path = "w2_common.rs"]
pub(crate) mod w2_common;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, Months, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use hb_api::{DaySchedule, TimeEntry, WorkMath};
use slint::{ComponentHandle, SharedString};

use self::w2_common::{PageCell, md_weekday, model, spawn};
use crate::{HoursDayCell, HoursPageData, PageArg, WorkCtx};

/// 格子的底色（C# Brushes2：OffDayBrush、BgBrush、FilledBrush、PartialBrush、MissingBrush）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CellBg {
    /// 月初前、月底後補的空格
    None,
    Off,
    Plain,
    Filled,
    Partial,
    Missing,
}

/// 數字的顏色：TextBrush、MutedBrush、ErrorBrush。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueTone {
    Text,
    Muted,
    Error,
}

/// 月曆的一格（C# `DayCellVm`）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DayCell {
    pub date: Option<NaiveDate>,
    pub day_text: String,
    pub tag: String,
    pub value: String,
    pub tip: String,
    pub bg: CellBg,
    pub value_tone: ValueTone,
    pub today: bool,
    pub future: bool,
}

impl DayCell {
    fn empty() -> Self {
        Self {
            date: None,
            day_text: String::new(),
            tag: String::new(),
            value: String::new(),
            tip: String::new(),
            bg: CellBg::None,
            value_tone: ValueTone::Text,
            today: false,
            future: false,
        }
    }
}

/// 一個月的月曆與上方四張數字卡。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MonthView {
    pub cells: Vec<DayCell>,
    pub filled: String,
    pub expected: String,
    pub expected_to_date: String,
    pub missing: String,
    pub missing_error: bool,
    pub unfilled: String,
    pub unfilled_detail: String,
}

/// `yyyy 年 M 月`。
pub(crate) fn month_text(month: NaiveDate) -> String {
    format!("{} 年 {} 月", month.year(), month.month())
}

fn first_of(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

/// C# `Render`：算出每一格與合計（today、now 由呼叫端給，測試才能固定）。
pub(crate) fn build_month(
    month: NaiveDate,
    schedule: &[DaySchedule],
    entries: &[TimeEntry],
    today: NaiveDate,
    now: NaiveDateTime,
) -> MonthView {
    let mut by_day: HashMap<NaiveDate, &DaySchedule> = HashMap::new();
    for s in schedule {
        by_day.entry(s.date).or_insert(s);
    }
    let timed: Vec<&TimeEntry> = entries.iter().filter(|e| e.started_at.is_some()).collect();
    let mut cells = Vec::new();
    let lead = month.weekday().num_days_from_monday(); // 週一開始
    for _ in 0..lead {
        cells.push(DayCell::empty());
    }

    let mut filled_total = 0.0;
    let mut expected_total = 0i64;
    let mut expected_to_date = 0i64;
    let mut missing_to_date = 0.0;
    let (mut partial_days, mut empty_days) = (0, 0);
    let mut d = month;
    while d.month() == month.month() {
        let day = d.and_time(NaiveTime::MIN);
        let worked = if d > today {
            0.0
        } else {
            WorkMath::union_minutes(timed.iter().copied(), day, if d == today { now } else { day + TimeDelta::days(1) })
        };
        let s = by_day.get(&d).copied();
        let expected = s.map_or(0, |s| s.expected_minutes);
        filled_total += worked;
        expected_total += expected as i64;
        let past = d < today;
        if d <= today {
            expected_to_date += expected as i64;
            missing_to_date += WorkMath::shortfall(worked, expected as f64);
        }

        let tag = match s {
            Some(s) if s.holiday.is_some() => s.holiday.clone().unwrap_or_default(),
            Some(s) if s.on_leave => "請假".to_owned(),
            _ => String::new(),
        };
        let exp = expected as f64;
        let (bg, value, value_tone) = if expected == 0 {
            (CellBg::Off, if worked > 0.0 { WorkMath::hours_text(worked) } else { String::new() }, ValueTone::Text)
        } else if d > today {
            (CellBg::Plain, format!("應 {}", WorkMath::hours_text(exp)), ValueTone::Muted)
        } else if WorkMath::is_filled(worked, exp) {
            (
                CellBg::Filled,
                format!("{} / {}", WorkMath::hours_text(worked), WorkMath::hours_text(exp)),
                ValueTone::Text,
            )
        } else if worked > 0.0 {
            if past {
                partial_days += 1;
            }
            (
                if past { CellBg::Partial } else { CellBg::Plain },
                format!("{} / {}", WorkMath::hours_text(worked), WorkMath::hours_text(exp)),
                ValueTone::Text,
            )
        } else {
            if past {
                empty_days += 1;
            }
            (
                if past { CellBg::Missing } else { CellBg::Plain },
                if past {
                    format!("沒填 · 應 {}", WorkMath::hours_text(exp))
                } else {
                    format!("0 / {}", WorkMath::hours_text(exp))
                },
                if past { ValueTone::Error } else { ValueTone::Muted },
            )
        };
        let is_today = d == today;
        cells.push(DayCell {
            date: Some(d),
            day_text: d.day().to_string(),
            tag: if is_today { "今天".to_owned() } else { tag },
            value,
            tip: format!(
                "{}：已填 {}{}\n點一下查看、補登",
                md_weekday(d),
                WorkMath::hours_minutes(worked),
                if expected > 0 { format!("／應出勤 {}", WorkMath::hours_minutes(exp)) } else { String::new() }
            ),
            bg,
            value_tone,
            today: is_today,
            future: d > today,
        });
        d = match d.succ_opt() {
            Some(next) => next,
            None => break,
        };
    }
    while cells.len() % 7 != 0 {
        cells.push(DayCell::empty());
    }

    MonthView {
        cells,
        filled: WorkMath::hours_text(filled_total),
        expected: WorkMath::hours_text(expected_total as f64),
        expected_to_date: format!("到今天 {}", WorkMath::hours_minutes(expected_to_date as f64)),
        missing: WorkMath::hours_text(missing_to_date),
        missing_error: missing_to_date >= 0.5,
        unfilled: format!("{} 天", partial_days + empty_days),
        unfilled_detail: format!("不足 {partial_days} · 沒填 {empty_days}"),
    }
}

struct Page {
    ctx: WorkCtx,
    month: Cell<NaiveDate>,
    version: Cell<u64>,
    /// 每一格對到的日期（點了開那天的工時紀錄）
    dates: RefCell<Vec<Option<NaiveDate>>>,
}

thread_local! {
    static PAGE: PageCell<Page> = const { PageCell::new() };
}

fn current() -> Option<Rc<Page>> {
    PAGE.with(|p| p.get())
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        month: Cell::new(first_of(hb_core::time::today())),
        version: Cell::new(0),
        dates: RefCell::new(Vec::new()),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    let g = window.global::<HoursPageData>();
    let move_month = |delta: i32| {
        move || {
            let Some(page) = current() else { return };
            let m = page.month.get();
            let next =
                if delta < 0 { m.checked_sub_months(Months::new(1)) } else { m.checked_add_months(Months::new(1)) };
            page.month.set(next.unwrap_or(m));
            spawn("切換月份", load(page));
        }
    };
    g.on_prev(move_month(-1));
    g.on_next(move_month(1));
    g.on_this_month(|| {
        let Some(page) = current() else { return };
        page.month.set(first_of(hb_core::time::today()));
        spawn("回到本月", load(page));
    });
    g.on_open_day(|index| {
        let Some(page) = current() else { return };
        let date = usize::try_from(index).ok().and_then(|i| page.dates.borrow().get(i).copied().flatten());
        if let Some(d) = date {
            page.ctx.open_day(d);
        }
    });
}

pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = current() else { return };
    if let Some(d) = arg.as_ref().and_then(PageArg::as_date) {
        page.month.set(first_of(d));
    }
    load(page).await;
}

async fn load(page: Rc<Page>) {
    page.version.set(page.version.get() + 1);
    let version = page.version.get();
    let month = page.month.get();
    let Some(window) = page.ctx.window() else { return };
    {
        let g = window.global::<HoursPageData>();
        g.set_month_text(month_text(month).into());
        g.set_this_month_enabled(month != first_of(hb_core::time::today()));
        g.set_loading_text("讀取中…".into());
    }
    let last = month.checked_add_months(Months::new(1)).and_then(|m| m.pred_opt()).unwrap_or(month);
    let api = page.ctx.engine.api();
    let result = async {
        let schedule = api.get_schedule_range(month, last).await?;
        let entries = api.list_time_entries(month, last).await?;
        Ok::<_, hb_api::McpError>((schedule, entries))
    }
    .await;
    let Some(window) = page.ctx.window() else { return };
    let g = window.global::<HoursPageData>();
    match result {
        Ok((schedule, entries)) => {
            if version != page.version.get() {
                return;
            }
            let view = build_month(month, &schedule, &entries, hb_core::time::today(), hb_core::time::now());
            render(&page, &g, view);
            g.set_loading_text(SharedString::new());
        }
        Err(e) => {
            log::error!("讀取工時月曆失敗: {}", e.0);
            if version == page.version.get() {
                g.set_loading_text(format!("讀取失敗：{}", e.0).into());
            }
        }
    }
}

fn render(page: &Page, g: &HoursPageData, view: MonthView) {
    *page.dates.borrow_mut() = view.cells.iter().map(|c| c.date).collect();
    let cells: Vec<HoursDayCell> = view
        .cells
        .into_iter()
        .map(|c| HoursDayCell {
            has_date: c.date.is_some(),
            day_text: c.day_text.into(),
            tag: c.tag.into(),
            value: c.value.into(),
            tip: c.tip.into(),
            bg: match c.bg {
                CellBg::None => 0,
                CellBg::Off => 1,
                CellBg::Plain => 2,
                CellBg::Filled => 3,
                CellBg::Partial => 4,
                CellBg::Missing => 5,
            },
            value_tone: match c.value_tone {
                ValueTone::Text => 0,
                ValueTone::Muted => 1,
                ValueTone::Error => 2,
            },
            today: c.today,
            future: c.future,
        })
        .collect();
    g.set_cells(model(cells));
    g.set_filled(view.filled.into());
    g.set_expected(view.expected.into());
    g.set_expected_to_date(view.expected_to_date.into());
    g.set_missing(view.missing.into());
    g.set_missing_error(view.missing_error);
    g.set_unfilled(view.unfilled.into());
    g.set_unfilled_detail(view.unfilled_detail.into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn sched(d: NaiveDate, minutes: i32) -> DaySchedule {
        DaySchedule { date: d, working: Vec::new(), expected_minutes: minutes, on_leave: false, holiday: None }
    }

    fn entry(d: NaiveDate, from: (u32, u32), to: (u32, u32)) -> TimeEntry {
        let at = |h, m| Local.from_local_datetime(&d.and_hms_opt(h, m, 0).unwrap()).unwrap().fixed_offset();
        TimeEntry::new("1", "F2P-1", "t", Some(at(from.0, from.1)), Some(at(to.0, to.1)), None, false, None)
    }

    #[test]
    fn 月曆_週一開始並補滿七格() {
        // 2026-11-01 是星期日：前面空 6 格；30 天 → 36 格 → 補到 42
        let month = ymd(2026, 11, 1);
        let today = ymd(2026, 11, 12);
        let v = build_month(month, &[], &[], today, today.and_hms_opt(10, 0, 0).unwrap());
        assert_eq!(42, v.cells.len());
        assert!(v.cells[..6].iter().all(|c| c.date.is_none()));
        assert_eq!(Some(month), v.cells[6].date);
        assert_eq!("今天", v.cells[6 + 11].tag);
        assert_eq!("2026 年 11 月", month_text(month));
    }

    #[test]
    fn 月曆_填滿不足沒填與合計() {
        let month = ymd(2026, 11, 1);
        let today = ymd(2026, 11, 5);
        let now = today.and_hms_opt(18, 0, 0).unwrap();
        let schedule = vec![
            sched(ymd(2026, 11, 2), 480),
            sched(ymd(2026, 11, 3), 480),
            sched(ymd(2026, 11, 4), 480),
            DaySchedule { holiday: Some("國慶".into()), ..sched(ymd(2026, 11, 1), 0) },
            sched(ymd(2026, 11, 6), 480),
        ];
        let entries = vec![
            entry(ymd(2026, 11, 2), (9, 0), (17, 0)),
            entry(ymd(2026, 11, 3), (9, 0), (11, 0)),
            // 重疊只算一次
            entry(ymd(2026, 11, 3), (10, 0), (12, 0)),
        ];
        let v = build_month(month, &schedule, &entries, today, now);
        let cell = |d: u32| v.cells.iter().find(|c| c.date == Some(ymd(2026, 11, d))).unwrap();
        assert_eq!((CellBg::Off, "國慶"), (cell(1).bg, cell(1).tag.as_str()));
        assert_eq!((CellBg::Filled, "8h / 8h"), (cell(2).bg, cell(2).value.as_str()));
        assert_eq!((CellBg::Partial, "3h / 8h"), (cell(3).bg, cell(3).value.as_str()));
        assert_eq!(
            (CellBg::Missing, "沒填 · 應 8h", ValueTone::Error),
            (cell(4).bg, cell(4).value.as_str(), cell(4).value_tone)
        );
        assert_eq!(
            (CellBg::Plain, "應 8h", ValueTone::Muted, true),
            (cell(6).bg, cell(6).value.as_str(), cell(6).value_tone, cell(6).future)
        );
        assert_eq!("11h", v.filled);
        assert_eq!("32h", v.expected);
        assert_eq!("到今天 24 小時", v.expected_to_date);
        assert_eq!("13h", v.missing);
        assert!(v.missing_error);
        assert_eq!(("2 天", "不足 1 · 沒填 1"), (v.unfilled.as_str(), v.unfilled_detail.as_str()));
        assert_eq!("11/2（週一）：已填 8 小時／應出勤 8 小時\n點一下查看、補登", cell(2).tip);
    }

    #[test]
    fn 月曆_今天還沒填不算缺() {
        let month = ymd(2026, 11, 1);
        let today = ymd(2026, 11, 2);
        let v = build_month(month, &[sched(today, 480)], &[], today, today.and_hms_opt(8, 0, 0).unwrap());
        let c = v.cells.iter().find(|c| c.date == Some(today)).unwrap();
        assert_eq!((CellBg::Plain, "0 / 8h", ValueTone::Muted), (c.bg, c.value.as_str(), c.value_tone));
        assert_eq!("不足 0 · 沒填 0", v.unfilled_detail);
    }
}
