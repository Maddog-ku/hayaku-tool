//! 工作台「工時紀錄」頁（對應 C# `UI/Work/DayEntriesPage.xaml(.cs)`）：查看、修改、補登某一天的計時紀錄。
//! 跟上 Hayaku 1.5.3／1.5.4（C# 版沒有）：點日期從日曆選日期（DateRangePicker 單日）；
//! 移動工時選好任務直接移過去、不再多確認（C# 版本來就沒有多一步），移完可以〔復原〕；
//! 選任務的視窗會列最近填過、指派給我的任務與所屬專案、上層（hb-ui-common task_picker）。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use hb_api::{DateTimeOffset, DaySchedule, KeyFinder, TimeEntry, WorkMath};
use hb_ui_common::date_range::{from_int, to_int, today_int};
use hb_ui_common::{DayTimelineData, day_timeline_model, inputs, task_picker};
use slint::{ComponentHandle, SharedString};

use super::hours::w2_common::{self, ConfirmSlot, PageCell, hm, md_weekday, model, spawn, strings};
use crate::{DayTimelineModel, EntriesPageData, EntriesRow, PageArg, WorkCtx};

/// 一段計時紀錄（C# `EntryRowVm`）：使用者改起訖、備註時跟目前的值比，有變才送。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EntryRowVm {
    pub entry: TimeEntry,
    pub start_text: String,
    pub end_text: String,
    pub note: String,
    pub duration_text: String,
}

impl EntryRowVm {
    pub fn new(entry: TimeEntry, day_start: NaiveDateTime, now: NaiveDateTime) -> Self {
        let minutes = WorkMath::union_minutes([&entry], day_start, now);
        let clock = WorkMath::clock(minutes);
        let duration_text = clock[..clock.len().saturating_sub(3)].to_owned();
        Self {
            start_text: entry.started_at.as_ref().map(hm).unwrap_or_default(),
            end_text: entry.ended_at.as_ref().map(hm).unwrap_or_default(),
            note: entry.note.clone().unwrap_or_default(),
            duration_text,
            entry,
        }
    }

    pub fn running(&self) -> bool {
        self.entry.running
    }

    /// 已結束的才能移到別的任務（移動＝在新任務補登同一段，再刪掉這段）。
    pub fn can_move(&self) -> bool {
        !self.running() && self.entry.started_at.is_some() && self.entry.ended_at.is_some()
    }

    pub fn move_tip(&self) -> &'static str {
        if self.can_move() {
            "移到別的任務（起訖、備註不變）"
        } else {
            "正在計時的那段不能移，先暫停再移"
        }
    }
}

/// 起訖的寬鬆解析：H:mm、HH:mm、Hmm、HHmm（C# `ParseTime`，不收只打小時）。
pub(crate) fn parse_clock(text: &str) -> Option<NaiveTime> {
    let t = text.trim();
    if t.len() <= 2 && t.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    inputs::parse_time(Some(t))
}

/// 那天的某個時間（本地時區）。
pub(crate) fn at(day: NaiveDate, text: &str) -> Option<DateTimeOffset> {
    parse_clock(text).map(|t| inputs::local(day, t))
}

/// 右上角的合計與班表文字（C# `Render` 的 TotalText、ScheduleText）；第二個值：還沒填滿（強調色）。
pub(crate) fn summary(worked: f64, schedule: Option<&DaySchedule>) -> (String, bool, String) {
    let expected = schedule.map_or(0, |s| s.expected_minutes) as f64;
    let total = format!(
        "{}{}",
        WorkMath::hours_minutes(worked),
        if expected > 0.0 { format!(" / {}", WorkMath::hours_minutes(expected)) } else { String::new() }
    );
    let filled = WorkMath::is_filled(worked, expected);
    let schedule_text = match schedule {
        Some(s) if !s.working.is_empty() => format!(
            "班表 {}{}",
            s.working
                .iter()
                .map(|w| format!("{}–{}", w.start.format("%H:%M"), w.end.format("%H:%M")))
                .collect::<Vec<_>>()
                .join("、"),
            if !filled { format!("　還差 {}", WorkMath::hours_minutes(expected - worked)) } else { String::new() }
        ),
        Some(DaySchedule { holiday: Some(h), .. }) => format!("放假（{h}）"),
        _ => "不上班".to_owned(),
    };
    (total, !filled, schedule_text)
}

/// 移動工時後的復原：(移過去的那段、新那段的 id、原本的任務)。
type Undo = (TimeEntry, String, String);

