//! 工作台「測試」頁（對應 C# `UI/Work/TestsPage.cs`）。
//!
//! 測試：網頁的三種測試視圖。
//! 需修復（工程師看自己要修的：開始修、送復驗，直接改期限與預估工時）、
//! 需測試（QA 看要驗的：通過、退回；要測的測試任務可以加入測試結果）、
//! 測試總覽（管理者看整體：數字、嚴重度分布、各測試任務進度、子狀態、逾時清單）。
//! 專案有測試子狀態（例 待更新測試機、待上正式機）時：修好了先到下一個子狀態，列表上方有批次按鈕〔已更新到測試機〕〔已更新到正式機〕。
//! 打開這頁才讀；收到測試結果的新通知時（通知本來就在輪詢）才自動重讀，不另外輪詢。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use hb_api::{DateTimeOffset, KeyFinder, TaskKind, TaskKinds, TestInfo, TestOverview, TestResultRow, TestSubStatus};
use hb_core::{TestActionRequest, TimerEngine};
use hb_ui_common::inputs::{self, StatusMessage};
use hb_ui_common::{KeyCopy, TaskText, add_test_result, advance_tests, test_action};
use serde_json::{Map, Value, json};
use slint::{ComponentHandle, Model, SharedString};

use super::hours::w2_common::{self, PageCell, model, project_items, spawn, strings};
use crate::{PageArg, TestsBatch, TestsCard, TestsItem, TestsPageData, TestsProgress, TestsStage, TestsSub, WorkCtx};

pub(crate) const VIEWS: [(&str, &str); 3] = [("fix", "需修復"), ("test", "需測試"), ("overview", "測試總覽")];
pub(crate) const GROUPINGS: [(&str, &str); 5] =
    [("", "不分組"), ("test", "依測試任務"), ("severity", "依嚴重度"), ("assignee", "依執行者"), ("project", "依專案")];
pub(crate) const PERIODS: [(&str, &str); 4] =
    [("30d", "近 30 天"), ("week", "本週"), ("month", "本月"), ("all", "全部")];

/// 工作台頁面的位置（pages::PAGES 的「測試」）。
const PAGE_INDEX: i32 = 2;

fn eq(a: &str, b: &str) -> bool {
    a.to_uppercase() == b.to_uppercase()
}

fn contains_ic(text: &str, q: &str) -> bool {
    text.to_uppercase().contains(&q.to_uppercase())
}

pub(crate) fn severity_rank(s: Option<&str>) -> usize {
    match s {
        None => 9,
        Some(s) => TaskKinds::SEVERITIES.iter().position(|x| *x == s).unwrap_or(8),
    }
}

/// 嚴重的、輪數多的、期限近的排前面。
pub(crate) fn sorted<'a>(rows: impl IntoIterator<Item = &'a TestResultRow>) -> Vec<&'a TestResultRow> {
    let mut list: Vec<&TestResultRow> = rows.into_iter().collect();
    list.sort_by(|a, b| {
        severity_rank(a.severity.as_deref())
            .cmp(&severity_rank(b.severity.as_deref()))
            .then(b.round.cmp(&a.round))
            .then(a.planned_end.unwrap_or(NaiveDate::MAX).cmp(&b.planned_end.unwrap_or(NaiveDate::MAX)))
    });
    list
}

/// 分組的名稱（C# `GroupOf`）。
pub(crate) fn group_of(grouping: &str, r: &TestResultRow) -> String {
    match grouping {
        "test" => match &r.test_task {
            Some(t) => format!("🧪 {t} {}", r.test_task_title.as_deref().unwrap_or("")).trim_end().to_owned(),
            None => "（沒有測試任務）".to_owned(),
        },
        "severity" => r.severity.clone().unwrap_or_else(|| "（沒有嚴重度）".to_owned()),
        "assignee" => {
            if r.assignees.is_empty() {
                "未指派".to_owned()
            } else {
                r.assignees.join("、")
            }
        }
        _ => {
            if r.project.is_empty() {
                "（沒有專案）".to_owned()
            } else {
                r.project.clone()
            }
        }
    }
}

/// 列上淡色說明的文字與顏色（0 淡色、1 警告、2 紅色）。
pub(crate) fn meta(r: &TestResultRow) -> (String, i32) {
    let assignees = if !r.assignees.is_empty() {
        Some(r.assignees.join("、"))
    } else if r.kind == TaskKind::TestResult {
        Some("未指派".to_owned())
    } else {
        None
    };
    let parts: Vec<String> = [
        Some(r.status.clone()),
        r.severity.clone(),
        r.category.clone(),
        assignees,
        r.reporter.as_ref().map(|rep| format!("回報 {rep}")),
        r.stale.then(|| "逾時未處理".to_owned()),
    ]
    .into_iter()
    .flatten()
    .filter(|x| !x.is_empty())
    .collect();
    let tone = if r.round >= 3 || r.stale {
        2
    } else if TaskKinds::severity_brush(r.severity.as_deref()) == "ErrorBrush" {
        1
    } else {
        0
    };
    (format!("　{}", parts.join(" · ")), tone)
}

/// 篩選（名稱或編號、嚴重度、第 3 輪起）。
pub(crate) fn filter_rows(rows: &[TestResultRow], q: &str, severity: Option<&str>, hot: bool) -> Vec<TestResultRow> {
    let q = q.trim();
    rows.iter()
        .filter(|r| q.is_empty() || contains_ic(&r.key, q) || contains_ic(&r.title, q))
        .filter(|r| severity.is_none() || r.severity.as_deref() == severity || r.kind == TaskKind::Test)
        .filter(|r| !hot || r.round >= 3)
        .cloned()
        .collect()
}

/// 清單上的一項：段落標題、段落說明、分組標題、一列。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Entry {
    Section { text: String, key: String },
    Hint(String),
    Group { text: String, key: String },
    Row(Box<TestResultRow>),
}

