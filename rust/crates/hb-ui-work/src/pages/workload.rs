//! 工作台「工作負荷」頁（對應 C# `UI/Work/WorkloadPage.cs`）。
//!
//! 我的工作負荷（跟網頁「工作負荷」視圖同一份算法）：一週每天分配到（預計）或實際計時的工時對上容量（應出勤），
//! 每格顯示幾個任務，合計欄顯示這週做過幾個不同任務；預計模式另列還沒填預計工時、算不進來的任務。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, TimeDelta};
use hb_api::{TimeEntry, WorkMath, Workload};
use hb_ui_common::inputs::StatusMessage;
use slint::ComponentHandle;

use super::hours::w2_common::{self, PageCell, md, model, spawn, strings};
use crate::{PageArg, WorkCtx, WorkloadCell, WorkloadPageData, WorkloadTaskRow};

const WEEKDAY_NAMES: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
/// 模式（key、文字、提示）。
pub(crate) const MODES: [(&str, &str, &str); 2] = [
    ("actual", "實際計時", "實際計時（同時計時只算一次，含補登、會議）"),
    ("planned", "預計", "任務的預計工時照每天的容量（應出勤扣請假、出差）比例分配，參加的會議也算進去"),
];

/// 這週一。
pub(crate) fn monday_of(d: NaiveDate) -> NaiveDate {
    d - TimeDelta::days(d.weekday().num_days_from_monday() as i64)
}

/// 一格的長條顏色：0 線條色（沒有）、1 強調色（還沒滿）、2 綠色（剛好）、3 紅色（超過）。
pub(crate) fn tone(minutes: i32, capacity: i32) -> (f32, i32) {
    let ratio = if capacity > 0 {
        minutes as f64 / capacity as f64
    } else if minutes > 0 {
        1.0
    } else {
        0.0
    };
    let tone = if ratio > 1.05 {
        3
    } else if ratio >= 0.95 {
        2
    } else if minutes > 0 {
        1
    } else {
        0
    };
    (ratio.min(1.0) as f32, tone)
}

/// 一格（C# `Cell`）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CellView {
    pub title: String,
    pub value: String,
    pub capacity: String,
    pub ratio: f32,
    pub tone: i32,
    pub tasks: String,
    pub today: bool,
    pub total: bool,
}

fn cell(title: String, minutes: i32, capacity: i32, tasks: String, today: bool, total: bool) -> CellView {
    let (ratio, tone) = tone(minutes, capacity);
    CellView {
        title,
        value: if minutes > 0 { WorkMath::hours_text(minutes as f64) } else { "—".to_owned() },
        capacity: if capacity > 0 {
            format!("/ {}", WorkMath::hours_text(capacity as f64))
        } else {
            "不上班".to_owned()
        },
        ratio,
        tone,
        tasks,
        today,
        total,
    }
}

/// 實際模式：每天做了幾個任務用自己的計時段算（伺服器只給每天的時數）；第二個值是這週做過幾個不同任務。
pub(crate) fn actual_counts(entries: &[TimeEntry], tasks_in_load: usize) -> (HashMap<NaiveDate, usize>, usize) {
    let mut per_day: HashMap<NaiveDate, Vec<String>> = HashMap::new();
    for e in entries {
        let Some(start) = e.started_at else { continue };
        let day = hb_core::time::local(&start).date();
        let list = per_day.entry(day).or_default();
        if !list.iter().any(|t| t.eq_ignore_ascii_case(&e.task)) {
            list.push(e.task.clone());
        }
    }
    let mut all: Vec<&str> = Vec::new();
    for e in entries {
        if !all.iter().any(|t| t.eq_ignore_ascii_case(&e.task)) {
            all.push(&e.task);
        }
    }
    (per_day.into_iter().map(|(d, l)| (d, l.len())).collect(), tasks_in_load.max(all.len()))
}

/// 七天加合計（C# `Render` 的格子）。
pub(crate) fn cells(
    load: &Workload,
    per_day: Option<&HashMap<NaiveDate, usize>>,
    distinct: usize,
    today: NaiveDate,
) -> Vec<CellView> {
    let mut list = Vec::new();
    for (i, name) in WEEKDAY_NAMES.iter().enumerate() {
        let date = load.from + TimeDelta::days(i as i64);
        let day = load.days.iter().find(|d| d.date == date);
        let count = match per_day {
            Some(p) => p.get(&date).copied().unwrap_or(0),
            None => day.map_or(0, |d| d.task_count.max(0) as usize),
        };
        list.push(cell(
            format!("週{name} {}", md(date)),
            day.map_or(0, |d| d.minutes),
            day.map_or(0, |d| d.capacity_minutes),
            if count > 0 { format!("{count} 個任務") } else { String::new() },
            date == today,
            false,
        ));
    }
    list.push(cell(
        "合計".to_owned(),
        load.total_minutes,
        load.capacity_minutes,
        if distinct > 0 { format!("做過 {distinct} 個不同任務") } else { String::new() },
        false,
        true,
    ));
    list
}