struct Page {
    ctx: WorkCtx,
    day: Cell<NaiveDate>,
    schedule: RefCell<Option<DaySchedule>>,
    rows: RefCell<Vec<EntryRowVm>>,
    gaps: RefCell<Vec<(NaiveDateTime, NaiveDateTime)>>,
    busy: Cell<bool>,
    undo: RefCell<Option<Undo>>,
    confirm: Rc<ConfirmSlot>,
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
        day: Cell::new(hb_core::time::today()),
        schedule: RefCell::new(None),
        rows: RefCell::new(Vec::new()),
        gaps: RefCell::new(Vec::new()),
        busy: Cell::new(false),
        undo: RefCell::new(None),
        confirm: Rc::new(ConfirmSlot::default()),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    w2_common::wire_confirm!(window, EntriesPageData, page.confirm.clone());
    let g = window.global::<EntriesPageData>();
    let show_day = |delta: Option<i64>| {
        move || {
            let Some(page) = current() else { return };
            let day = match delta {
                Some(d) => page.day.get().checked_add_signed(TimeDelta::days(d)).unwrap_or(page.day.get()),
                None => hb_core::time::today(),
            };
            page.day.set(day);
            spawn("切換日期", load(page));
        }
    };
    g.on_prev_day(show_day(Some(-1)));
    g.on_next_day(show_day(Some(1)));
    g.on_today(show_day(None));
    g.on_date_picked(|v| {
        let Some(page) = current() else { return };
        let Some(day) = from_int(v) else { return };
        if day != page.day.get() {
            page.day.set(day);
            spawn("切換日期", load(page));
        }
    });
    g.on_edit_start(|i, text| edited(i, Field::Start, text.as_str()));
    g.on_edit_end(|i, text| edited(i, Field::End, text.as_str()));
    g.on_edit_note(|i, text| edited(i, Field::Note, text.as_str()));
    g.on_delete(|i| {
        let Some(page) = current() else { return };
        let Some(row) = usize::try_from(i).ok().and_then(|i| page.rows.borrow().get(i).cloned()) else { return };
        let Some(window) = page.ctx.window() else { return };
        let text = format!(
            "刪除 {} {}–{} 這段紀錄？",
            row.entry.task,
            row.start_text,
            if row.running() { "進行中".to_owned() } else { row.end_text.clone() }
        );
        let p = page.clone();
        w2_common::ask!(window, EntriesPageData, page.confirm, "刪除紀錄", text, move || {
            let id = row.entry.id.clone();
            let engine = p.ctx.engine.clone();
            spawn("刪除紀錄", async move {
                run(p, async move { engine.delete_time_entry(&id).await }).await;
            });
        });
    });
    g.on_add(|| {
        let Some(page) = current() else { return };
        spawn("補登", add(page));
    });
    g.on_gap(|i| {
        let Some(page) = current() else { return };
        let Some((start, end)) = usize::try_from(i).ok().and_then(|i| page.gaps.borrow().get(i).copied()) else {
            return;
        };
        let Some(window) = page.ctx.window() else { return };
        let g = window.global::<EntriesPageData>();
        g.set_new_start(start.format("%H:%M").to_string().into());
        g.set_new_end(end.format("%H:%M").to_string().into());
        g.set_focus_task(g.get_focus_task() + 1);
    });
    g.on_move(|i| {
        let Some(page) = current() else { return };
        let Some(row) = usize::try_from(i).ok().and_then(|i| page.rows.borrow().get(i).cloned()) else { return };
        if !row.can_move() {
            return;
        }
        spawn("移動工時", async move {
            let tasks: Vec<_> = page
                .ctx
                .engine
                .my_tasks()
                .iter()
                .filter(|t| !t.key.eq_ignore_ascii_case(&row.entry.task))
                .cloned()
                .collect();
            let prompt = format!(
                "把 {} {}–{} 這段移到哪個任務？（也可以直接輸入編號）",
                row.entry.task, row.start_text, row.end_text
            );
            // 選好任務直接移過去（同 Hayaku 1.5.4 的〔移過去〕，不再多問一次），移完可以〔復原〕
            let Some(keys) = task_picker::pick(&tasks, &prompt, false).await else { return };
            let Some(to) = keys.first().cloned() else { return };
            move_entry(page, row.entry, to, false).await;
        });
    });
    g.on_undo(|| {
        let Some(page) = current() else { return };
        let Some((moved, new_id, from)) = page.undo.borrow().clone() else { return };
        let entry = TimeEntry { id: new_id, ..moved };
        spawn("復原移動", move_entry(page, entry, from, true));
    });
}

