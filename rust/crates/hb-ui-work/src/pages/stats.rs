//! 工作台「統計」頁（對應 C# `UI/Work/StatsPage.xaml(.cs)`）。
//!
//! 統計（跟網頁統計頁同一套數字）：依專案或依人、期間（本週…今年或自訂）、各專案／各人的工時長條與比例、明細表，可以匯出 CSV。
//! 打開這頁或換條件才讀，一次一個請求。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, NaiveDate};
use hb_api::json::round_away;
use hb_api::{StatsReport, WorkMath};
use hb_ui_common::date_range::{from_int, to_int, today_int};
use hb_ui_common::inputs::StatusMessage;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use slint::ComponentHandle;

use super::hours::w2_common::{self, PageCell, model, spawn, strings};
use crate::{PageArg, StatsBar, StatsPageData, StatsTableRow, WorkCtx};

pub(crate) const PERSPECTIVES: [(&str, &str); 2] = [("project", "依專案"), ("assignee", "依人")];
pub(crate) const PRESETS: [(&str, &str); 7] = [
    ("thisWeek", "本週"),
    ("lastWeek", "上週"),
    ("thisMonth", "本月"),
    ("lastMonth", "上月"),
    ("last30Days", "最近 30 天"),
    ("thisYear", "今年"),
    ("custom", "自訂"),
];
/// 預設的期間（本月）。
const DEFAULT_PRESET: usize = 2;

/// 去掉小數尾巴的 0（.NET 自訂格式的 `#`）。
fn trimmed(x: f64, digits: usize) -> String {
    let s = format!("{:.digits$}", round_away(x, digits as i32));
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    if s == "-0" { "0".to_owned() } else { s }
}

/// `ToString("0%")`：乘 100、四捨五入到整數。
pub(crate) fn percent_of(ratio: f64) -> String {
    format!("{}%", trimmed(ratio * 100.0, 0))
}

/// C# `Percent`：分母是 0 時是「—」。
pub(crate) fn percent(part: f64, whole: f64) -> String {
    if whole <= 0.0 { "—".to_owned() } else { percent_of(part / whole) }
}

/// C# `Hours`：分鐘 → 小時（最多兩位小數）。
pub(crate) fn hours(minutes: i32) -> String {
    trimmed(minutes as f64 / 60.0, 2)
}

/// 區間的粒度：一個月內一天一欄，四個月內一週一欄，再長一個月一欄（CSV 才不會太寬）。
pub(crate) fn granularity_for(preset: &str, from: Option<NaiveDate>, to: Option<NaiveDate>) -> &'static str {
    match (preset, from, to) {
        ("thisYear", _, _) => "month",
        ("custom", Some(f), Some(t)) => {
            let days = (t - f).num_days();
            if days <= 31 {
                "day"
            } else if days <= 120 {
                "week"
            } else {
                "month"
            }
        }
        _ => "day",
    }
}

/// 明細表（第一列是標題）：畫面上的表與 CSV 共用。工時用小時（小數）。
pub(crate) fn table(r: &StatsReport) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    if r.by_project() {
        rows.push(s(&["專案", "實際（小時）", "預計（小時）", "累計（小時）", "完成／任務", "完成率", "逾期"]));
        for x in &r.rows {
            rows.push(vec![
                x.name.clone(),
                hours(x.actual_minutes),
                hours(x.estimated_minutes),
                hours(x.total_logged_minutes),
                format!("{}/{}", x.done, x.assigned),
                match x.completion_rate {
                    Some(rate) => percent_of(rate),
                    None => percent(x.done as f64, x.assigned as f64),
                },
                x.overdue.to_string(),
            ]);
        }
    } else {
        rows.push(s(&["人", "實際（小時）", "應出勤（小時）", "達成", "工作天", "有計時天數", "完成／指派", "逾期"]));
        for x in &r.rows {
            rows.push(vec![
                x.name.clone(),
                hours(x.actual_minutes),
                hours(x.expected_minutes),
                percent(x.actual_minutes as f64, x.expected_minutes as f64),
                x.workdays.to_string(),
                x.active_days.to_string(),
                format!("{}/{}", x.done, x.assigned),
                x.overdue.to_string(),
            ]);
        }
    }
    rows
}