/// 照選的分組方式列出（C# `AddGroups`）；parent 是收合用的段落代號。
pub(crate) fn add_groups(
    out: &mut Vec<Entry>,
    rows: &[TestResultRow],
    indent: &str,
    parent: &str,
    grouping: &str,
    collapsed: &[String],
) {
    if grouping.is_empty() || rows.len() < 2 {
        out.extend(sorted(rows).into_iter().map(|r| Entry::Row(Box::new(r.clone()))));
        return;
    }
    let mut groups: Vec<(String, Vec<TestResultRow>)> = Vec::new();
    for r in rows {
        let g = group_of(grouping, r);
        match groups.iter_mut().find(|(k, _)| *k == g) {
            Some((_, list)) => list.push(r.clone()),
            None => groups.push((g, vec![r.clone()])),
        }
    }
    if grouping == "severity" {
        groups.sort_by_key(|(_, list)| severity_rank(list[0].severity.as_deref()));
    }
    for (name, list) in groups {
        let key = format!("{parent}|{grouping}|{name}");
        let open = !collapsed.contains(&key);
        out.push(Entry::Group {
            text: format!("{indent}{} {name}　{}", if open { "▾" } else { "▸" }, list.len()),
            key,
        });
        if open {
            out.extend(sorted(&list).into_iter().map(|r| Entry::Row(Box::new(r.clone()))));
        }
    }
}

/// 需修復、需測試的清單與摘要（C# `Render`）。
pub(crate) fn build_list(
    view: &str,
    rows: &[TestResultRow],
    grouping: &str,
    mine: bool,
    collapsed: &[String],
) -> (Vec<Entry>, String) {
    let mut out = Vec::new();
    if view == "fix" {
        add_groups(&mut out, rows, "", "fix", grouping, collapsed);
        let hot = rows.iter().filter(|r| r.round >= 3).count();
        let summary = if rows.is_empty() {
            if mine {
                "沒有你要修的測試結果 🎉".to_owned()
            } else {
                "沒有要修的測試結果".to_owned()
            }
        } else {
            format!(
                "要修 {} 個{}",
                rows.len(),
                if hot > 0 { format!("（第 3 輪起 {hot} 個 🔥）") } else { String::new() }
            )
        };
        return (out, summary);
    }
    let by = |g: &str| rows.iter().filter(|r| r.group == g).cloned().collect::<Vec<_>>();
    let (retest, to_test, tests) = (by("retest"), by("toTest"), by("tests"));
    let mut section = |id: &str, title: String, list: &[TestResultRow], hint: &str| {
        if list.is_empty() {
            return;
        }
        let key = format!("{view}|{id}");
        let open = !collapsed.contains(&key);
        out.push(Entry::Section { text: format!("{} {title}", if open { "▾" } else { "▸" }), key: key.clone() });
        if !open {
            return;
        }
        out.push(Entry::Hint(hint.to_owned()));
        add_groups(&mut out, list, "    ", &key, grouping, collapsed);
    };
    section("retest", format!("待復驗（{}）", retest.len()), &retest, "修好了，等你驗：通過或退回");
    section("toTest", format!("要測的測試結果（{}）", to_test.len()), &to_test, "還沒測或測試中的");
    section(
        "tests",
        format!("要測的測試任務（{}）", tests.len()),
        &tests,
        "未開始、測試中的測試任務：測到問題就加入測試結果",
    );
    let summary = if rows.is_empty() {
        "沒有要測的 🎉".to_owned()
    } else {
        format!("待復驗 {}・要測 {}・測試任務 {}", retest.len(), to_test.len(), tests.len())
    };
    (out, summary)
}

/// `avg.ToString("0.#")`。
pub(crate) fn one_decimal(x: f64) -> String {
    let s = format!("{:.1}", hb_api::json::round_away(x, 1));
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// 測試總覽上方的數字卡（標題、數字、說明、要不要紅色）。
pub(crate) fn overview_cards(o: &TestOverview) -> Vec<(String, String, String, bool)> {
    let mut cards = vec![
        ("未結案".to_owned(), o.open.to_string(), format!("致命 {}・嚴重 {}", o.critical, o.major), o.critical > 0),
        (
            "待復驗".to_owned(),
            o.retest.to_string(),
            o.oldest_retest_days.map(|d| format!("最久等了 {d} 天")).unwrap_or_default(),
            false,
        ),
        ("新增".to_owned(), o.created.to_string(), String::new(), false),
        (
            "結案".to_owned(),
            (o.done + o.wont_fix).to_string(),
            format!("完成 {}・不處理 {}", o.done, o.wont_fix),
            false,
        ),
    ];
    if o.pending_release > 0 || o.sub_statuses.as_ref().is_some_and(|s| !s.is_empty()) {
        cards.push(("待上線".to_owned(), o.pending_release.to_string(), "QA 通過、等部署".to_owned(), false));
    }
    cards.push((
        "平均輪數".to_owned(),
        o.avg_round.map(one_decimal).unwrap_or_else(|| "—".to_owned()),
        format!("第 3 輪起 {}", o.round3_plus),
        o.round3_plus > 0,
    ));
    cards.push(("逾時未處理".to_owned(), o.stale.to_string(), String::new(), o.stale > 0));
    cards
}

/// 子狀態的一段：(階段, [(文字, 有沒有批次按鈕的那個子狀態)])。
/// 子狀態的一段：(階段, [(文字, 有批次按鈕時的那個子狀態)])。
pub(crate) type StageLine = (String, Vec<(String, Option<TestSubStatus>)>);

pub(crate) fn sub_status_lines(subs: &[TestSubStatus], project: Option<&str>) -> Vec<StageLine> {
    let mut stages: Vec<StageLine> = Vec::new();
    for s in subs {
        let count = s.count.unwrap_or(0);
        let text = format!(
            "{} {count}{}",
            s.name,
            match s.oldest_days {
                Some(d) if count > 0 => format!("（最久 {d} 天）"),
                _ => String::new(),
            }
        );
        let button = (s.batch_text().is_some() && count > 0 && project.is_some()).then(|| s.clone());
        match stages.iter_mut().find(|(k, _)| *k == s.stage) {
            Some((_, list)) => list.push((text, button)),
            None => stages.push((s.stage.clone(), vec![(text, button)])),
        }
    }
    stages
}

/// 一列的動作按鈕。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Primary {
    /// 單筆推到下一步（例 待更新測試機 →〔已更新到測試機〕）
    Advance(TestSubStatus),
    /// 修好了：送復驗或〔修好了，待更新〕
    Submit,
    /// 🔧 開始修
    Fix,
    /// ✓ 通過
    Pass,
}