/// arg：要看的日期；沒給就是上次看的那天。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = current() else { return };
    if let Some(d) = arg.as_ref().and_then(PageArg::as_date) {
        page.day.set(d);
    }
    if let Some(window) = page.ctx.window() {
        let items: Vec<String> = page.ctx.engine.my_tasks().iter().map(|t| format!("{}  {}", t.key, t.title)).collect();
        window.global::<EntriesPageData>().set_task_items(strings(items));
    }
    load(page).await;
}

fn show_error(page: &Page, message: Option<&str>) {
    if let Some(window) = page.ctx.window() {
        window.global::<EntriesPageData>().set_error_text(message.unwrap_or("").into());
    }
}

fn set_enabled(page: &Page, enabled: bool) {
    if let Some(window) = page.ctx.window() {
        window.global::<EntriesPageData>().set_busy(!enabled);
    }
}

async fn load(page: Rc<Page>) {
    let day = page.day.get();
    let Some(window) = page.ctx.window() else { return };
    {
        let g = window.global::<EntriesPageData>();
        g.set_date_text(md_weekday(day).into());
        g.set_today_enabled(day != hb_core::time::today());
        g.set_pick_date(to_int(Some(day)));
        g.set_today_int(today_int());
        g.set_loading(true);
        g.set_empty(false);
        g.set_error_text(SharedString::new());
    }
    let result = page.ctx.engine.load_day(day).await;
    let Some(window) = page.ctx.window() else { return };
    let g = window.global::<EntriesPageData>();
    match result {
        Ok((entries, schedule)) => {
            if day != page.day.get() {
                g.set_loading(false); // 讀取途中又換了一天（C# 的 finally 照樣收起讀取中）
                return;
            }
            *page.schedule.borrow_mut() = Some(schedule);
            render(&page, &g, entries);
        }
        Err(e) => {
            g.set_error_text(format!("讀取失敗：{}", e.message()).into());
            page.rows.borrow_mut().clear();
            g.set_rows(model(Vec::new()));
        }
    }
    g.set_loading(false);
}

fn render(page: &Page, g: &EntriesPageData, entries: Vec<TimeEntry>) {
    let now = hb_core::time::now();
    let day = page.day.get();
    let day_start = day.and_time(NaiveTime::MIN);
    let rows: Vec<EntryRowVm> = entries.iter().cloned().map(|e| EntryRowVm::new(e, day_start, now)).collect();
    g.set_rows(model(rows.iter().map(to_row).collect()));
    g.set_empty(rows.is_empty());
    *page.rows.borrow_mut() = rows;

    let worked = WorkMath::union_minutes(entries.iter(), day_start, now);
    let schedule = page.schedule.borrow().clone();
    let (total, accent, schedule_text) = summary(worked, schedule.as_ref());
    g.set_total_text(total.into());
    g.set_total_accent(accent);
    g.set_schedule_text(schedule_text.into());

    let segments = schedule.as_ref().map(|s| s.working.as_slice());
    let data = DayTimelineData::new(day, segments, &entries, now, true);
    g.set_timeline(day_timeline_model!(DayTimelineModel, data));

    // 班表時段裡還沒有任何計時的空檔（5 分鐘以上），點一下就帶入補登欄位
    let gaps = match &schedule {
        Some(s) => WorkMath::gaps(&s.working, entries.iter(), day_start, now, 5.0),
        None => Vec::new(),
    };
    g.set_gaps_label(
        match (&schedule, gaps.is_empty()) {
            (None, _) => "",
            (Some(_), true) => "班表時段都有紀錄了。",
            (Some(_), false) => "沒有紀錄的時段：",
        }
        .into(),
    );
    g.set_gaps(strings(gaps.iter().map(|(s, e)| format!("{}–{} ＋", s.format("%H:%M"), e.format("%H:%M")))));
    *page.gaps.borrow_mut() = gaps;
}

fn to_row(r: &EntryRowVm) -> EntriesRow {
    EntriesRow {
        key: r.entry.task.as_str().into(),
        title: r.entry.title.as_str().into(),
        running: r.running(),
        start: r.start_text.as_str().into(),
        end: r.end_text.as_str().into(),
        duration: r.duration_text.as_str().into(),
        note: r.note.as_str().into(),
        can_move: r.can_move(),
        move_tip: r.move_tip().into(),
    }
}

