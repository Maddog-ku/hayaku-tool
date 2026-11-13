//! 工作台「提醒」頁（對應 C# `UI/Work/RemindersPage.xaml(.cs)`）：新增（一次或重複、可綁任務）、延後、暫停重複、刪除。
//! 跟上 Hayaku 1.5.4（C# 版沒有）：快速時間照自己的上班時段即時算（[`quick_times`]）；
//! 「綁在」直接列出最近填過、指派給我的任務與近期會議（[`bind_choices`]，只用 engine 已有的資料，不多發請求），選會議送 `meeting: true`。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeDelta, Timelike};
use hb_api::{DateTimeOffset, DaySchedule, KeyFinder, MeetingItem, ReminderFull, TaskItem, TimeEntry};
use hb_ui_common::inputs::{self, StatusMessage};
use serde_json::{Map, Value, json};
use slint::{ComponentHandle, SharedString};

use super::hours::w2_common::{self, ConfirmSlot, PageCell, model, spawn, strings};
use crate::{PageArg, RemindersBind, RemindersPageData, RemindersQuick, RemindersRow, WorkCtx};

/// 分頁（C# RadioButton 的 Tag）。
pub(crate) const TABS: [&str; 3] = ["upcoming", "fired", "repeat"];
pub(crate) const TAB_LABELS: [&str; 3] = ["即將到來", "已提醒", "重複"];
/// 重複（C# ComboBoxItem 的 Content、Tag）。
pub(crate) const REPEATS: [(&str, &str); 5] = [
    ("不重複", "none"),
    ("每天", "daily"),
    ("每個工作日", "workdays"),
    ("每週（同一個星期幾）", "weekly"),
    ("每月（同一天）", "monthly"),
];
/// 時間照（上面的日期、任務預計開始、任務預計結束）。
pub(crate) const RELATIVES: [(&str, &str); 3] =
    [("上面的日期", ""), ("任務預計開始", "plannedStart"), ("任務預計結束", "plannedEnd")];
/// 提醒誰。
pub(crate) const AUDIENCES: [(&str, &str); 2] = [("只有我", "me"), ("任務的所有執行者", "assignees")];

/// 提醒列表的一列（C# `ReminderListVm`）。
pub(crate) fn when_text(at: Option<&DateTimeOffset>, today: NaiveDate) -> String {
    let Some(at) = at else { return "—".to_owned() };
    let local = hb_core::time::local(at);
    let hm = local.format("%H:%M");
    if local.date() == today {
        format!("今天 {hm}")
    } else if Some(local.date()) == today.succ_opt() {
        format!("明天 {hm}")
    } else {
        format!("{}/{} {hm}", local.month(), local.day())
    }
}

pub(crate) fn detail_text(item: &ReminderFull) -> String {
    [
        item.task.clone(),
        item.repeat_text.clone(),
        item.paused.then(|| "已暫停".to_owned()),
        (!item.editable).then(|| "別人設的".to_owned()),
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// 排序：已提醒的新的在前，其他照時間（沒有時間的在最後）。
pub(crate) fn sort(items: &mut [ReminderFull], tab: &str) {
    items.sort_by_key(|r| {
        let t = r.at.map(|a| a.timestamp());
        if tab == "fired" { -t.unwrap_or(0) } else { t.unwrap_or(i64::MAX) }
    });
}

pub(crate) fn empty_text(tab: &str) -> &'static str {
    match tab {
        "fired" => "沒有已提醒的紀錄",
        "repeat" => "沒有重複提醒",
        _ => "沒有即將到來的提醒，在上面新增一個",
    }
}

/// 新增欄位的預設時間：一小時後、取整到 0 或 30 分。
pub(crate) fn next_slot(now: NaiveDateTime) -> NaiveDateTime {
    let next = now + TimeDelta::hours(1);
    let minute = if next.minute() < 30 { 0 } else { 30 };
    next.date().and_hms_opt(next.hour(), minute, 0).unwrap_or(next)
}

/// 快速時間的一個選項。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QuickTime {
    pub tag: &'static str,
    pub label: &'static str,
    pub at: NaiveDateTime,
}

/// 快速時間（跟上 Hayaku 1.5.4，C# 版只有 30 分鐘後、1 小時後、明天上班）：照自己的上班時段即時算；
/// 已經過了或不適用的不列。schedule 是今天的班表（不是今天的當作沒有）；is_workday 判斷某天要不要上班。
/// 明天、下個工作日的時間照今天的上班時段（第一段開始＝上班、第二段開始＝午休後）；今天不用上班、
/// 或今天請了假（時段不是平常的）就不知道平常幾點上班，不列這幾個。
/// before_end_minutes：「今天下班前」是下班前幾分鐘（設定 › 一般，預設 15）。
pub(crate) fn quick_times(
    now: NaiveDateTime,
    schedule: Option<&DaySchedule>,
    before_end_minutes: i64,
    is_workday: impl Fn(NaiveDate) -> bool,
) -> Vec<QuickTime> {
    let mut out: Vec<QuickTime> = [("5", "5 分鐘後", 5), ("10", "10 分鐘後", 10), ("60", "1 小時後", 60)]
        .into_iter()
        .map(|(tag, label, m)| QuickTime { tag, label, at: now + TimeDelta::minutes(m) })
        .collect();
    let today = now.date();
    let Some(schedule) = schedule.filter(|s| s.date == today && !s.working.is_empty()) else {
        return out;
    };
    let mut segments = schedule.working.clone();
    segments.sort_by_key(|w| w.start);
    let first = segments[0].start;
    let after_lunch = segments.get(1).map(|w| w.start);
    if let Some(lunch) = after_lunch.map(|t| today.and_time(t)).filter(|t| *t > now) {
        out.push(QuickTime { tag: "lunch", label: "午休後", at: lunch });
    }
    if let Some(last) = segments.last() {
        let before_end = today.and_time(last.end) - TimeDelta::minutes(before_end_minutes);
        if before_end > now && before_end > today.and_time(last.start) {
            out.push(QuickTime { tag: "end", label: "今天下班前", at: before_end });
        }
    }
    if schedule.on_leave {
        return out;
    }
    let tomorrow = today.succ_opt();
    if let Some(t) = tomorrow.filter(|t| is_workday(*t)) {
        out.push(QuickTime { tag: "tomorrow", label: "明天上班", at: t.and_time(first) });
    }
    // 下個工作日：跳過不上班的日子（最多找一個月）
    let next = (1..=31).filter_map(|n| today.checked_add_signed(TimeDelta::days(n))).find(|d| is_workday(*d));
    if let Some(next) = next {
        // 下個工作日就是明天時，上午跟「明天上班」一樣，不重複列
        if Some(next) != tomorrow {
            out.push(QuickTime { tag: "next-am", label: "下個工作日上午", at: next.and_time(first) });
        }
        if let Some(lunch) = after_lunch {
            out.push(QuickTime { tag: "next-pm", label: "下個工作日下午", at: next.and_time(lunch) });
        }
    }
    out
}