/// 列上主要按鈕的文字與提示。
fn primary_text(engine: &TimerEngine, r: &TestResultRow, p: &Primary) -> (String, String) {
    match p {
        Primary::Advance(step) => {
            (format!("⇢ {}", step.batch_text().unwrap_or_default()), format!("{}：更新好了就推到下一步", step.name))
        }
        Primary::Submit => match engine.fix_done_step(&r.key, &r.status) {
            None => ("✓ 送復驗".into(), "修好了，請回報的人驗".into()),
            Some(next) => (
                "✓ 修好了，待更新".into(),
                format!(
                    "改成{}{}",
                    next.name,
                    next.batch_text().map(|b| format!("，{b}後再送復驗")).unwrap_or_default()
                ),
            ),
        },
        Primary::Fix => ("🔧 開始修".into(), "狀態改成修復中，可以順手填預估工時".into()),
        Primary::Pass => (
            "✓ 通過".into(),
            match engine.pass_step(&r.key, &r.status) {
                Some(next) => format!("復驗通過，改成{}（還不算完成）", next.name),
                None => "復驗通過，結案".into(),
            },
        ),
    }
}

/// 一列在這個視圖的主要動作（測試任務沒有，只有〔加入測試結果〕）。
fn primary_of(engine: &TimerEngine, view: &str, r: &TestResultRow) -> Option<Primary> {
    if r.kind == TaskKind::Test {
        return None;
    }
    if let Some(step) = engine.batch_step(&r.key, &r.status) {
        return Some(Primary::Advance(step));
    }
    if view == "fix" {
        if r.status == TimerEngine::STATUS_FIXING
            || (r.stage_name() == TimerEngine::STATUS_FIXING && r.status != "待修復")
        {
            Some(Primary::Submit)
        } else {
            Some(Primary::Fix)
        }
    } else {
        Some(Primary::Pass)
    }
}

/// 畫面上一列對回的資料。
#[derive(Clone)]
struct RowMeta {
    row: TestResultRow,
    primary: Option<Primary>,
    /// 期限欄位目前存的值（C# DueBox 的 original）
    due: String,
}

struct Page {
    ctx: WorkCtx,
    view: Cell<usize>,
    rows: RefCell<Vec<TestResultRow>>,
    overview: RefCell<Option<TestOverview>>,
    collapsed: RefCell<Vec<String>>,
    loaded: Cell<NaiveDateTime>,
    loaded_key: RefCell<String>,
    seen_notification: Cell<DateTimeOffset>,
    version: Cell<u64>,
    project_keys: RefCell<Vec<String>>,
    /// 清單（list 0）與總覽的逾時清單（list 1）每一項對回的資料
    items: RefCell<[Vec<Option<RowMeta>>; 2]>,
    /// 批次按鈕：(專案, 子狀態, 編號)；總覽子狀態的按鈕放在 sub_buttons
    batches: RefCell<Vec<(String, TestSubStatus, Vec<String>)>>,
    sub_buttons: RefCell<Vec<TestSubStatus>>,
    changed_handler: Cell<Option<hb_core::HandlerId>>,
}

thread_local! {
    static PAGE: PageCell<Page> = const { PageCell::new() };
}