/// 讀回來的：(工作負荷, 實際模式每天幾個任務, 這週做過幾個不同任務)。
type Loaded = (Workload, Option<HashMap<NaiveDate, usize>>, usize);

struct Page {
    ctx: WorkCtx,
    monday: Cell<NaiveDate>,
    mode: Cell<usize>,
    version: Cell<u64>,
}

thread_local! {
    static PAGE: PageCell<Page> = const { PageCell::new() };
}

fn current() -> Option<Rc<Page>> {
    PAGE.with(|p| p.get())
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&WorkloadPageData) -> R) -> Option<R> {
        let window = self.ctx.window()?;
        Some(f(&window.global::<WorkloadPageData>()))
    }
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        monday: Cell::new(monday_of(hb_core::time::today())),
        mode: Cell::new(0),
        version: Cell::new(0),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    let g = window.global::<WorkloadPageData>();
    g.set_modes(strings(MODES.iter().map(|m| m.1)));
    g.set_mode_index(0);
    g.set_mode_tip(MODES[0].2.into());
    g.on_mode_selected(|i| {
        let Some(page) = current() else { return };
        let i = usize::try_from(i).unwrap_or(0).min(MODES.len() - 1);
        page.mode.set(i);
        page.with(|g| g.set_mode_tip(MODES[i].2.into()));
        spawn("讀取工作負荷", load(page));
    });
    let nav = |days: i64| {
        move || {
            let Some(page) = current() else { return };
            page.monday.set(if days == 0 {
                monday_of(hb_core::time::today())
            } else {
                page.monday.get() + TimeDelta::days(days)
            });
            spawn("讀取工作負荷", load(page));
        }
    };
    g.on_prev(nav(-7));
    g.on_next(nav(7));
    g.on_this_week(nav(0));
    g.on_open_task(|key| {
        if let Some(page) = current() {
            page.ctx.show_page("tasks", Some(PageArg::Text(key.to_string())));
        }
    });
}

pub async fn activate(_ctx: WorkCtx, _arg: Option<PageArg>) {
    if let Some(page) = current() {
        load(page).await;
    }
}

async fn load(page: Rc<Page>) {
    page.version.set(page.version.get() + 1);
    let version = page.version.get();
    let (from, to) = (page.monday.get(), page.monday.get() + TimeDelta::days(6));
    let mode = MODES[page.mode.get()].0;
    page.with(|g| {
        g.set_range_text(format!("{}～{}", md(from), md(to)).into());
        g.set_status_text("".into());
    });
    let api = page.ctx.engine.api();
    let result: Result<Loaded, String> = async {
        let load = api.my_workload(from, to, mode).await.map_err(|e| e.0)?;
        let mut per_day = None;
        let mut distinct = load.tasks.len();
        if mode == "actual" {
            let entries = api.list_time_entries(from, to).await.map_err(|e| e.0)?;
            let (p, d) = actual_counts(&entries, distinct);
            per_day = Some(p);
            distinct = d;
        }
        Ok((load, per_day, distinct))
    }
    .await;
    if version != page.version.get() {
        return;
    }
    match result {
        Ok((load, per_day, distinct)) => render(&page, &load, per_day.as_ref(), distinct),
        Err(message) => {
            log::error!("讀取工作負荷失敗: {message}");
            let (text, tone) = w2_common::status(Some(&StatusMessage::error(format!("讀取失敗：{message}"))));
            page.with(|g| {
                g.set_status_text(text);
                g.set_status_tone(tone);
            });
        }
    }
}