/// 「綁在」清單的一項：任務或會議。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindChoice {
    /// 0 最近填過、1 指派給我的、2 近期會議
    pub group: usize,
    pub key: String,
    pub title: String,
    pub meta: String,
}

pub(crate) const BIND_GROUPS: [&str; 3] = ["最近填過", "指派給我的", "近期會議"];

/// 「綁在」直接列的選項（跟上 Hayaku 1.5.4，C# 版沒有）：今天計時過的任務（新的在前）、指派給我的其他任務、
/// 還沒結束的會議（照開始時間）。只用 engine 已讀到的資料。
pub(crate) fn bind_choices(
    entries: &[TimeEntry],
    my_tasks: &[TaskItem],
    meetings: &[MeetingItem],
    now: &DateTimeOffset,
    today: NaiveDate,
) -> Vec<BindChoice> {
    let mut out: Vec<BindChoice> = Vec::new();
    let has = |out: &[BindChoice], key: &str| out.iter().any(|c| c.key.eq_ignore_ascii_case(key));
    let mut recent: Vec<&TimeEntry> = entries.iter().filter(|e| !e.task.is_empty()).collect();
    recent.sort_by_key(|e| std::cmp::Reverse(e.started_at));
    for e in recent {
        if out.len() >= 8 {
            break;
        }
        if !has(&out, &e.task) {
            let meta = if e.running { "計時中".to_owned() } else { String::new() };
            out.push(BindChoice { group: 0, key: e.task.clone(), title: e.title.clone(), meta });
        }
    }
    let mine: Vec<BindChoice> = my_tasks
        .iter()
        .filter(|t| !has(&out, &t.key))
        .take(30)
        .map(|t| BindChoice { group: 1, key: t.key.clone(), title: t.title.clone(), meta: t.status.clone() })
        .collect();
    out.extend(mine);
    let mut upcoming: Vec<&MeetingItem> = meetings.iter().filter(|m| !m.key.is_empty() && m.end_at >= *now).collect();
    upcoming.sort_by_key(|m| m.start_at);
    out.extend(upcoming.into_iter().take(10).map(|m| BindChoice {
        group: 2,
        key: m.key.clone(),
        title: m.title.clone(),
        meta: when_text(Some(&m.start_at), today),
    }));
    out
}

/// 日期欄的文字：今天、明天（quick 才有）或 M/d。
pub(crate) fn date_box_text(at: NaiveDateTime, today: NaiveDate, with_tomorrow: bool) -> String {
    if at.date() == today {
        "今天".to_owned()
    } else if with_tomorrow && Some(at.date()) == today.succ_opt() {
        "明天".to_owned()
    } else {
        format!("{}/{}", at.month(), at.day())
    }
}

/// 新增表單（畫面上的欄位）。
#[derive(Debug, Clone, Default)]
pub(crate) struct Form {
    pub message: String,
    pub task: String,
    pub date: String,
    pub time: String,
    pub offset: String,
    pub repeat: usize,
    pub relative: usize,
    pub audience: usize,
    /// 綁的是會議（從「綁在」清單選了會議；Hayaku 1.5.4 起，C# 版沒有）
    pub meeting: bool,
}

/// 驗證失敗時要把焦點移到哪。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    None,
    Message,
    Task,
}