fn current() -> Option<Rc<Page>> {
    PAGE.with(|p| p.get())
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&TestsPageData) -> R) -> Option<R> {
        let window = self.ctx.window()?;
        Some(f(&window.global::<TestsPageData>()))
    }

    fn view(&self) -> &'static str {
        VIEWS[self.view.get().min(VIEWS.len() - 1)].0
    }

    fn selected_project(&self) -> Option<String> {
        let i = self.with(|g| g.get_project_index())?;
        self.project_keys.borrow().get(usize::try_from(i).ok()?).filter(|k| !k.is_empty()).cloned()
    }

    fn show(&self, m: Option<StatusMessage>) {
        let (text, tone) = w2_common::status(m.as_ref());
        self.with(|g| {
            g.set_status_text(text);
            g.set_status_tone(tone);
        });
    }

    fn set_summary(&self, text: &str) {
        self.with(|g| g.set_summary(text.into()));
    }

    fn meta(&self, list: i32, index: i32) -> Option<RowMeta> {
        let items = self.items.borrow();
        items.get(usize::try_from(list).ok()?)?.get(usize::try_from(index).ok()?).cloned().flatten()
    }

    /// 改畫面上的一列（忙碌中的按鈕、期限文字）。
    fn update_row(&self, list: i32, index: i32, f: impl FnOnce(&mut TestsItem)) {
        self.with(|g| {
            let rows = if list == 0 { g.get_items() } else { g.get_stale_rows() };
            if let Some(mut item) = usize::try_from(index).ok().and_then(|i| rows.row_data(i)) {
                f(&mut item);
                rows.set_row_data(index as usize, item);
            }
        });
    }
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        view: Cell::new(0),
        rows: RefCell::new(Vec::new()),
        overview: RefCell::new(None),
        collapsed: RefCell::new(Vec::new()),
        loaded: Cell::new(hb_core::time::MIN),
        loaded_key: RefCell::new(String::new()),
        seen_notification: Cell::new(hb_core::time::now_offset()),
        version: Cell::new(0),
        project_keys: RefCell::new(Vec::new()),
        items: RefCell::new([Vec::new(), Vec::new()]),
        batches: RefCell::new(Vec::new()),
        sub_buttons: RefCell::new(Vec::new()),
        changed_handler: Cell::new(None),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    let engine = ctx.engine.clone();
    let g = window.global::<TestsPageData>();
    g.set_views(strings(VIEWS.iter().map(|v| v.1)));
    g.set_groupings(strings(GROUPINGS.iter().map(|v| v.1)));
    g.set_group_index(1);
    g.set_periods(strings(PERIODS.iter().map(|v| v.1)));
    g.set_severities(strings(std::iter::once("全部嚴重度").chain(TaskKinds::SEVERITIES)));
    g.set_mine(true);

    g.on_view_selected(|i| {
        let Some(page) = current() else { return };
        page.view.set(usize::try_from(i).unwrap_or(0).min(VIEWS.len() - 1));
        spawn("讀取測試視圖", load(page, false));
    });
    g.on_reload_needed(|| {
        if let Some(page) = current() {
            spawn("讀取測試視圖", load(page, false));
        }
    });
    g.on_render_needed(|| {
        if let Some(page) = current() {
            render(&page);
        }
    });
    g.on_refresh(|| {
        if let Some(page) = current() {
            spawn("讀取測試視圖", load(page, true));
        }
    });
    g.on_toggle(|key| {
        let Some(page) = current() else { return };
        {
            let mut collapsed = page.collapsed.borrow_mut();
            let before = collapsed.len();
            collapsed.retain(|k| k != key.as_str());
            if collapsed.len() == before {
                collapsed.push(key.to_string());
            }
        }
        render(&page);
    });
    {
        let engine = engine.clone();
        g.on_copy_key(move |key| KeyCopy::copy(&engine, &key));
    }
    g.on_open_task(|key| {
        if let Some(page) = current() {
            page.ctx.show_page("tasks", Some(PageArg::Text(key.to_string())));
        }
    });
    g.on_row_action(|list, index, id| {
        let Some(page) = current() else { return };
        let Some(meta) = page.meta(list, index) else { return };
        row_action(page, list, index, meta, id.as_str());
    });
    g.on_due_edited(|list, index, text| {
        let Some(page) = current() else { return };
        let Some(meta) = page.meta(list, index) else { return };
        save_due(page, list, index, meta, text.to_string());
    });
    // 回傳 true：存了，輸入框離開焦點（C# Keyboard.ClearFocus）
    g.on_estimate_accepted(|list, index, text| {
        let Some(page) = current() else { return false };
        let Some(meta) = page.meta(list, index) else { return false };
        let text = text.trim().to_owned();
        if text.is_empty() {
            return false;
        }
        let hours = text.parse::<f64>().ok().filter(|h| h.is_finite() && (0.0..=1666.0).contains(h));
        let Some(h) = hours else {
            page.show(Some(StatusMessage::error("預估工時請填小時數，例 2 或 0.5")));
            return false;
        };
        spawn(
            "改預估工時",
            save_field(page, meta.row.key.clone(), "estimatedHours", json!(h), format!("預估工時 {h} 小時")),
        );
        true
    });
    g.on_batch(|i| {
        let Some(page) = current() else { return };
        let Some((project, step, keys)) = usize::try_from(i).ok().and_then(|i| page.batches.borrow().get(i).cloned())
        else {
            return;
        };
        open_advance(&page, &project, &step, Some(keys));
    });
    g.on_sub_batch(|i| {
        let Some(page) = current() else { return };
        let Some(step) = usize::try_from(i).ok().and_then(|i| page.sub_buttons.borrow().get(i).cloned()) else {
            return;
        };
        if let Some(project) = page.selected_project() {
            open_advance(&page, &project, &step, None);
        }
    });

    // 有測試結果的新通知（被退回、待復驗…）時重讀，看到的是別人剛改的；不另外輪詢
    let handler = Rc::new(Cell::new(None::<hb_core::HandlerId>));
    let id = {
        let handler = handler.clone();
        let weak = window.as_weak();
        let engine2 = engine.clone();
        engine.events().changed.subscribe(move |_| {
            let Some(window) = weak.upgrade() else {
                if let Some(id) = handler.take() {
                    engine2.events().changed.unsubscribe(id);
                }
                return;
            };
            if !window.window().is_visible() || window.get_page_index() != PAGE_INDEX {
                return;
            }
            let Some(page) = current() else { return };
            let latest = page
                .ctx
                .engine
                .notifications()
                .iter()
                .filter(|n| n.is_test_result())
                .map(|n| n.created_at)
                .max()
                .unwrap_or_else(hb_api::json::min_value);
            if latest <= page.seen_notification.get() {
                return;
            }
            page.seen_notification.set(latest);
            spawn("讀取測試視圖", load(page, true));
        })
    };
    handler.set(Some(id));
    page.changed_handler.set(Some(id));
}

/// arg：fix、test、overview 切到那個視圖。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = current() else { return };
    render_projects(&page);
    if let Some(view) = arg.as_ref().and_then(PageArg::as_text)
        && let Some(i) = VIEWS.iter().position(|v| v.0 == view)
        && i != page.view.get()
    {
        page.view.set(i);
        page.with(|g| g.set_view_index(i as i32));
    }
    load(page, false).await;
}

fn render_projects(page: &Page) {
    let selected = page.selected_project();
    let items = project_items(&page.ctx.engine, Some("全部專案"));
    let keys: Vec<String> = items.iter().map(|p| p.key.to_string()).collect();
    let index = selected.and_then(|s| keys.iter().position(|k| *k == s)).unwrap_or(0);
    page.with(|g| {
        g.set_projects(model(items));
        g.set_project_index(index as i32);
    });
    *page.project_keys.borrow_mut() = keys;
}