/// CSV：明細表，空一行，再來每段期間（日／週／月）的工時（列是專案或人，欄是期間，單位小時）。
/// 欄位有逗號、引號、換行時加引號。
pub(crate) fn csv(r: &StatsReport) -> String {
    fn escape(s: &str) -> String {
        if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_owned() }
    }
    let mut sb = String::new();
    let mut line = |cells: &[String]| {
        sb.push_str(&cells.iter().map(|c| escape(c)).collect::<Vec<_>>().join(","));
        sb.push_str("\r\n");
    };
    for row in table(r) {
        line(&row);
    }
    if !r.buckets.is_empty() && !r.cells.is_empty() {
        line(&[]); // 空一行（C# 的 sb.Append 換行）
        let unit = match r.granularity.as_str() {
            "week" => "每週（起日）",
            "month" => "每月",
            _ => "每天",
        };
        let mut header = vec![format!("{}／{unit}（小時）", if r.by_project() { "專案" } else { "人" })];
        header.extend(r.buckets.iter().cloned());
        line(&header);
        let mut by_key: HashMap<&str, HashMap<&str, i32>> = HashMap::new();
        for c in &r.cells {
            by_key.entry(c.key.as_str()).or_default().insert(c.bucket.as_str(), c.minutes);
        }
        for row in r.rows.iter().filter(|x| by_key.contains_key(x.key.as_str())) {
            let cells = &by_key[row.key.as_str()];
            let mut out = vec![row.name.clone()];
            out.extend(r.buckets.iter().map(|b| cells.get(b.as_str()).map_or_else(|| "0".to_owned(), |m| hours(*m))));
            line(&out);
        }
    }
    sb
}

/// 匯出的預設檔名。
pub(crate) fn export_file_name(r: &StatsReport) -> String {
    let range = match (r.from, r.to) {
        (Some(f), Some(t)) => format!("_{}_{}", f.format("%Y-%m-%d"), t.format("%Y-%m-%d")),
        _ => String::new(),
    };
    format!("Hayaku統計_{}{range}.csv", if r.by_project() { "依專案" } else { "依人" })
}

/// 期間文字（C# RangeText）。
pub(crate) fn range_text(r: &StatsReport) -> String {
    match (r.from, r.to) {
        (Some(f), Some(t)) => format!(
            "{} – {}{}",
            f.format("%Y/%m/%d"),
            t.format("%Y/%m/%d"),
            if r.by_project() {
                "　只算已核准的任務；超出權限的專案、人會自動略過"
            } else {
                "　只算已核准的任務；只看得到自己或有權限的人"
            }
        ),
        _ => String::new(),
    }
}

/// 上方四張卡與長條的文字（C# `Render`）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StatsView {
    pub actual: String,
    pub actual_desc: String,
    pub second_label: String,
    pub second: String,
    pub second_desc: String,
    pub done: String,
    pub done_desc: String,
    pub overdue: String,
    pub overdue_alert: bool,
    pub overdue_desc: String,
    pub bars_title: String,
    pub bar_note: String,
    /// (名稱, 時數, 佔比, 長度比例)
    pub bars: Vec<(String, String, String, f32)>,
}