/// C# `AddAsync` 的驗證與參數：成功回 (create_reminder 參數, 成功訊息)；失敗回 (錯誤訊息, 焦點)。
pub(crate) fn build_request(
    f: &Form,
    now: DateTimeOffset,
    today: NaiveDate,
) -> Result<(Map<String, Value>, String), (String, Focus)> {
    let message = f.message.trim();
    let task_text = f.task.trim();
    let mut task: Option<String> = None;
    if !task_text.is_empty() {
        task = KeyFinder::parse(Some(task_text));
        if task.is_none() {
            return Err(("綁任務請填任務編號，例 F2P-97".into(), Focus::None));
        }
    }
    if message.is_empty() && task.is_none() {
        return Err(("請填提醒內容".into(), Focus::Message));
    }
    let relative = RELATIVES.get(f.relative).map_or("", |r| r.1);
    let audience = AUDIENCES.get(f.audience).map_or("me", |a| a.1);
    let repeat = REPEATS.get(f.repeat).map_or("none", |r| r.1);
    if (!relative.is_empty() || audience == "assignees") && task.is_none() {
        return Err((
            if !relative.is_empty() {
                "照任務預計日期提醒要先填綁哪個任務"
            } else {
                "提醒所有執行者要先填綁哪個任務"
            }
            .into(),
            Focus::Task,
        ));
    }
    if !relative.is_empty() && f.meeting {
        return Err(("照任務預計日期提醒只能綁任務，綁會議請改回「上面的日期」".into(), Focus::None));
    }
    if !relative.is_empty() && repeat != "none" {
        return Err(("照任務預計日期提醒不能重複，重複請改回「上面的日期」".into(), Focus::None));
    }
    let Some(time) = inputs::parse_time(Some(&f.time)) else {
        return Err(("時間請用 HH:mm，例 14:30".into(), Focus::None));
    };
    let hm = time.format("%H:%M").to_string();
    let mut args = Map::new();
    if !message.is_empty() {
        args.insert("message".into(), json!(message));
    }
    if let Some(t) = &task {
        args.insert("task".into(), json!(t));
        if f.meeting {
            args.insert("meeting".into(), json!(true));
        }
    }
    if audience == "assignees" {
        args.insert("audience".into(), json!("assignees"));
    }
    if !relative.is_empty() {
        let offset_text = f.offset.trim();
        let offset = match (if offset_text.is_empty() { "0" } else { offset_text }).parse::<i32>() {
            Ok(o) if (-30..=30).contains(&o) => o,
            _ => return Err(("前後幾天請填 -30～30 的整數，例 -1 是前一天".into(), Focus::None)),
        };
        args.insert("relativeTo".into(), json!(relative));
        args.insert("offsetDays".into(), json!(offset));
        args.insert("time".into(), json!(hm));
        let when = format!(
            "{}{}",
            if relative == "plannedStart" { "預計開始" } else { "預計結束" },
            if offset == 0 {
                "當天".to_owned()
            } else if offset < 0 {
                format!("前 {} 天", -offset)
            } else {
                format!("後 {offset} 天")
            }
        );
        let success = format!(
            "已新增：{} {when} {hm}{}",
            task.as_deref().unwrap_or(""),
            if message.is_empty() { String::new() } else { format!(" {message}") }
        );
        return Ok((args, success));
    }
    let Some(date) = inputs::parse_date(Some(&f.date), Some(today)) else {
        return Err(("日期看不懂，請用 今天、明天、11/5 這類寫法".into(), Focus::None));
    };
    if repeat == "none" {
        let at = inputs::local(date, time);
        if at < now {
            return Err(("這個時間已經過了".into(), Focus::None));
        }
        args.insert("at".into(), json!(inputs::iso(&at)));
    } else {
        let rule = match repeat {
            "daily" => json!({ "freq": "DAILY", "interval": 1, "workdaysOnly": false }),
            "workdays" => json!({ "freq": "DAILY", "interval": 1, "workdaysOnly": true }),
            "weekly" => json!({ "freq": "WEEKLY", "interval": 1, "weekdays": [date.weekday().num_days_from_monday()] }),
            _ => json!({ "freq": "MONTHLY", "interval": 1, "monthDay": date.day() }),
        };
        args.insert(
            "repeat".into(),
            json!({ "rule": rule, "startDate": inputs::date_text(Some(date)), "timeOfDay": hm }),
        );
    }
    let success = format!(
        "已新增：{}/{} {hm} {}",
        date.month(),
        date.day(),
        if message.is_empty() { task.as_deref().unwrap_or("") } else { message }
    );
    Ok((args, success))
}

struct Page {
    ctx: WorkCtx,
    tab: Cell<usize>,
    version: Cell<u64>,
    items: RefCell<Vec<ReminderFull>>,
    confirm: Rc<ConfirmSlot>,
    /// 「綁在」清單畫面上每一列（None＝分組標題）
    binds: RefCell<Vec<Option<BindChoice>>>,
    /// 目前列出的快速時間（一樣就不重畫）
    quicks: RefCell<Vec<QuickTime>>,
}

thread_local! {
    static PAGE: PageCell<Page> = const { PageCell::new() };
}

fn current() -> Option<Rc<Page>> {
    PAGE.with(|p| p.get())
}