async fn load(page: Rc<Page>, force: bool) {
    let (mine, period_index) = page.with(|g| (g.get_mine(), g.get_period_index())).unwrap_or((true, 0));
    let period = PERIODS[usize::try_from(period_index).unwrap_or(0).min(PERIODS.len() - 1)].0;
    let view = page.view();
    let project = page.selected_project();
    let key = format!("{view}|{}|{}|{period}", project.as_deref().unwrap_or(""), if mine { "True" } else { "False" });
    page.with(|g| g.set_overview_mode(view == "overview"));
    if !force && key == *page.loaded_key.borrow() && hb_core::time::now() - page.loaded.get() < TimeDelta::minutes(1) {
        render(&page);
        return;
    }
    page.version.set(page.version.get() + 1);
    let version = page.version.get();
    page.show(None);
    page.set_summary("讀取中…");
    let engine = page.ctx.engine.clone();
    let result: Result<bool, String> = async {
        if view == "overview" {
            let overview = engine.api().test_overview(project.as_deref(), period).await.map_err(|e| e.0)?;
            if version != page.version.get() {
                return Ok(false);
            }
            *page.overview.borrow_mut() = Some(overview);
        } else {
            let rows = engine.api().list_test_results(view, project.as_deref(), mine).await.map_err(|e| e.0)?;
            // 子狀態（修好了到哪、批次按鈕）看專案設定；每個專案只讀一次
            engine.load_projects_of(rows.iter().map(|r| r.key.as_str())).await;
            if version != page.version.get() {
                return Ok(false);
            }
            *page.rows.borrow_mut() = rows;
        }
        Ok(true)
    }
    .await;
    match result {
        Ok(true) => {
            page.loaded.set(hb_core::time::now());
            *page.loaded_key.borrow_mut() = key;
            render(&page);
        }
        Ok(false) => {}
        Err(message) => {
            if version != page.version.get() {
                return;
            }
            log::error!("讀取測試視圖 {view} 失敗: {message}");
            page.set_summary("");
            page.show(Some(StatusMessage::error(format!("讀取失敗：{message}"))));
        }
    }
}

fn to_item(engine: &TimerEngine, view: &str, e: &Entry) -> (TestsItem, Option<RowMeta>) {
    match e {
        Entry::Section { text, key } => (
            TestsItem { kind: 0, text: text.as_str().into(), toggle_key: key.as_str().into(), ..Default::default() },
            None,
        ),
        Entry::Hint(text) => (TestsItem { kind: 1, text: text.as_str().into(), ..Default::default() }, None),
        Entry::Group { text, key } => (
            TestsItem { kind: 2, text: text.as_str().into(), toggle_key: key.as_str().into(), ..Default::default() },
            None,
        ),
        Entry::Row(r) => {
            let (meta_text, meta_tone) = meta(r);
            let primary = primary_of(engine, view, r);
            let (primary_text, primary_tip) = primary.as_ref().map(|p| primary_text(engine, r, p)).unwrap_or_default();
            let due = r.planned_end.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
            let info =
                TestInfo::new(r.kind, r.category.as_deref(), r.severity.as_deref(), r.round, r.reporter.as_deref());
            let item = TestsItem {
                kind: 3,
                key: r.key.as_str().into(),
                title: TaskText::title(&r.title, &info).into(),
                tip: format!(
                    "{} {}{}\n點編號複製、點名稱看詳細",
                    r.key,
                    r.title,
                    r.test_task
                        .as_ref()
                        .map(|tt| format!("\n測試任務 {tt} {}", r.test_task_title.as_deref().unwrap_or("")))
                        .unwrap_or_default()
                )
                .into(),
                meta: meta_text.into(),
                meta_tone,
                hot: r.round >= 3,
                mode: if r.kind == TaskKind::Test {
                    0
                } else if view == "fix" {
                    1
                } else {
                    2
                },
                due: due.as_str().into(),
                primary_text: primary_text.into(),
                primary_tip: primary_tip.into(),
                ..Default::default()
            };
            (item, Some(RowMeta { row: (**r).clone(), primary, due }))
        }
    }
}

fn render(page: &Page) {
    let engine = page.ctx.engine.clone();
    page.batches.borrow_mut().clear();
    let view = page.view();
    if view == "overview" {
        render_overview(page);
        return;
    }
    let Some((q, severity_index, hot, grouping_index, mine)) = page
        .with(|g| (g.get_filter().to_string(), g.get_severity_index(), g.get_hot(), g.get_group_index(), g.get_mine()))
    else {
        return;
    };
    let severity =
        usize::try_from(severity_index).ok().filter(|i| *i > 0).and_then(|i| TaskKinds::SEVERITIES.get(i - 1).copied());
    let rows = filter_rows(&page.rows.borrow(), &q, severity, hot);
    let batch = batch_buttons(&engine, &rows);
    let grouping = GROUPINGS[usize::try_from(grouping_index).unwrap_or(0).min(GROUPINGS.len() - 1)].0;
    let (entries, summary) = build_list(view, &rows, grouping, mine, &page.collapsed.borrow());
    let (items, metas): (Vec<TestsItem>, Vec<Option<RowMeta>>) =
        entries.iter().map(|e| to_item(&engine, view, e)).unzip();
    page.with(|g| {
        g.set_items(model(items));
        g.set_batches(model(
            batch
                .iter()
                .map(|(text, tip, _)| TestsBatch { text: text.as_str().into(), tip: tip.as_str().into() })
                .collect(),
        ));
        g.set_summary(summary.into());
    });
    page.items.borrow_mut()[0] = metas;
    *page.batches.borrow_mut() = batch.into_iter().map(|(_, _, b)| b).collect();
}