pub(crate) fn view(r: &StatsReport) -> StatsView {
    let total = r.rows.iter().map(|x| x.actual_minutes as i64).sum::<i64>().max(1) as f64;
    let max = r.rows.iter().map(|x| x.actual_minutes).max().unwrap_or(0).max(1) as f64;
    let bars = r
        .rows
        .iter()
        .filter(|x| x.actual_minutes > 0)
        .map(|x| {
            (
                x.name.clone(),
                WorkMath::hours_minutes(x.actual_minutes as f64),
                percent(x.actual_minutes as f64, total),
                (x.actual_minutes as f64 / max) as f32,
            )
        })
        .collect();
    let mut v = StatsView {
        actual: WorkMath::hours_text(r.actual_minutes as f64),
        actual_desc: String::new(),
        second_label: String::new(),
        second: String::new(),
        second_desc: String::new(),
        done: r.done.to_string(),
        done_desc: String::new(),
        overdue: r.overdue.to_string(),
        overdue_alert: r.overdue > 0,
        overdue_desc: String::new(),
        bars_title: String::new(),
        bar_note: String::new(),
        bars,
    };
    if r.by_project() {
        v.actual_desc = "專案裡所有人的計時".into();
        v.second_label = "預計工時".into();
        v.second = WorkMath::hours_text(r.estimated_minutes as f64);
        v.second_desc = "這段期間任務的預計工時".into();
        v.done_desc = if r.task_total > 0 {
            format!("共 {} 個任務，完成率 {}", r.task_total, percent(r.done as f64, r.task_total as f64))
        } else {
            "沒有任務".into()
        };
        v.overdue_desc = "還沒完成、預計結束已過".into();
        v.bars_title = "各專案工時".into();
        v.bar_note = "各專案照實計算；同一個人同時計時兩個專案時兩邊都算。".into();
    } else {
        v.actual_desc = if r.rows.len() > 1 {
            format!("{} 人合計，每人同時計時只算一次", r.rows.len())
        } else {
            "同時計時只算一次".into()
        };
        v.second_label = "應出勤".into();
        v.second = WorkMath::hours_text(r.expected_minutes as f64);
        v.second_desc = if r.rows.len() == 1 {
            format!("{} 個工作天，有計時 {} 天", r.rows[0].workdays, r.rows[0].active_days)
        } else {
            "上班時段扣掉假日與請假".into()
        };
        v.done_desc = format!("指派 {} 個", r.task_total);
        v.overdue_desc = "指派了、還沒完成".into();
        v.bars_title = "各人工時".into();
        v.bar_note = "每個人的實際工時，同時計時只算一次。".into();
    }
    v
}

struct Page {
    ctx: WorkCtx,
    perspective: Cell<usize>,
    preset: Cell<usize>,
    version: Cell<u64>,
    report: RefCell<Option<StatsReport>>,
}

thread_local! {
    static PAGE: PageCell<Page> = const { PageCell::new() };
}