impl Page {
    fn tab(&self) -> &'static str {
        TABS[self.tab.get().min(TABS.len() - 1)]
    }

    fn with<R>(&self, f: impl FnOnce(&RemindersPageData) -> R) -> Option<R> {
        let window = self.ctx.window()?;
        Some(f(&window.global::<RemindersPageData>()))
    }

    fn show_form(&self, m: Option<StatusMessage>) {
        let (text, tone) = w2_common::status(m.as_ref());
        self.with(|g| {
            g.set_form_status(text);
            g.set_form_tone(tone);
        });
    }

    fn show_list(&self, m: Option<StatusMessage>) {
        let (text, tone) = w2_common::status(m.as_ref());
        self.with(|g| {
            g.set_list_status(text);
            g.set_list_tone(tone);
        });
    }

    fn reset_form(&self) {
        let now = hb_core::time::now();
        let next = next_slot(now);
        self.with(|g| {
            g.set_date_text(date_box_text(next, now.date(), false).into());
            g.set_time_text(next.format("%H:%M").to_string().into());
            g.set_message(SharedString::new());
            g.set_task_text(SharedString::new());
            g.set_repeat_index(0);
            g.set_relative_index(0);
            g.set_offset_text(SharedString::new());
            g.set_audience_index(0);
            g.set_bind_meeting(false);
        });
        self.refresh_quick();
    }

    /// 算快速時間：今天的班表、哪些天要上班（不用上班的日子快取；還沒記過就只跳過週末）。
    fn compute_quick(&self) -> Vec<QuickTime> {
        let engine = &self.ctx.engine;
        let (cached, off) = {
            let st = engine.state();
            (!st.off_days_updated.is_empty(), st.off_days.clone())
        };
        let is_workday = move |d: NaiveDate| {
            if cached {
                !off.contains_key(&d.format("%Y-%m-%d").to_string())
            } else {
                d.weekday().num_days_from_monday() < 5
            }
        };
        let before_end = i64::from(engine.config().reminder_before_end_minutes.clamp(0, 600));
        quick_times(hb_core::time::now(), engine.schedule().as_ref(), before_end, is_workday)
    }

    /// 重算快速時間（打開這頁、新增後、滑鼠移到新增區、按了快速時間時；只用本機資料）。有變才重畫。
    fn refresh_quick(&self) {
        let quicks = self.compute_quick();
        if *self.quicks.borrow() == quicks {
            return;
        }
        let today = hb_core::time::today();
        let rows: Vec<RemindersQuick> = quicks
            .iter()
            .map(|q| RemindersQuick {
                tag: q.tag.into(),
                label: q.label.into(),
                tip: format!("{} {}", inputs::short_date_at(q.at.date(), today), q.at.format("%H:%M")).into(),
            })
            .collect();
        self.with(|g| g.set_quick_items(model(rows)));
        *self.quicks.borrow_mut() = quicks;
    }

    /// 打開「綁在」清單：用 engine 已有的今天計時、我的任務、會議重排（不發請求）。
    fn refresh_binds(&self) {
        let engine = &self.ctx.engine;
        let now = hb_core::time::now_offset();
        let choices = bind_choices(&engine.entries(), &engine.my_tasks(), &engine.meetings(), &now, now.date_naive());
        let mut slots: Vec<Option<BindChoice>> = Vec::new();
        let mut rows: Vec<RemindersBind> = Vec::new();
        for (group, label) in BIND_GROUPS.iter().enumerate() {
            let items: Vec<&BindChoice> = choices.iter().filter(|c| c.group == group).collect();
            if items.is_empty() {
                continue;
            }
            slots.push(None);
            rows.push(RemindersBind { header: true, title: (*label).into(), ..Default::default() });
            for c in items {
                slots.push(Some(c.clone()));
                rows.push(RemindersBind {
                    header: false,
                    key: c.key.as_str().into(),
                    title: c.title.as_str().into(),
                    meta: c.meta.as_str().into(),
                    meeting: c.group == 2,
                });
            }
        }
        let empty = rows.is_empty();
        self.with(|g| {
            g.set_bind_rows(model(rows));
            g.set_bind_empty(if empty { "還沒有計時過、指派給我的任務或近期會議" } else { "" }.into());
        });
        *self.binds.borrow_mut() = slots;
    }

    fn form(&self) -> Form {
        self.with(|g| Form {
            message: g.get_message().to_string(),
            task: g.get_task_text().to_string(),
            date: g.get_date_text().to_string(),
            time: g.get_time_text().to_string(),
            offset: g.get_offset_text().to_string(),
            repeat: usize::try_from(g.get_repeat_index()).unwrap_or(0),
            relative: usize::try_from(g.get_relative_index()).unwrap_or(0),
            audience: usize::try_from(g.get_audience_index()).unwrap_or(0),
            meeting: g.get_bind_meeting(),
        })
        .unwrap_or_default()
    }

    fn item(&self, index: i32) -> Option<ReminderFull> {
        usize::try_from(index).ok().and_then(|i| self.items.borrow().get(i).cloned())
    }
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        tab: Cell::new(0),
        version: Cell::new(0),
        items: RefCell::new(Vec::new()),
        confirm: Rc::new(ConfirmSlot::default()),
        binds: RefCell::new(Vec::new()),
        quicks: RefCell::new(Vec::new()),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    w2_common::wire_confirm!(window, RemindersPageData, page.confirm.clone());
    let g = window.global::<RemindersPageData>();
    g.set_tab_items(strings(TAB_LABELS));
    g.set_repeat_items(strings(REPEATS.iter().map(|r| r.0)));
    g.set_relative_items(strings(RELATIVES.iter().map(|r| r.0)));
    g.set_audience_items(strings(AUDIENCES.iter().map(|r| r.0)));
    page.reset_form();

    g.on_tab_selected(|i| {
        let Some(page) = current() else { return };
        page.tab.set(usize::try_from(i).unwrap_or(0));
        spawn("切換提醒分頁", load(page));
    });
    g.on_quick(|tag| {
        let Some(page) = current() else { return };
        let today = hb_core::time::today();
        // 按下時再算一次：清單是之前算的，那個時間可能已經過了
        let at = page.compute_quick().into_iter().find(|q| q.tag == tag.as_str()).map(|q| q.at);
        page.refresh_quick();
        let Some(at) = at else {
            page.show_form(Some(StatusMessage::error("這個時間已經過了")));
            return;
        };
        page.show_form(None);
        page.with(|g| {
            g.set_date_text(date_box_text(at, today, true).into());
            g.set_time_text(at.format("%H:%M").to_string().into());
            g.set_repeat_index(0);
            g.set_focus_message(g.get_focus_message() + 1);
        });
    });
    g.on_quick_refresh(|| {
        if let Some(page) = current() {
            page.refresh_quick();
        }
    });
    g.on_bind_open(|| {
        if let Some(page) = current() {
            page.refresh_binds();
        }
    });
    g.on_bind_picked(|i| {
        let Some(page) = current() else { return };
        let Some(choice) = usize::try_from(i).ok().and_then(|i| page.binds.borrow().get(i).cloned().flatten()) else {
            return;
        };
        page.with(|g| {
            g.set_task_text(choice.key.as_str().into());
            g.set_bind_meeting(choice.group == 2);
            if g.get_message().is_empty() {
                g.set_message(choice.title.as_str().into());
            }
            g.set_focus_message(g.get_focus_message() + 1);
        });
    });
    // 自己打的編號當作任務（從清單選了會議才送 meeting）
    g.on_task_edited(|_| {
        if let Some(page) = current() {
            page.with(|g| g.set_bind_meeting(false));
        }
    });
    g.on_add(|| {
        if let Some(page) = current() {
            spawn("新增提醒", add(page));
        }
    });
    g.on_snooze(|i| {
        let Some(page) = current() else { return };
        let Some(item) = page.item(i) else { return };
        spawn("延後提醒", async move {
            let api = page.ctx.engine.api();
            if row_action(&page, i, "snooze", async move { w2_common::mcp(api.snooze_reminder(&item.id, 10).await) })
                .await
            {
                load(page).await;
            }
        });
    });
    g.on_pause(|i| {
        let Some(page) = current() else { return };
        let Some(item) = page.item(i) else { return };
        spawn("暫停提醒", async move {
            let mut args = Map::new();
            args.insert("id".into(), json!(item.id));
            args.insert("paused".into(), json!(!item.paused));
            let api = page.ctx.engine.api();
            if row_action(&page, i, "pause", async move { w2_common::mcp(api.update_reminder(args).await) }).await {
                load(page).await;
            }
        });
    });
    g.on_open_task(|i| {
        let Some(page) = current() else { return };
        if let Some(task) = page.item(i).and_then(|r| r.task) {
            page.ctx.show_page("tasks", Some(PageArg::Text(task)));
        }
    });
    g.on_delete(|i| {
        let Some(page) = current() else { return };
        let Some(item) = page.item(i) else { return };
        let Some(window) = page.ctx.window() else { return };
        let p = page.clone();
        w2_common::ask!(
            window,
            RemindersPageData,
            page.confirm,
            "刪除提醒",
            format!("刪除提醒「{}」？", item.text),
            move || {
                spawn("刪除提醒", async move {
                    let api = p.ctx.engine.api();
                    let id = item.id.clone();
                    if row_action(&p, i, "delete", async move { w2_common::mcp(api.delete_reminder(&id).await) }).await
                    {
                        p.ctx.engine.after_external_change().await;
                        load(p).await;
                    }
                });
            }
        );
    });
}