/// 列表上方的批次按鈕：畫面上有可以批次推的子狀態（例 待更新測試機）時，每個專案、每個子狀態一顆，推的是畫面上列出來的那幾筆。
#[allow(clippy::type_complexity)]
fn batch_buttons(
    engine: &TimerEngine,
    rows: &[TestResultRow],
) -> Vec<(String, String, (String, TestSubStatus, Vec<String>))> {
    let mut groups: Vec<(String, TestSubStatus, Vec<String>)> = Vec::new();
    for r in rows.iter().filter(|r| r.kind == TaskKind::TestResult) {
        let Some(step) = engine.batch_step(&r.key, &r.status) else { continue };
        let project = KeyFinder::project_of(&r.key).to_owned();
        match groups.iter_mut().find(|(p, s, _)| *p == project && s.name == step.name) {
            Some((_, _, keys)) => {
                if !keys.iter().any(|k| eq(k, &r.key)) {
                    keys.push(r.key.clone());
                }
            }
            None => groups.push((project, step, vec![r.key.clone()])),
        }
    }
    let mut projects: Vec<&str> = Vec::new();
    for (p, _, _) in &groups {
        if !projects.iter().any(|x| eq(x, p)) {
            projects.push(p);
        }
    }
    let multi = projects.len() > 1;
    groups
        .iter()
        .map(|(project, step, keys)| {
            let text = format!(
                "⇢ {}（{}）{}",
                step.batch_text().unwrap_or_default(),
                keys.len(),
                if multi { format!("　{project}") } else { String::new() }
            );
            let tip = format!("把 {project} 列出來的「{}」一次推到下一步，可以填更新說明（例版本號）", step.name);
            (text, tip, (project.clone(), step.clone(), keys.clone()))
        })
        .collect()
}

fn render_overview(page: &Page) {
    let Some(o) = page.overview.borrow().clone() else { return };
    let engine = page.ctx.engine.clone();
    let project = page.selected_project();
    let period_index = page.with(|g| g.get_period_index()).unwrap_or(0);
    let period_label = PERIODS[usize::try_from(period_index).unwrap_or(0).min(PERIODS.len() - 1)].1;
    let cards = overview_cards(&o);
    let severity_text = if o.by_severity.is_empty() {
        String::new()
    } else {
        format!("嚴重度　{}", o.by_severity.iter().map(|(s, c)| format!("{s} {c}")).collect::<Vec<_>>().join("　"))
    };
    let mut sub_buttons = Vec::new();
    let stages: Vec<TestsStage> = o
        .sub_statuses
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(|subs| {
            sub_status_lines(subs, project.as_deref())
                .into_iter()
                .map(|(stage, list)| TestsStage {
                    stage: stage.into(),
                    subs: model(
                        list.into_iter()
                            .map(|(text, button)| {
                                let (button_text, tip, index) = match &button {
                                    Some(s) => {
                                        sub_buttons.push(s.clone());
                                        (
                                            format!("⇢ {}", s.batch_text().unwrap_or_default()),
                                            format!(
                                                "把 {} 所有「{}」推到下一步",
                                                project.as_deref().unwrap_or(""),
                                                s.name
                                            ),
                                            sub_buttons.len() as i32 - 1,
                                        )
                                    }
                                    None => (String::new(), String::new(), -1),
                                };
                                TestsSub { text: text.into(), button: button_text.into(), tip: tip.into(), index }
                            })
                            .collect(),
                    ),
                })
                .collect()
        })
        .unwrap_or_default();
    let progress: Vec<TestsProgress> = o
        .tests
        .iter()
        .map(|t| TestsProgress {
            key: t.key.as_str().into(),
            title: format!(" {}", t.title).into(),
            ratio: t.passed as f32 / t.total.max(1) as f32,
            counts: format!(
                "全部 {}・通過 {}・修復中 {}・測試中 {}",
                t.total,
                t.passed,
                t.fixing,
                t.testing + t.retest
            )
            .into(),
        })
        .collect();
    let stale: Vec<TestResultRow> =
        sorted(&o.stale_items).into_iter().map(|r| TestResultRow { stale: true, ..r.clone() }).collect();
    let (stale_items, stale_metas): (Vec<TestsItem>, Vec<Option<RowMeta>>) =
        stale.iter().map(|r| to_item(&engine, "overview", &Entry::Row(Box::new(r.clone())))).unzip();
    page.with(|g| {
        g.set_summary(format!("{period_label}・{}", project.as_deref().unwrap_or("全部專案")).into());
        g.set_cards(model(
            cards
                .into_iter()
                .map(|(label, value, sub, alert)| TestsCard {
                    label: label.into(),
                    value: value.into(),
                    sub: sub.into(),
                    alert,
                })
                .collect(),
        ));
        g.set_severity_text(severity_text.into());
        g.set_stages(model(stages));
        g.set_tests_title(format!("各測試任務（{}）", o.tests.len()).into());
        g.set_progress(model(progress));
        g.set_stale_title(if o.stale_items.is_empty() {
            SharedString::new()
        } else {
            format!("逾時未處理（{}）", o.stale_items.len()).into()
        });
        g.set_stale_rows(model(stale_items));
        g.set_items(model(Vec::new()));
        g.set_batches(model(Vec::new()));
    });
    page.items.borrow_mut()[1] = stale_metas;
    *page.sub_buttons.borrow_mut() = sub_buttons;
}

fn row_action(page: Rc<Page>, list: i32, index: i32, meta: RowMeta, id: &str) {
    let r = meta.row.clone();
    match id {
        "add" => {
            let p = page.clone();
            let added: add_test_result::Added = Rc::new(move |key: &str| {
                p.show(Some(StatusMessage::ok(if key.is_empty() {
                    "已加入測試結果".to_owned()
                } else {
                    format!("已加入測試結果 {key}")
                })));
                spawn("讀取測試視圖", load(p.clone(), true));
            });
            add_test_result::open(&page.ctx.engine, &r.key, &r.title, None, &[], Some(added));
        }
        "reopen" => open_dialog(&page, "reopen", &r),
        "primary" => match meta.primary {
            Some(Primary::Advance(step)) => {
                open_advance(&page, KeyFinder::project_of(&r.key), &step, Some(vec![r.key.clone()]))
            }
            Some(Primary::Fix) => open_dialog(&page, "fix", &r),
            Some(Primary::Submit) => {
                let engine = page.ctx.engine.clone();
                spawn(
                    "送復驗",
                    run(page, list, index, async move { engine.submit_retest(&r.key, Some(&r.status)).await }),
                );
            }
            Some(Primary::Pass) => {
                let engine = page.ctx.engine.clone();
                spawn("復驗通過", run(page, list, index, async move { engine.retest(&r.key, true, None).await }));
            }
            None => {}
        },
        other => log::error!("測試頁沒有這個動作：{other}"),
    }
}

