//! 工作台「工作日誌」頁（對應 C# `UI/Work/WorkLogPage.xaml(.cs)`）：某天的工作日誌，照計時紀錄整理的任務與時數，
//! 加上自己在網頁寫的日誌；可以請 Claude 整理草稿、寫進工時備註。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, TimeDelta};
use hb_api::{TimeEntry, WorkLog, WorkMath};
use hb_core::ClaudeDigest;
use hb_ui_common::task_text::set_clipboard;
use slint::{ComponentHandle, SharedString};

use super::hours::w2_common::{self, ConfirmSlot, PageCell, hm, md_weekday, model, spawn, tone};
use crate::{PageArg, WorkCtx, WorkLogPageData, WorkLogRow};

const DRAFT_BUTTON: &str = "✦ Claude 整理";

/// 上方日期（C# `M/d（ddd）` ＋ 今天）。
pub(crate) fn date_text(day: NaiveDate, today: NaiveDate) -> String {
    format!("{}{}", md_weekday(day), if day == today { " 今天" } else { "" })
}

/// 〔複製成文字〕的內容：「任務 時數」清單，貼到日報（C# StringBuilder.AppendLine 是 CRLF）。
pub(crate) fn copy_text(day: NaiveDate, log: &WorkLog) -> String {
    let mut sb = String::new();
    sb.push_str(&format!(
        "{}/{}/{} 工作日誌（共 {}）\r\n",
        day.year(),
        day.month(),
        day.day(),
        WorkMath::hours_minutes(log.total_minutes as f64)
    ));
    for e in &log.entries {
        sb.push_str(&format!("- {}　{}\r\n", e.task, WorkMath::hours_minutes(e.minutes as f64)));
    }
    if !log.markdown.trim().is_empty() {
        sb.push_str("\r\n");
        sb.push_str(log.markdown.trim());
        sb.push_str("\r\n");
    }
    sb
}

/// 草稿上方的說明。
pub(crate) fn draft_info(digest: &ClaudeDigest) -> String {
    if digest.prompt_count + digest.commit_count == 0 {
        "這天在「專案」頁的資料夾裡沒有 Claude Code 對話或 commit，只列出計時的任務。".to_owned()
    } else {
        format!(
            "依這天 {} 句 Claude Code 指令、{} 個 commit 整理；可以先改再複製或存起來。",
            digest.prompt_count, digest.commit_count
        )
    }
}