fn render(page: &Page, load: &Workload, per_day: Option<&HashMap<NaiveDate, usize>>, distinct: usize) {
    let views = cells(load, per_day, distinct, hb_core::time::today());
    let mut tasks = load.tasks.clone();
    tasks.sort_by_key(|t| std::cmp::Reverse(t.minutes));
    page.with(|g| {
        g.set_days(model(
            views
                .into_iter()
                .map(|c| WorkloadCell {
                    title: c.title.into(),
                    value: c.value.into(),
                    capacity: c.capacity.into(),
                    ratio: c.ratio,
                    tone: c.tone,
                    tasks: c.tasks.into(),
                    today: c.today,
                    total: c.total,
                })
                .collect(),
        ));
        g.set_tasks_empty(
            if load.tasks.is_empty() {
                if load.mode == "actual" {
                    "這週還沒有計時"
                } else {
                    "這週沒有分配到預計工時的任務"
                }
            } else {
                ""
            }
            .into(),
        );
        g.set_tasks(model(
            tasks
                .iter()
                .map(|t| WorkloadTaskRow {
                    key: t.key.as_str().into(),
                    title: t.title.as_str().into(),
                    right: WorkMath::hours_minutes(t.minutes as f64).into(),
                })
                .collect(),
        ));
        let show_incomplete = load.mode == "planned" && !load.incomplete.is_empty();
        g.set_incomplete_title(
            if show_incomplete { format!("算不進來的任務 {}", load.incomplete.len()) } else { String::new() }.into(),
        );
        g.set_incomplete(model(if show_incomplete {
            load.incomplete
                .iter()
                .map(|(key, title, reason)| WorkloadTaskRow {
                    key: key.as_str().into(),
                    title: title.as_str().into(),
                    right: reason.as_str().into(),
                })
                .collect()
        } else {
            Vec::new()
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use hb_api::{WorkloadDay, WorkloadTask};

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn 週一與顏色() {
        assert_eq!(ymd(2026, 11, 9), monday_of(ymd(2026, 11, 12)));
        assert_eq!(ymd(2026, 11, 9), monday_of(ymd(2026, 11, 15)));
        assert_eq!((0.0, 0), tone(0, 480));
        assert_eq!((0.5, 1), tone(240, 480));
        assert_eq!((1.0, 2), tone(480, 480));
        assert_eq!(2, tone(470, 480).1);
        assert_eq!((1.0, 3), tone(600, 480));
        // 不上班的日子有計時：比例當 1，跟 C# 一樣是綠色
        assert_eq!((1.0, 2), tone(60, 0));
    }

    #[test]
    fn 實際模式_每天幾個任務() {
        let t = |d: u32, h: u32| {
            Some(Local.from_local_datetime(&ymd(2026, 11, d).and_hms_opt(h, 0, 0).unwrap()).unwrap().fixed_offset())
        };
        let entries = vec![
            TimeEntry::new("1", "F2P-1", "", t(9, 9), t(9, 10), None, false, None),
            TimeEntry::new("2", "f2p-1", "", t(9, 11), t(9, 12), None, false, None),
            TimeEntry::new("3", "F2P-2", "", t(10, 9), t(10, 10), None, false, None),
            TimeEntry::new("4", "F2P-3", "", None, None, Some(30), false, None),
        ];
        let (per_day, distinct) = actual_counts(&entries, 1);
        assert_eq!(Some(&1), per_day.get(&ymd(2026, 11, 9)));
        assert_eq!(Some(&1), per_day.get(&ymd(2026, 11, 10)));
        assert_eq!(3, distinct);
    }

    #[test]
    fn 格子() {
        let from = ymd(2026, 11, 9);
        let load = Workload {
            mode: "planned".into(),
            from,
            to: ymd(2026, 11, 15),
            total_minutes: 900,
            capacity_minutes: 2400,
            days: vec![
                WorkloadDay { date: from, minutes: 480, capacity_minutes: 480, task_count: 2 },
                WorkloadDay { date: ymd(2026, 11, 14), minutes: 0, capacity_minutes: 0, task_count: 0 },
            ],
            tasks: vec![WorkloadTask { key: "F2P-1".into(), title: "a".into(), minutes: 900 }],
            incomplete: vec![],
        };
        let c = cells(&load, None, 1, ymd(2026, 11, 9));
        assert_eq!(8, c.len());
        assert_eq!(
            ("週一 11/9", "8h", "/ 8h", "2 個任務", true, 2),
            (
                c[0].title.as_str(),
                c[0].value.as_str(),
                c[0].capacity.as_str(),
                c[0].tasks.as_str(),
                c[0].today,
                c[0].tone
            )
        );
        assert_eq!(
            ("週六 11/14", "—", "不上班", ""),
            (c[5].title.as_str(), c[5].value.as_str(), c[5].capacity.as_str(), c[5].tasks.as_str())
        );
        assert_eq!(
            ("合計", "15h", "/ 40h", "做過 1 個不同任務", true),
            (c[7].title.as_str(), c[7].value.as_str(), c[7].capacity.as_str(), c[7].tasks.as_str(), c[7].total)
        );
        // 實際模式：沒有計時段的那天就是 0 個任務（不看伺服器的數字）
        let c = cells(&load, Some(&HashMap::new()), 0, ymd(2026, 11, 9));
        assert_eq!(("", ""), (c[0].tasks.as_str(), c[7].tasks.as_str()));
    }
}