/// arg：任務編號 → 預先填好「綁任務」。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = current() else { return };
    if let Some(k) = arg.as_ref().and_then(PageArg::as_text).and_then(|t| KeyFinder::parse(Some(t))) {
        let title = page.ctx.engine.title_of(&k);
        page.with(|g| {
            g.set_task_text(k.as_str().into());
            g.set_bind_meeting(false);
            if g.get_message().is_empty()
                && let Some(t) = title
            {
                g.set_message(t.into());
            }
            g.set_focus_message(g.get_focus_message() + 1);
        });
    }
    page.refresh_quick();
    load(page).await;
}

async fn load(page: Rc<Page>) {
    page.version.set(page.version.get() + 1);
    let version = page.version.get();
    let tab = page.tab();
    page.with(|g| {
        g.set_empty_text("讀取中…".into());
        g.set_empty_visible(true);
    });
    page.show_list(None);
    let result = page.ctx.engine.api().list_reminders_full(tab, None).await;
    match result {
        Ok(mut items) => {
            if version != page.version.get() {
                return;
            }
            sort(&mut items, tab);
            let today = hb_core::time::today();
            let rows: Vec<RemindersRow> = items
                .iter()
                .map(|r| RemindersRow {
                    when: when_text(r.at.as_ref(), today).into(),
                    when_muted: r.paused || tab == "fired",
                    text: r.text.as_str().into(),
                    detail: detail_text(r).into(),
                    snooze: tab != "repeat",
                    pause: tab == "repeat" && r.editable,
                    pause_text: if r.paused { "恢復" } else { "暫停" }.into(),
                    has_task: r.task.is_some(),
                    deletable: r.editable,
                    busy: SharedString::new(),
                })
                .collect();
            let n = rows.len();
            page.with(|g| {
                g.set_rows(model(rows));
                g.set_empty_text(empty_text(tab).into());
                g.set_empty_visible(n == 0);
                g.set_filter_text(if n > 0 { format!("{n} 則") } else { String::new() }.into());
            });
            *page.items.borrow_mut() = items;
        }
        Err(e) => {
            log::error!("讀取提醒失敗: {}", e.0);
            if version == page.version.get() {
                page.with(|g| g.set_empty_visible(false));
                page.show_list(Some(StatusMessage::error(format!("讀取失敗：{}", e.0))));
            }
        }
    }
}

/// 列上的按鈕（C# `Inputs.Run((Button)sender, ListStatus, …)`）：按鈕變成處理中，失敗顯示在列表上方。
async fn row_action(
    page: &Rc<Page>,
    index: i32,
    which: &'static str,
    action: impl std::future::Future<Output = Result<(), inputs::ActionError>>,
) -> bool {
    let set_busy = |busy: bool| {
        page.with(|g| {
            let rows = g.get_rows();
            if let Some(mut row) = usize::try_from(index).ok().and_then(|i| slint::Model::row_data(&rows, i)) {
                row.busy = if busy { which } else { "" }.into();
                slint::Model::set_row_data(&rows, index as usize, row);
            }
        });
    };
    inputs::run(set_busy, |m| page.show_list(m), || action, None).await
}