#[derive(Clone, Copy)]
enum Field {
    Start,
    End,
    Note,
}

/// 使用者改了起訖或備註（C# `OnRowEdited`；離開欄位或按 Enter 時，跟目前的值不同才算）。
fn edited(index: i32, field: Field, text: &str) {
    let Some(page) = current() else { return };
    let Some(i) = usize::try_from(index).ok() else { return };
    let row = {
        let mut rows = page.rows.borrow_mut();
        let Some(row) = rows.get_mut(i) else { return };
        let current = match field {
            Field::Start => &mut row.start_text,
            Field::End => &mut row.end_text,
            Field::Note => &mut row.note,
        };
        if current == text {
            return;
        }
        *current = text.to_owned();
        row.clone()
    };
    if page.busy.get() {
        return;
    }
    let day = page.day.get();
    let (mut start, mut end, mut note) = (None, None, None);
    match field {
        Field::Start => {
            start = at(day, &row.start_text);
            if start.is_none() {
                show_error(&page, Some("開始時間格式不對，請用 HH:mm（例 09:30）"));
                return;
            }
        }
        Field::End => {
            end = at(day, &row.end_text);
            if end.is_none() {
                show_error(&page, Some("結束時間格式不對，請用 HH:mm（例 17:30）"));
                return;
            }
        }
        Field::Note => note = Some(row.note.clone()),
    }
    let effective_start = start.or(row.entry.started_at);
    let effective_end = end.or(row.entry.ended_at);
    if let (Some(s), Some(e)) = (effective_start, effective_end)
        && e <= s
    {
        show_error(&page, Some("結束時間要晚於開始時間"));
        return;
    }
    let engine = page.ctx.engine.clone();
    let id = row.entry.id.clone();
    spawn("修改紀錄", async move {
        run(page, async move { engine.update_time_entry(&id, start, end, note.as_deref()).await }).await;
    });
}

async fn add(page: Rc<Page>) {
    let Some(window) = page.ctx.window() else { return };
    let g = window.global::<EntriesPageData>();
    let Some(task) = KeyFinder::parse(Some(g.get_new_task().as_str())) else {
        show_error(&page, Some("請輸入任務編號，例 F2P-97"));
        g.set_focus_task(g.get_focus_task() + 1);
        return;
    };
    let day = page.day.get();
    let (Some(start), Some(end)) = (at(day, &g.get_new_start()), at(day, &g.get_new_end())) else {
        show_error(&page, Some("請填開始與結束時間（HH:mm）"));
        return;
    };
    if end <= start {
        show_error(&page, Some("結束時間要晚於開始時間"));
        return;
    }
    let note = g.get_new_note().to_string();
    let engine = page.ctx.engine.clone();
    drop(g);
    if run(page.clone(), async move { engine.add_time_entry(&task, start, end, Some(&note)).await }).await
        && let Some(window) = page.ctx.window()
    {
        let g = window.global::<EntriesPageData>();
        g.set_new_start(SharedString::new());
        g.set_new_end(SharedString::new());
        g.set_new_note(SharedString::new());
    }
}

/// 移動（或復原）一段工時，成功後可以復原。
async fn move_entry(page: Rc<Page>, entry: TimeEntry, to_task: String, undo: bool) {
    if let Some(window) = page.ctx.window() {
        let g = window.global::<EntriesPageData>();
        g.set_undo_visible(false);
    }
    *page.undo.borrow_mut() = None;
    show_error(&page, None);
    set_enabled(&page, false);
    let result = page.ctx.engine.move_time_entry(&entry, &to_task).await;
    let mut error = None;
    match result {
        Ok(new_id) => {
            let target = to_task.to_uppercase();
            let text = if undo {
                format!("已復原，這段回到 {target}")
            } else {
                format!(
                    "已把 {}–{} 從 {} 移到 {target}",
                    entry.started_at.as_ref().map(hm).unwrap_or_default(),
                    entry.ended_at.as_ref().map(hm).unwrap_or_default(),
                    entry.task
                )
            };
            if !undo && let Some(id) = new_id {
                *page.undo.borrow_mut() =
                    Some((TimeEntry { task: target.clone(), ..entry.clone() }, id, entry.task.clone()));
            }
            if let Some(window) = page.ctx.window() {
                let g = window.global::<EntriesPageData>();
                g.set_undo_text(text.into());
                g.set_undo_button(page.undo.borrow().is_some());
                g.set_undo_visible(true);
            }
        }
        Err(e) => error = Some(format!("移動失敗：{}", e.message())),
    }
    set_enabled(&page, true);
    // C# 先顯示錯誤再 LoadAsync，而 LoadAsync 一開頭會清掉錯誤；這裡改成讀完再顯示
    load(page.clone()).await;
    if let Some(error) = error {
        show_error(&page, Some(&error));
    }
}