fn current() -> Option<Rc<Page>> {
    PAGE.with(|p| p.get())
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&StatsPageData) -> R) -> Option<R> {
        let window = self.ctx.window()?;
        Some(f(&window.global::<StatsPageData>()))
    }

    fn show(&self, m: Option<StatusMessage>) {
        let (text, tone) = w2_common::status(m.as_ref());
        self.with(|g| {
            g.set_status_text(text);
            g.set_status_tone(tone);
        });
    }
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        perspective: Cell::new(0),
        preset: Cell::new(DEFAULT_PRESET),
        version: Cell::new(0),
        report: RefCell::new(None),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    let g = window.global::<StatsPageData>();
    g.set_perspectives(strings(PERSPECTIVES.iter().map(|p| p.1)));
    g.set_perspective_index(0);
    g.set_presets(strings(PRESETS.iter().map(|p| p.1)));
    g.set_preset_index(DEFAULT_PRESET as i32);
    let today = hb_core::time::today();
    g.set_custom_start(to_int(today.with_day(1)));
    g.set_custom_end(to_int(Some(today)));
    g.set_today(today_int());

    g.on_perspective_selected(|i| {
        let Some(page) = current() else { return };
        page.perspective.set(usize::try_from(i).unwrap_or(0).min(PERSPECTIVES.len() - 1));
        spawn("讀取統計", load(page));
    });
    g.on_preset_selected(|i| {
        let Some(page) = current() else { return };
        page.preset.set(usize::try_from(i).unwrap_or(DEFAULT_PRESET).min(PRESETS.len() - 1));
        spawn("讀取統計", load(page));
    });
    g.on_custom_applied(|| {
        if let Some(page) = current() {
            spawn("讀取統計", load(page));
        }
    });
    g.on_export(|| {
        if let Some(page) = current() {
            export(&page);
        }
    });
    g.on_open_web(|| {
        if let Some(page) = current() {
            hb_win::open(&page.ctx.engine.web_url("/stats"));
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
    page.show(None);
    page.with(|g| {
        for set in
            [StatsPageData::set_actual, StatsPageData::set_second, StatsPageData::set_done, StatsPageData::set_overdue]
        {
            set(g, "…".into());
        }
        g.set_export_enabled(false);
    });
    let preset = PRESETS[page.preset.get()].0;
    let (mut from, mut to) = (None, None);
    if preset == "custom" {
        let (start, end) =
            page.with(|g| (from_int(g.get_custom_start()), from_int(g.get_custom_end()))).unwrap_or((None, None));
        from = start;
        to = end.or(start);
        if from.is_none() {
            page.show(Some(StatusMessage::error("請選自訂的期間")));
            return;
        }
    }
    let perspective = PERSPECTIVES[page.perspective.get()].0;
    let result = page
        .ctx
        .engine
        .api()
        .stats(
            perspective,
            if preset == "custom" { None } else { Some(preset) },
            from,
            to,
            granularity_for(preset, from, to),
        )
        .await;
    match result {
        Ok(report) => {
            if version != page.version.get() {
                return;
            }
            render(&page, &report);
            let has = !report.rows.is_empty();
            *page.report.borrow_mut() = Some(report);
            page.with(|g| g.set_export_enabled(has));
        }
        Err(e) => {
            log::error!("讀取統計失敗: {}", e.0);
            if version == page.version.get() {
                page.show(Some(StatusMessage::error(format!("讀取失敗：{}", e.0))));
            }
        }
    }
}

fn render(page: &Page, r: &StatsReport) {
    let v = view(r);
    let t = table(r);
    page.with(|g| {
        g.set_range_text(range_text(r).into());
        g.set_actual(v.actual.into());
        g.set_actual_desc(v.actual_desc.into());
        g.set_second_label(v.second_label.into());
        g.set_second(v.second.into());
        g.set_second_desc(v.second_desc.into());
        g.set_done(v.done.into());
        g.set_done_desc(v.done_desc.into());
        g.set_overdue(v.overdue.into());
        g.set_overdue_alert(v.overdue_alert);
        g.set_overdue_desc(v.overdue_desc.into());
        g.set_bars_title(v.bars_title.into());
        g.set_bar_note(v.bar_note.into());
        let empty = v.bars.is_empty();
        g.set_bars(model(
            v.bars
                .into_iter()
                .map(|(name, hours, share, ratio)| StatsBar {
                    name: name.into(),
                    hours: hours.into(),
                    share: share.into(),
                    ratio,
                })
                .collect(),
        ));
        g.set_bars_empty(empty);
        g.set_table_empty(t.len() <= 1);
        g.set_table(model(if t.len() <= 1 {
            Vec::new()
        } else {
            t.iter().map(|row| StatsTableRow { cells: strings(row) }).collect()
        }));
    });
}

/// 匯出 CSV（加 BOM，Excel 直接打開中文才不會變亂碼）。
fn export(page: &Page) {
    let Some(r) = page.report.borrow().clone() else { return };
    let mut dialog = rfd::FileDialog::new()
        .set_title("匯出統計")
        .set_file_name(export_file_name(&r))
        .add_filter("CSV 檔案 (*.csv)", &["csv"]);
    if let Some(window) = page.ctx.window() {
        let handle = window.window().window_handle();
        if handle.window_handle().is_ok() && handle.display_handle().is_ok() {
            dialog = dialog.set_parent(&handle);
        }
    }
    let Some(path) = dialog.save_file() else { return };
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(csv(&r).as_bytes());
    match std::fs::write(&path, bytes) {
        Ok(()) => page.show(Some(StatusMessage::ok(format!("已匯出到 {}", path.display())))),
        Err(e) => {
            log::error!("匯出統計失敗: {e}");
            page.show(Some(StatusMessage::error(format!("存檔失敗：{e}"))));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::{StatsCell, StatsRow};

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn row(key: &str, name: &str, actual: i32) -> StatsRow {
        StatsRow {
            key: key.into(),
            name: name.into(),
            actual_minutes: actual,
            estimated_minutes: 0,
            total_logged_minutes: 0,
            expected_minutes: 0,
            workdays: 0,
            active_days: 0,
            assigned: 0,
            done: 0,
            overdue: 0,
            completion_rate: None,
        }
    }

    fn report(perspective: &str, rows: Vec<StatsRow>) -> StatsReport {
        StatsReport {
            perspective: perspective.into(),
            from: Some(ymd(2026, 10, 1)),
            to: Some(ymd(2026, 10, 31)),
            granularity: "week".into(),
            buckets: vec!["2026-09-28".into(), "2026-10-05".into()],
            rows,
            cells: vec![],
            actual_minutes: 0,
            estimated_minutes: 0,
            expected_minutes: 0,
            done: 0,
            task_total: 0,
            overdue: 0,
        }
    }

    #[test]
    fn 粒度() {
        assert_eq!("month", granularity_for("thisYear", None, None));
        assert_eq!("day", granularity_for("thisMonth", None, None));
        assert_eq!("day", granularity_for("custom", Some(ymd(2026, 1, 1)), Some(ymd(2026, 2, 1))));
        assert_eq!("week", granularity_for("custom", Some(ymd(2026, 1, 1)), Some(ymd(2026, 2, 2))));
        assert_eq!("week", granularity_for("custom", Some(ymd(2026, 1, 1)), Some(ymd(2026, 5, 1))));
        assert_eq!("month", granularity_for("custom", Some(ymd(2026, 1, 1)), Some(ymd(2026, 5, 2))));
        assert_eq!("day", granularity_for("custom", None, None));
    }

    #[test]
    fn 小時與百分比() {
        assert_eq!("1.5", hours(90));
        assert_eq!("2", hours(120));
        assert_eq!("0.68", hours(41));
        assert_eq!("0", hours(0));
        assert_eq!("—", percent(1.0, 0.0));
        assert_eq!("33%", percent(1.0, 3.0));
        assert_eq!("3%", percent(1.0, 40.0));
        assert_eq!("100%", percent_of(1.0));
    }

    #[test]
    fn 明細表與_csv_依專案() {
        let mut f2p = row("F2P", "臉部辨識, 一期", 11300);
        f2p.estimated_minutes = 1710;
        f2p.done = 43;
        f2p.assigned = 59;
        f2p.overdue = 11;
        let mut rev = row("REV", "內部", 76);
        rev.completion_rate = Some(0.5);
        let mut r = report("project", vec![f2p, rev]);
        r.cells = vec![StatsCell { key: "F2P".into(), bucket: "2026-09-28".into(), minutes: 90 }];
        let t = table(&r);
        assert_eq!(vec!["專案", "實際（小時）", "預計（小時）", "累計（小時）", "完成／任務", "完成率", "逾期"], t[0]);
        assert_eq!(vec!["臉部辨識, 一期", "188.33", "28.5", "0", "43/59", "73%", "11"], t[1]);
        assert_eq!("50%", t[2][5]);
        assert_eq!(
            "專案,實際（小時）,預計（小時）,累計（小時）,完成／任務,完成率,逾期\r\n\
             \"臉部辨識, 一期\",188.33,28.5,0,43/59,73%,11\r\n\
             內部,1.27,0,0,0/0,50%,0\r\n\
             \r\n\
             專案／每週（起日）（小時）,2026-09-28,2026-10-05\r\n\
             \"臉部辨識, 一期\",1.5,0\r\n",
            csv(&r)
        );
        assert_eq!("Hayaku統計_依專案_2026-10-01_2026-10-31.csv", export_file_name(&r));
        assert_eq!("2026/10/01 – 2026/10/31　只算已核准的任務；超出權限的專案、人會自動略過", range_text(&r));
    }

    #[test]
    fn 依人的卡片與長條() {
        let mut me = row("harry", "Harry", 600);
        me.expected_minutes = 960;
        me.workdays = 2;
        me.active_days = 2;
        let mut r = report("assignee", vec![me, row("amy", "Amy", 0)]);
        r.actual_minutes = 600;
        r.expected_minutes = 960;
        r.task_total = 5;
        r.overdue = 1;
        let v = view(&r);
        assert_eq!(("10h", "2 人合計，每人同時計時只算一次"), (v.actual.as_str(), v.actual_desc.as_str()));
        assert_eq!(
            ("應出勤", "16h", "上班時段扣掉假日與請假"),
            (v.second_label.as_str(), v.second.as_str(), v.second_desc.as_str())
        );
        assert_eq!(("指派 5 個", true), (v.done_desc.as_str(), v.overdue_alert));
        assert_eq!(vec![("Harry".to_owned(), "10 小時".to_owned(), "100%".to_owned(), 1.0)], v.bars);
        assert_eq!(vec!["Harry", "10", "16", "63%", "2", "2", "0/0", "0"], table(&r)[1]);
        r.rows.truncate(1);
        assert_eq!("2 個工作天，有計時 2 天", view(&r).second_desc);
        assert_eq!("Hayaku統計_依人_2026-10-01_2026-10-31.csv", export_file_name(&r));
    }
}