async fn add(page: Rc<Page>) {
    let form = page.form();
    let now = hb_core::time::now_offset();
    let (args, success) = match build_request(&form, now, now.date_naive()) {
        Ok(r) => r,
        Err((message, focus)) => {
            page.show_form(Some(StatusMessage::error(message)));
            page.with(|g| match focus {
                Focus::Message => g.set_focus_message(g.get_focus_message() + 1),
                Focus::Task => g.set_focus_task(g.get_focus_task() + 1),
                Focus::None => {}
            });
            return;
        }
    };
    let api = page.ctx.engine.api();
    let ok = inputs::run(
        |busy| {
            page.with(|g| g.set_add_busy(busy));
        },
        |m| page.show_form(m),
        || async move { w2_common::mcp(api.create_reminder(args).await) },
        Some(&success),
    )
    .await;
    if !ok {
        return;
    }
    page.reset_form();
    page.ctx.engine.after_external_change().await;
    load(page).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(d: NaiveDate, h: u32, m: u32) -> DateTimeOffset {
        Local.from_local_datetime(&d.and_hms_opt(h, m, 0).unwrap()).unwrap().fixed_offset()
    }

    fn item(text: &str) -> ReminderFull {
        ReminderFull {
            id: "1".into(),
            text: text.into(),
            at: None,
            task: None,
            repeat_text: None,
            paused: false,
            editable: true,
        }
    }

    #[test]
    fn 時間文字() {
        let today = ymd(2026, 11, 12);
        assert_eq!("今天 09:30", when_text(Some(&at(today, 9, 30)), today));
        assert_eq!("明天 08:00", when_text(Some(&at(ymd(2026, 11, 13), 8, 0)), today));
        assert_eq!("11/20 14:05", when_text(Some(&at(ymd(2026, 11, 20), 14, 5)), today));
        assert_eq!("—", when_text(None, today));
    }

    #[test]
    fn 說明文字() {
        let mut r = item("x");
        assert_eq!("", detail_text(&r));
        r.task = Some("F2P-1".into());
        r.repeat_text = Some("每天 09:00".into());
        r.paused = true;
        r.editable = false;
        assert_eq!("F2P-1 · 每天 09:00 · 已暫停 · 別人設的", detail_text(&r));
    }

    #[test]
    fn 排序_已提醒新的在前() {
        let d = ymd(2026, 11, 12);
        let mut items = vec![
            ReminderFull { at: Some(at(d, 10, 0)), ..item("a") },
            ReminderFull { at: None, ..item("b") },
            ReminderFull { at: Some(at(d, 9, 0)), ..item("c") },
        ];
        sort(&mut items, "upcoming");
        assert_eq!(vec!["c", "a", "b"], items.iter().map(|r| r.text.as_str()).collect::<Vec<_>>());
        sort(&mut items, "fired");
        assert_eq!(vec!["a", "c", "b"], items.iter().map(|r| r.text.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn 預設時間_一小時後取整() {
        let d = ymd(2026, 11, 12);
        assert_eq!(d.and_hms_opt(10, 0, 0).unwrap(), next_slot(d.and_hms_opt(9, 12, 40).unwrap()));
        assert_eq!(d.and_hms_opt(10, 30, 0).unwrap(), next_slot(d.and_hms_opt(9, 45, 0).unwrap()));
        assert_eq!("今天", date_box_text(d.and_hms_opt(10, 0, 0).unwrap(), d, false));
        assert_eq!("11/13", date_box_text(ymd(2026, 11, 13).and_hms_opt(0, 30, 0).unwrap(), d, false));
        assert_eq!("明天", date_box_text(ymd(2026, 11, 13).and_hms_opt(9, 0, 0).unwrap(), d, true));
    }

    fn form(message: &str, task: &str, date: &str, time: &str) -> Form {
        Form { message: message.into(), task: task.into(), date: date.into(), time: time.into(), ..Form::default() }
    }

    #[test]
    fn 新增_驗證順序() {
        let today = ymd(2026, 11, 12);
        let now = at(today, 9, 0);
        let err = |f: &Form| build_request(f, now, today).unwrap_err();
        assert_eq!(("綁任務請填任務編號，例 F2P-97".into(), Focus::None), err(&form("x", "abc", "今天", "10:00")));
        assert_eq!(("請填提醒內容".into(), Focus::Message), err(&form(" ", "", "今天", "10:00")));
        let f = Form { relative: 1, ..form("x", "", "今天", "10:00") };
        assert_eq!(("照任務預計日期提醒要先填綁哪個任務".into(), Focus::Task), err(&f));
        let f = Form { audience: 1, ..form("x", "", "今天", "10:00") };
        assert_eq!(("提醒所有執行者要先填綁哪個任務".into(), Focus::Task), err(&f));
        let f = Form { relative: 2, repeat: 1, ..form("x", "F2P-1", "今天", "10:00") };
        assert_eq!("照任務預計日期提醒不能重複，重複請改回「上面的日期」", err(&f).0);
        assert_eq!("時間請用 HH:mm，例 14:30", err(&form("x", "", "今天", "25:00")).0);
        assert_eq!("日期看不懂，請用 今天、明天、11/5 這類寫法", err(&form("x", "", "下週", "10:00")).0);
        assert_eq!("這個時間已經過了", err(&form("x", "", "今天", "08:00")).0);
        let f = Form { relative: 1, offset: "31".into(), ..form("x", "F2P-1", "", "10:00") };
        assert_eq!("前後幾天請填 -30～30 的整數，例 -1 是前一天", err(&f).0);
    }

    #[test]
    fn 新增_一次與重複的參數() {
        let today = ymd(2026, 11, 12);
        let now = at(today, 9, 0);
        let (args, success) = build_request(&form("開會", "", "明天", "1400"), now, today).unwrap();
        assert_eq!(json!({ "message": "開會", "at": inputs::iso(&at(ymd(2026, 11, 13), 14, 0)) }), Value::Object(args));
        assert_eq!("已新增：11/13 14:00 開會", success);

        let f = Form { repeat: 3, ..form("", "f2p-7", "11/16", "9:30") };
        let (args, success) = build_request(&f, now, today).unwrap();
        assert_eq!(
            json!({ "task": "F2P-7", "repeat": { "rule": { "freq": "WEEKLY", "interval": 1, "weekdays": [0] }, "startDate": "2026-11-16", "timeOfDay": "09:30" } }),
            Value::Object(args)
        );
        assert_eq!("已新增：11/16 09:30 F2P-7", success);

        let f = Form { repeat: 4, ..form("繳費", "", "11/5", "9:30") };
        let (args, _) = build_request(&f, now, today).unwrap();
        assert_eq!(json!({ "freq": "MONTHLY", "interval": 1, "monthDay": 5 }), args["repeat"]["rule"]);
        let f = Form { repeat: 2, ..form("站會", "", "今天", "9:30") };
        let (args, _) = build_request(&f, now, today).unwrap();
        assert_eq!(json!({ "freq": "DAILY", "interval": 1, "workdaysOnly": true }), args["repeat"]["rule"]);
    }

    fn day_schedule(d: NaiveDate, segments: &[(u32, u32, u32, u32)], on_leave: bool) -> DaySchedule {
        DaySchedule {
            date: d,
            working: segments
                .iter()
                .map(|&(h1, m1, h2, m2)| hb_api::WorkSegment {
                    start: chrono::NaiveTime::from_hms_opt(h1, m1, 0).unwrap(),
                    end: chrono::NaiveTime::from_hms_opt(h2, m2, 0).unwrap(),
                })
                .collect(),
            expected_minutes: 480,
            on_leave,
            holiday: None,
        }
    }

    fn weekdays(d: NaiveDate) -> bool {
        d.weekday().num_days_from_monday() < 5
    }

    fn tags(q: &[QuickTime]) -> Vec<(&'static str, NaiveDateTime)> {
        q.iter().map(|q| (q.tag, q.at)).collect()
    }

    #[test]
    fn 快速時間_照班表即時算() {
        // 2026-11-12 週四，08:30–12:00、13:30–17:30
        let d = ymd(2026, 11, 12);
        let s = day_schedule(d, &[(13, 30, 17, 30), (8, 30, 12, 0)], false);
        let now = d.and_hms_opt(10, 0, 0).unwrap();
        let q = quick_times(now, Some(&s), 30, weekdays);
        assert_eq!(
            vec![
                ("5", d.and_hms_opt(10, 5, 0).unwrap()),
                ("10", d.and_hms_opt(10, 10, 0).unwrap()),
                ("60", d.and_hms_opt(11, 0, 0).unwrap()),
                ("lunch", d.and_hms_opt(13, 30, 0).unwrap()),
                ("end", d.and_hms_opt(17, 0, 0).unwrap()),
                ("tomorrow", ymd(2026, 11, 13).and_hms_opt(8, 30, 0).unwrap()),
                // 下個工作日就是明天：上午跟「明天上班」重複，不列
                ("next-pm", ymd(2026, 11, 13).and_hms_opt(13, 30, 0).unwrap()),
            ],
            tags(&q)
        );
        assert_eq!(
            vec!["5 分鐘後", "10 分鐘後", "1 小時後", "午休後", "今天下班前", "明天上班", "下個工作日下午"],
            q.iter().map(|q| q.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn 快速時間_今天下班前照設定的分鐘數() {
        let d = ymd(2026, 11, 12);
        let s = day_schedule(d, &[(13, 30, 17, 30), (8, 30, 12, 0)], false);
        let now = d.and_hms_opt(10, 0, 0).unwrap();
        let end =
            |minutes| quick_times(now, Some(&s), minutes, weekdays).into_iter().find(|q| q.tag == "end").map(|q| q.at);
        assert_eq!(Some(d.and_hms_opt(17, 15, 0).unwrap()), end(15));
        assert_eq!(Some(d.and_hms_opt(16, 30, 0).unwrap()), end(60));
        // 17:20 時，下班前 15 分鐘（17:15）已經過了，不列
        let late = d.and_hms_opt(17, 20, 0).unwrap();
        assert!(!quick_times(late, Some(&s), 15, weekdays).iter().any(|q| q.tag == "end"));
    }

    #[test]
    fn 快速時間_過了午休與下班前就不列() {
        let d = ymd(2026, 11, 12);
        let s = day_schedule(d, &[(8, 30, 12, 0), (13, 30, 17, 30)], false);
        let q = quick_times(d.and_hms_opt(14, 0, 0).unwrap(), Some(&s), 30, weekdays);
        assert!(!q.iter().any(|q| q.tag == "lunch"));
        assert!(q.iter().any(|q| q.tag == "end"));
        let q = quick_times(d.and_hms_opt(17, 0, 0).unwrap(), Some(&s), 30, weekdays);
        assert!(!q.iter().any(|q| q.tag == "end" || q.tag == "lunch"));
    }

    #[test]
    fn 快速時間_下個工作日跳過週末與不上班的日子() {
        // 週五：明天週六不上班；下週一（11/16）放假 → 下個工作日是 11/17
        let d = ymd(2026, 11, 13);
        let s = day_schedule(d, &[(9, 0, 12, 0), (13, 0, 18, 0)], false);
        let off = [ymd(2026, 11, 16)];
        let q = quick_times(d.and_hms_opt(9, 0, 0).unwrap(), Some(&s), 30, |x| weekdays(x) && !off.contains(&x));
        let t = tags(&q);
        assert!(!t.iter().any(|(tag, _)| *tag == "tomorrow"));
        assert!(t.contains(&("next-am", ymd(2026, 11, 17).and_hms_opt(9, 0, 0).unwrap())));
        assert!(t.contains(&("next-pm", ymd(2026, 11, 17).and_hms_opt(13, 0, 0).unwrap())));
    }

    #[test]
    fn 快速時間_沒有班表或請假只列不靠平常時段的() {
        let d = ymd(2026, 11, 14);
        let now = d.and_hms_opt(10, 0, 0).unwrap();
        // 今天不用上班（沒有時段）、或班表不是今天的：只有幾分鐘後
        let off = day_schedule(d, &[], false);
        assert_eq!(
            vec!["5", "10", "60"],
            quick_times(now, Some(&off), 30, weekdays).iter().map(|q| q.tag).collect::<Vec<_>>()
        );
        let old = day_schedule(ymd(2026, 11, 13), &[(9, 0, 18, 0)], false);
        assert_eq!(3, quick_times(now, Some(&old), 30, weekdays).len());
        assert_eq!(3, quick_times(now, None, 30, weekdays).len());
        // 請了半天假：今天的午休後、下班前照算，明天、下個工作日不列；只有一段就沒有午休後
        let d = ymd(2026, 11, 12);
        let half = day_schedule(d, &[(13, 30, 17, 30)], true);
        let q = quick_times(d.and_hms_opt(9, 0, 0).unwrap(), Some(&half), 30, weekdays);
        assert_eq!(vec!["5", "10", "60", "end"], q.iter().map(|q| q.tag).collect::<Vec<_>>());
    }

    #[test]
    fn 綁在_最近填過指派給我的近期會議() {
        let today = ymd(2026, 11, 12);
        let now = at(today, 10, 0);
        let entry = |task: &str, h: u32, running: bool| {
            let mut e =
                TimeEntry::new("x", task, format!("{task} 標題"), Some(at(today, h, 0)), None, None, running, None);
            e.running = running;
            e
        };
        let entries = vec![entry("F2P-1", 8, false), entry("F2P-2", 9, true), entry("f2p-1", 7, false)];
        let task = |k: &str| TaskItem {
            key: k.into(),
            title: format!("{k} 名稱"),
            project: "F2P".into(),
            status: "進行中".into(),
        };
        let mine = vec![task("F2P-1"), task("F2P-3")];
        let meeting = |key: &str, h: u32| MeetingItem {
            key: key.into(),
            id: key.into(),
            title: format!("{key} 會"),
            project: "F2P".into(),
            start_at: at(today, h, 0),
            end_at: at(today, h + 1, 0),
            location: None,
            my_status: None,
        };
        let meetings = vec![meeting("F2P-9", 14), meeting("F2P-8", 8), meeting("F2P-7", 11)];
        let c = bind_choices(&entries, &mine, &meetings, &now, today);
        let keys: Vec<(usize, &str)> = c.iter().map(|c| (c.group, c.key.as_str())).collect();
        // 最近的在前、不分大小寫去重；指派給我的跳過已列的；已經結束的會議不列，照開始時間
        assert_eq!(vec![(0, "F2P-2"), (0, "F2P-1"), (1, "F2P-3"), (2, "F2P-7"), (2, "F2P-9")], keys);
        assert_eq!("計時中", c[0].meta);
        assert_eq!("今天 11:00", c[3].meta);
    }

    #[test]
    fn 新增_綁會議送meeting() {
        let today = ymd(2026, 11, 12);
        let now = at(today, 9, 0);
        let f = Form { meeting: true, ..form("", "REVIVAL-9", "今天", "10:00") };
        let (args, _) = build_request(&f, now, today).unwrap();
        assert_eq!(
            json!({ "task": "REVIVAL-9", "meeting": true, "at": inputs::iso(&at(today, 10, 0)) }),
            Value::Object(args)
        );
        let f = Form { meeting: true, relative: 1, ..form("", "REVIVAL-9", "", "10:00") };
        assert_eq!(
            "照任務預計日期提醒只能綁任務，綁會議請改回「上面的日期」",
            build_request(&f, now, today).unwrap_err().0
        );
    }

    #[test]
    fn 新增_照任務預計日期() {
        let today = ymd(2026, 11, 12);
        let now = at(today, 9, 0);
        let f = Form { relative: 2, offset: "-1".into(), audience: 1, ..form("要交了", "F2P-9", "", "17:00") };
        let (args, success) = build_request(&f, now, today).unwrap();
        assert_eq!(
            json!({ "message": "要交了", "task": "F2P-9", "audience": "assignees", "relativeTo": "plannedEnd", "offsetDays": -1, "time": "17:00" }),
            Value::Object(args)
        );
        assert_eq!("已新增：F2P-9 預計結束前 1 天 17:00 要交了", success);
        let f = Form { relative: 1, ..form("", "F2P-9", "", "9:00") };
        assert_eq!("已新增：F2P-9 預計開始當天 09:00", build_request(&f, now, today).unwrap().1);
    }
}