/// 單筆動作（C# `RunAsync`）：按鈕變成處理中；成功顯示伺服器的說明並重讀。
async fn run(
    page: Rc<Page>,
    list: i32,
    index: i32,
    action: impl std::future::Future<Output = Result<String, hb_core::CoreError>>,
) {
    let message: Rc<RefCell<String>> = Rc::default();
    let out = message.clone();
    let ok = inputs::run(
        |busy| page.update_row(list, index, |item| item.busy = busy),
        |m| page.show(m),
        || async move {
            *out.borrow_mut() = action.await?;
            Ok(())
        },
        None,
    )
    .await;
    if ok {
        page.show(Some(StatusMessage::ok(message.borrow().clone())));
        load(page, true).await;
    }
}

fn open_dialog(page: &Rc<Page>, action: &str, r: &TestResultRow) {
    let p = page.clone();
    let done: test_action::Done = Rc::new(move |message: &str| {
        p.show(Some(StatusMessage::ok(message)));
        spawn("讀取測試視圖", load(p.clone(), true));
    });
    let request = TestActionRequest { action: action.to_owned(), key: r.key.clone(), title: r.title.clone() };
    test_action::open(&page.ctx.engine, &request, Some(done));
}

fn open_advance(page: &Rc<Page>, project: &str, step: &TestSubStatus, keys: Option<Vec<String>>) {
    let p = page.clone();
    let done: test_action::Done = Rc::new(move |message: &str| {
        p.show(Some(StatusMessage::ok(message)));
        spawn("讀取測試視圖", load(p.clone(), true));
    });
    advance_tests::open(&page.ctx.engine, project, step, keys, Some(done));
}

/// 需修復視圖直接改期限：打完按 Enter 或離開就存（C# `DueBox`）。
fn save_due(page: Rc<Page>, list: i32, index: i32, meta: RowMeta, text: String) {
    let text = text.trim().to_owned();
    if text == meta.due {
        return;
    }
    let mut due: Option<NaiveDate> = None;
    if !text.is_empty() {
        due = inputs::parse_date(Some(&text), None);
        if due.is_none() {
            page.show(Some(StatusMessage::error("期限看不懂，例 2026-11-05、11/5、明天")));
            let original = meta.due.clone();
            page.update_row(list, index, |item| item.due = original.as_str().into());
            return;
        }
    }
    let original = due.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
    if let Some(m) =
        page.items.borrow_mut().get_mut(list as usize).and_then(|l| l.get_mut(index as usize)).and_then(Option::as_mut)
    {
        m.due = original.clone();
    }
    page.update_row(list, index, |item| item.due = original.as_str().into());
    let value = due.map_or(Value::Null, |d| json!(inputs::date_text(Some(d))));
    spawn("改期限", save_field(page, meta.row.key, "plannedEnd", value, "期限".to_owned()));
}

async fn save_field(page: Rc<Page>, key: String, field: &'static str, value: Value, what: String) {
    let message: Rc<RefCell<Option<String>>> = Rc::default();
    let out = message.clone();
    let api = page.ctx.engine.api();
    let k = key.clone();
    let ok = inputs::run(
        |_| {},
        |m| page.show(m),
        || async move {
            let mut fields = Map::new();
            fields.insert(field.into(), value);
            *out.borrow_mut() = w2_common::mcp(api.update_task(&k, fields).await)?;
            Ok(())
        },
        None,
    )
    .await;
    if ok {
        let text = message.borrow().clone().filter(|m| !m.is_empty()).unwrap_or_else(|| format!("{key} 已更新{what}"));
        page.show(Some(StatusMessage::ok(text)));
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        if let Some(id) = self.changed_handler.take() {
            self.ctx.engine.events().changed.unsubscribe(id);
        }
    }
}

#[cfg(test)]
mod checks {
    use super::*;

    fn row(key: &str, severity: Option<&str>, round: i32, group: &str) -> TestResultRow {
        TestResultRow {
            key: key.into(),
            title: format!("問題 {key}"),
            project: "F2P".into(),
            status: "待修復".into(),
            kind: TaskKind::TestResult,
            category: None,
            severity: severity.map(Into::into),
            round,
            assignees: vec![],
            reporter: None,
            planned_end: None,
            test_task: Some("F2P-1".into()),
            test_task_title: Some("登入測試".into()),
            stale: false,
            group: group.into(),
            stage: None,
        }
    }