/// 寫進工時備註前的預覽（只列有要寫的計時段）。
pub(crate) fn notes_preview(entries: &[TimeEntry], digest: &ClaudeDigest) -> String {
    entries
        .iter()
        .filter_map(|x| {
            let note = digest.notes.get(&x.id)?;
            let end = if x.running { "進行中".to_owned() } else { x.ended_at.as_ref().map(hm).unwrap_or_default() };
            Some(format!("{} {}–{}：{note}", x.task, x.started_at.as_ref().map(hm).unwrap_or_default(), end))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Page {
    ctx: WorkCtx,
    day: Cell<NaiveDate>,
    log: RefCell<Option<WorkLog>>,
    digest: RefCell<Option<ClaudeDigest>>,
    entries: RefCell<Vec<TimeEntry>>,
    version: Cell<u64>,
    confirm: Rc<ConfirmSlot>,
}

thread_local! {
    static PAGE: PageCell<Page> = const { PageCell::new() };
}

fn current() -> Option<Rc<Page>> {
    PAGE.with(|p| p.get())
}

fn show(page: &Page, message: Option<&str>, error: bool) {
    if let Some(window) = page.ctx.window() {
        let g = window.global::<WorkLogPageData>();
        g.set_status_text(message.unwrap_or("").into());
        g.set_status_tone(tone(error));
    }
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        day: Cell::new(hb_core::time::today()),
        log: RefCell::new(None),
        digest: RefCell::new(None),
        entries: RefCell::new(Vec::new()),
        version: Cell::new(0),
        confirm: Rc::new(ConfirmSlot::default()),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    w2_common::wire_confirm!(window, WorkLogPageData, page.confirm.clone());
    let g = window.global::<WorkLogPageData>();
    g.set_draft_button(DRAFT_BUTTON.into());
    let move_day = |delta: Option<i64>| {
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
    g.on_prev(move_day(Some(-1)));
    g.on_next(move_day(Some(1)));
    g.on_today(move_day(None));
    g.on_copy(|| {
        let Some(page) = current() else { return };
        let Some(log) = page.log.borrow().clone() else { return };
        match set_clipboard(&copy_text(page.day.get(), &log)) {
            Ok(()) => show(&page, Some("已複製到剪貼簿"), false),
            Err(e) => show(&page, Some(&format!("複製失敗：{e}")), true),
        }
    });
    g.on_draft(|| {
        let Some(page) = current() else { return };
        spawn("整理 Claude 工作日誌", build_draft(page));
    });
    g.on_close_draft(|| {
        if let Some(window) = current().and_then(|p| p.ctx.window()) {
            window.global::<WorkLogPageData>().set_draft_visible(false);
        }
    });
    g.on_copy_draft(|| {
        let Some(page) = current() else { return };
        let Some(window) = page.ctx.window() else { return };
        let text = window.global::<WorkLogPageData>().get_draft_text();
        match set_clipboard(&text) {
            Ok(()) => show(&page, Some("草稿已複製到剪貼簿"), false),
            Err(e) => show(&page, Some(&format!("複製失敗：{e}")), true),
        }
    });
    g.on_capture_draft(|| {
        let Some(page) = current() else { return };
        let Some(window) = page.ctx.window() else { return };
        let g = window.global::<WorkLogPageData>();
        let text = g.get_draft_text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        g.set_capture_enabled(false);
        spawn("存到隨手記", async move {
            page.ctx.engine.quick_capture(&text, false).await;
            if let Some(window) = page.ctx.window() {
                window.global::<WorkLogPageData>().set_capture_enabled(true);
            }
            let error = page.ctx.engine.error();
            show(&page, Some(error.as_deref().unwrap_or("已存到隨手記")), error.is_some());
        });
    });
    g.on_apply_notes(|| {
        let Some(page) = current() else { return };
        let Some(digest) = page.digest.borrow().clone().filter(|d| !d.notes.is_empty()) else { return };
        let Some(window) = page.ctx.window() else { return };
        let preview = notes_preview(&page.entries.borrow(), &digest);
        let p = page.clone();
        w2_common::ask!(
            window,
            WorkLogPageData,
            page.confirm,
            "寫進工時備註",
            format!("寫進這些計時段的備註？（已經有備註的不會動）\n\n{preview}"),
            move || spawn("寫進工時備註", apply_notes(p, digest))
        );
    });
    g.on_open_entries(|| {
        if let Some(page) = current() {
            page.ctx.open_day(page.day.get());
        }
    });
    g.on_open_web(|| {
        if let Some(page) = current() {
            hb_win::open(&page.ctx.engine.web_url("/notebook"));
        }
    });
}

/// arg：日期，或 "draft"（今天，載入後直接整理 Claude 草稿）。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = current() else { return };
    let draft = arg.as_ref().and_then(PageArg::as_text) == Some("draft");
    if let Some(PageArg::Date(d)) = arg {
        page.day.set(d);
    }
    if draft {
        page.day.set(hb_core::time::today());
    }
    load(page.clone()).await;
    if draft && page.log.borrow().is_some() {
        build_draft(page).await;
    }
}

async fn load(page: Rc<Page>) {
    page.version.set(page.version.get() + 1);
    let version = page.version.get();
    let day = page.day.get();
    let today = hb_core::time::today();
    let Some(window) = page.ctx.window() else { return };
    {
        let g = window.global::<WorkLogPageData>();
        g.set_date_text(date_text(day, today).into());
        g.set_today_enabled(day != today);
        g.set_status_text(SharedString::new());
        g.set_draft_visible(false);
        g.set_total_text("讀取中…".into());
    }
    *page.digest.borrow_mut() = None;
    let result = page.ctx.engine.api().get_work_log(day).await;
    let Some(window) = page.ctx.window() else { return };
    let g = window.global::<WorkLogPageData>();
    match result {
        Ok(log) => {
            if version != page.version.get() {
                return;
            }
            render(&g, &log);
            *page.log.borrow_mut() = Some(log);
        }
        Err(e) => {
            log::error!("讀取工作日誌失敗: {}", e.0);
            if version == page.version.get() {
                g.set_total_text("—".into());
                g.set_status_text(format!("讀取失敗：{}", e.0).into());
                g.set_status_tone(0);
            }
        }
    }
}

fn render(g: &WorkLogPageData, log: &WorkLog) {
    g.set_total_text(format!("共 {}", WorkMath::hours_minutes(log.total_minutes as f64)).into());
    let rows: Vec<WorkLogRow> = log
        .entries
        .iter()
        .map(|e| WorkLogRow {
            task: e.task.as_str().into(),
            hours: WorkMath::hours_minutes(e.minutes as f64).into(),
            ranges: e.ranges.iter().map(|(s, e)| format!("{}–{}", hm(s), hm(e))).collect::<Vec<_>>().join("、").into(),
        })
        .collect();
    g.set_entries(model(rows));
    g.set_empty(log.entries.is_empty());
    g.set_note_text(log.markdown.as_str().into());
    g.set_note_visible(!log.markdown.trim().is_empty());
    g.set_copy_enabled(!log.entries.is_empty());
}

async fn build_draft(page: Rc<Page>) {
    let Some(log) = page.log.borrow().clone() else { return };
    let version = page.version.get();
    let day = page.day.get();
    let set_button = |busy: bool| {
        if let Some(window) = page.ctx.window() {
            let g = window.global::<WorkLogPageData>();
            g.set_draft_enabled(!busy);
            g.set_draft_button(if busy { "整理中…" } else { DRAFT_BUTTON }.into());
        }
    };
    set_button(true);
    show(&page, None, true);
    match page.ctx.engine.load_day(day).await {
        Ok((entries, _)) => {
            let digest = page.ctx.engine.build_digest(day, &log.entries, &entries).await;
            if version == page.version.get()
                && let Some(window) = page.ctx.window()
            {
                let g = window.global::<WorkLogPageData>();
                g.set_draft_text(digest.markdown.as_str().into());
                g.set_draft_info(draft_info(&digest).into());
                let n = digest.notes.len();
                g.set_notes_button(
                    if n > 0 {
                        format!("寫進工時備註（{n} 段）")
                    } else {
                        "沒有可寫的工時備註".to_owned()
                    }
                    .into(),
                );
                g.set_notes_enabled(n > 0);
                g.set_draft_visible(true);
                *page.entries.borrow_mut() = entries;
                *page.digest.borrow_mut() = Some(digest);
            }
        }
        Err(e) => {
            log::error!("整理 Claude 工作日誌失敗: {}", e.message());
            show(&page, Some(&format!("整理失敗：{}", e.message())), true);
        }
    }
    set_button(false);
}

async fn apply_notes(page: Rc<Page>, digest: ClaudeDigest) {
    let set_enabled = |enabled: bool| {
        if let Some(window) = page.ctx.window() {
            window.global::<WorkLogPageData>().set_notes_enabled(enabled);
        }
    };
    set_enabled(false);
    page.ctx.engine.set_entry_notes(&digest.notes).await;
    if let Some(error) = page.ctx.engine.error() {
        show(&page, Some(&error), true);
        set_enabled(true);
        return;
    }
    if let Some(window) = page.ctx.window() {
        window.global::<WorkLogPageData>().set_notes_button(format!("已寫入 {} 段備註", digest.notes.len()).into());
    }
    show(&page, Some("已寫進工時備註，可在「工時紀錄」修改"), false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use hb_api::WorkLogEntry;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn 日期文字_今天加註() {
        let today = ymd(2026, 11, 12);
        assert_eq!("11/12（週四） 今天", date_text(today, today));
        assert_eq!("11/11（週三）", date_text(ymd(2026, 11, 11), today));
    }

    #[test]
    fn 複製成文字() {
        let log = WorkLog {
            date: ymd(2026, 11, 12),
            total_minutes: 150,
            entries: vec![
                WorkLogEntry { task: "F2P-1".into(), minutes: 90, ranges: vec![] },
                WorkLogEntry { task: "F2P-2".into(), minutes: 60, ranges: vec![] },
            ],
            markdown: "  今天修了登入  \n".into(),
        };
        assert_eq!(
            "2026/11/12 工作日誌（共 2 小時 30 分）\r\n- F2P-1　1 小時 30 分\r\n- F2P-2　1 小時\r\n\r\n今天修了登入\r\n",
            copy_text(ymd(2026, 11, 12), &log)
        );
        let empty = WorkLog { markdown: " ".into(), entries: vec![], ..log };
        assert_eq!("2026/11/12 工作日誌（共 2 小時 30 分）\r\n", copy_text(ymd(2026, 11, 12), &empty));
    }

    #[test]
    fn 草稿說明與備註預覽() {
        let mut digest =
            ClaudeDigest { markdown: String::new(), notes: Default::default(), prompt_count: 0, commit_count: 0 };
        assert_eq!("這天在「專案」頁的資料夾裡沒有 Claude Code 對話或 commit，只列出計時的任務。", draft_info(&digest));
        digest.prompt_count = 3;
        digest.commit_count = 2;
        assert_eq!("依這天 3 句 Claude Code 指令、2 個 commit 整理；可以先改再複製或存起來。", draft_info(&digest));
        let day = ymd(2026, 11, 12);
        let t = |h, m| Local.from_local_datetime(&day.and_hms_opt(h, m, 0).unwrap()).unwrap().fixed_offset();
        let entries = vec![
            TimeEntry::new("a", "F2P-1", "", Some(t(9, 0)), Some(t(10, 0)), None, false, None),
            TimeEntry::new("b", "F2P-2", "", Some(t(10, 0)), None, None, true, None),
            TimeEntry::new("c", "F2P-3", "", Some(t(11, 0)), Some(t(12, 0)), None, false, None),
        ];
        digest.notes.insert("a".into(), "修登入".into());
        digest.notes.insert("b".into(), "寫測試".into());
        assert_eq!("F2P-1 09:00–10:00：修登入\nF2P-2 10:00–進行中：寫測試", notes_preview(&entries, &digest));
    }
}