/// 執行修改並重新讀取；失敗時顯示 Hayaku 回的錯誤（C# `RunAsync`：engine 的動作把錯誤寫在 Error）。
async fn run(page: Rc<Page>, action: impl std::future::Future<Output = ()>) -> bool {
    page.busy.set(true);
    show_error(&page, None);
    set_enabled(&page, false);
    action.await;
    let ok = match page.ctx.engine.error() {
        Some(error) => {
            // C# 先顯示錯誤再 LoadAsync，而 LoadAsync 一開頭會清掉錯誤，使用者看不到；這裡改成讀完再顯示
            load(page.clone()).await;
            show_error(&page, Some(&error));
            false
        }
        None => {
            load(page.clone()).await;
            true
        }
    };
    set_enabled(&page, true);
    page.busy.set(false);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use hb_api::WorkSegment;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn 起訖解析_不收只打小時() {
        assert_eq!(NaiveTime::from_hms_opt(9, 30, 0), parse_clock("9:30"));
        assert_eq!(NaiveTime::from_hms_opt(9, 30, 0), parse_clock(" 0930 "));
        // .NET ParseExact 的 H 會先吃兩位數（90 點），所以 905 不行，跟 C# 一樣
        assert_eq!(None, parse_clock("905"));
        assert_eq!(NaiveTime::from_hms_opt(14, 0, 0), parse_clock("1400"));
        assert_eq!(None, parse_clock("9"));
        assert_eq!(None, parse_clock("25:00"));
        assert_eq!(None, parse_clock(""));
    }

    #[test]
    fn 紀錄列_時長與能不能移() {
        let day = ymd(2026, 11, 12);
        let t = |h, m| Local.from_local_datetime(&day.and_hms_opt(h, m, 0).unwrap()).unwrap().fixed_offset();
        let done = TimeEntry::new("1", "F2P-1", "a", Some(t(9, 0)), Some(t(10, 30)), None, false, None);
        let vm = EntryRowVm::new(done, day.and_time(NaiveTime::MIN), day.and_hms_opt(12, 0, 0).unwrap());
        assert_eq!(
            ("09:00", "10:30", "1:30"),
            (vm.start_text.as_str(), vm.end_text.as_str(), vm.duration_text.as_str())
        );
        assert!(vm.can_move());
        let running = TimeEntry::new("2", "F2P-1", "a", Some(t(11, 0)), None, None, true, None);
        let vm = EntryRowVm::new(running, day.and_time(NaiveTime::MIN), day.and_hms_opt(12, 15, 0).unwrap());
        assert_eq!(("", "1:15"), (vm.end_text.as_str(), vm.duration_text.as_str()));
        assert!(!vm.can_move());
        assert_eq!("正在計時的那段不能移，先暫停再移", vm.move_tip());
    }

    #[test]
    fn 合計與班表文字() {
        let seg = |a: (u32, u32), b: (u32, u32)| WorkSegment {
            start: NaiveTime::from_hms_opt(a.0, a.1, 0).unwrap(),
            end: NaiveTime::from_hms_opt(b.0, b.1, 0).unwrap(),
        };
        let s = DaySchedule {
            date: ymd(2026, 11, 12),
            working: vec![seg((8, 30), (12, 0)), seg((13, 0), (17, 30))],
            expected_minutes: 480,
            on_leave: false,
            holiday: None,
        };
        assert_eq!(
            ("6 小時 / 8 小時".to_owned(), true, "班表 08:30–12:00、13:00–17:30　還差 2 小時".to_owned()),
            summary(360.0, Some(&s))
        );
        assert_eq!(
            ("8 小時 / 8 小時".to_owned(), false, "班表 08:30–12:00、13:00–17:30".to_owned()),
            summary(480.0, Some(&s))
        );
        let off = DaySchedule { working: vec![], expected_minutes: 0, holiday: Some("國慶日".into()), ..s.clone() };
        assert_eq!(("30 分".to_owned(), false, "放假（國慶日）".to_owned()), summary(30.0, Some(&off)));
        assert_eq!("不上班", summary(0.0, None).2);
    }
}