    fn keys(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Section { text, .. } | Entry::Group { text, .. } => format!("[{text}]"),
                Entry::Hint(t) => format!("({t})"),
                Entry::Row(r) => r.key.clone(),
            })
            .collect()
    }

    #[test]
    fn 排序_嚴重輪數期限() {
        let mut a = row("A", Some("一般"), 1, "fix");
        a.planned_end = NaiveDate::from_ymd_opt(2026, 11, 20);
        let mut b = row("B", Some("一般"), 1, "fix");
        b.planned_end = NaiveDate::from_ymd_opt(2026, 11, 13);
        let rows = vec![
            row("C", None, 5, "fix"),
            a,
            row("D", Some("致命"), 1, "fix"),
            row("E", Some("一般"), 3, "fix"),
            b,
            row("F", Some("怪"), 1, "fix"),
        ];
        let order: Vec<&str> = sorted(&rows).iter().map(|r| r.key.as_str()).collect();
        assert_eq!(vec!["D", "E", "B", "A", "F", "C"], order);
    }

    #[test]
    fn 需修復_分組與收合() {
        let rows = vec![row("A", Some("一般"), 1, "fix"), row("B", Some("致命"), 3, "fix")];
        let (entries, summary) = build_list("fix", &rows, "severity", true, &[]);
        assert_eq!(vec!["[▾ 致命　1]", "B", "[▾ 一般　1]", "A"], keys(&entries));
        assert_eq!("要修 2 個（第 3 輪起 1 個 🔥）", summary);
        let (entries, _) = build_list("fix", &rows, "severity", true, &["fix|severity|致命".to_owned()]);
        assert_eq!(vec!["[▸ 致命　1]", "[▾ 一般　1]", "A"], keys(&entries));
        let (entries, _) = build_list("fix", &rows[..1], "severity", true, &[]);
        assert_eq!(vec!["A"], keys(&entries));
        assert_eq!("沒有你要修的測試結果 🎉", build_list("fix", &[], "", true, &[]).1);
        assert_eq!("沒有要修的測試結果", build_list("fix", &[], "", false, &[]).1);
    }

    #[test]
    fn 需測試_三段() {
        let mut t = row("T", None, 0, "tests");
        t.kind = TaskKind::Test;
        let rows = vec![row("R1", None, 1, "retest"), row("R2", None, 2, "retest"), t];
        let (entries, summary) = build_list("test", &rows, "test", true, &["test|tests".to_owned()]);
        assert_eq!(
            vec![
                "[▾ 待復驗（2）]",
                "(修好了，等你驗：通過或退回)",
                "[    ▾ 🧪 F2P-1 登入測試　2]",
                "R2",
                "R1",
                "[▸ 要測的測試任務（1）]"
            ],
            keys(&entries)
        );
        assert_eq!("待復驗 2・要測 0・測試任務 1", summary);
        assert_eq!("沒有要測的 🎉", build_list("test", &[], "", true, &[]).1);
    }

    #[test]
    fn 列的說明與顏色() {
        let mut r = row("A", Some("嚴重"), 1, "fix");
        r.category = Some("Bug".into());
        r.reporter = Some("Amy".into());
        assert_eq!(("　待修復 · 嚴重 · Bug · 未指派 · 回報 Amy".to_owned(), 1), meta(&r));
        r.assignees = vec!["Bob".into(), "Cat".into()];
        r.stale = true;
        assert_eq!(("　待修復 · 嚴重 · Bug · Bob、Cat · 回報 Amy · 逾時未處理".to_owned(), 2), meta(&r));
        let mut t = row("T", None, 0, "tests");
        t.kind = TaskKind::Test;
        assert_eq!(("　待修復".to_owned(), 0), meta(&t));
    }

    #[test]
    fn 篩選() {
        let mut t = row("F2P-9", None, 0, "tests");
        t.kind = TaskKind::Test;
        let rows = vec![row("F2P-2", Some("致命"), 3, "fix"), row("F2P-3", Some("一般"), 1, "fix"), t];
        let k = |v: Vec<TestResultRow>| v.into_iter().map(|r| r.key).collect::<Vec<_>>();
        assert_eq!(vec!["F2P-2", "F2P-9"], k(filter_rows(&rows, "", Some("致命"), false)));
        assert_eq!(vec!["F2P-2"], k(filter_rows(&rows, "", None, true)));
        assert_eq!(vec!["F2P-3"], k(filter_rows(&rows, "f2p-3", None, false)));
        assert_eq!(vec!["F2P-3"], k(filter_rows(&rows, "問題 F2P-3", None, false)));
    }

    #[test]
    fn 總覽卡片與子狀態() {
        let mut o = TestOverview {
            open: 5,
            critical: 1,
            major: 2,
            retest: 3,
            oldest_retest_days: Some(4),
            created: 7,
            done: 6,
            wont_fix: 1,
            avg_round: Some(1.25),
            round3_plus: 0,
            stale: 0,
            by_severity: vec![],
            tests: vec![],
            stale_items: vec![],
            pending_release: 0,
            sub_statuses: None,
        };
        let cards = overview_cards(&o);
        assert_eq!(6, cards.len());
        assert_eq!(("未結案".to_owned(), "5".to_owned(), "致命 1・嚴重 2".to_owned(), true), cards[0]);
        assert_eq!("最久等了 4 天", cards[1].2);
        assert_eq!(("結案".to_owned(), "7".to_owned(), "完成 6・不處理 1".to_owned(), false), cards[3]);
        assert_eq!(("平均輪數".to_owned(), "1.3".to_owned(), "第 3 輪起 0".to_owned(), false), cards[4]);
        o.avg_round = None;
        o.sub_statuses = Some(vec![
            TestSubStatus {
                count: Some(2),
                oldest_days: Some(3),
                ..TestSubStatus::new("修復中", "待更新測試機", Some("更新到測試機"), None)
            },
            TestSubStatus::new("修復中", "待佈署", None, None),
            TestSubStatus {
                count: Some(1), ..TestSubStatus::new("待復驗", "待上正式機", Some("已上正式機"), None)
            },
        ]);
        let cards = overview_cards(&o);
        assert_eq!(("待上線".to_owned(), "0".to_owned()), (cards[4].0.clone(), cards[4].1.clone()));
        assert_eq!("—", cards[5].1);
        let lines = sub_status_lines(o.sub_statuses.as_deref().unwrap(), Some("F2P"));
        assert_eq!(2, lines.len());
        assert_eq!("修復中", lines[0].0);
        assert_eq!("待更新測試機 2（最久 3 天）", lines[0].1[0].0);
        assert!(lines[0].1[0].1.is_some());
        assert_eq!(("待佈署 0".to_owned(), None), lines[0].1[1].clone());
        assert!(sub_status_lines(o.sub_statuses.as_deref().unwrap(), None)[0].1[0].1.is_none());
        assert_eq!("2", one_decimal(2.0));
    }
}
