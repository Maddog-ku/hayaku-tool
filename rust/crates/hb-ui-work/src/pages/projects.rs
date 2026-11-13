//! 工作台「專案」頁（對應 C# `UI/Work/ProjectsPage.cs`）。
//!
//! 專案：任一個專案的全部任務（不只指派給我的），樹狀顯示；可套用專案存好的視圖、篩選、只看我的、只看沒有子任務的，
//! 只顯示已逾期，勾選多個一次改狀態、優先級、類型或套用子任務範本。點編號複製編號，點名稱在「我的任務」的詳細打開。
//! 跟網頁一樣不列測試結果，測試任務後面標底下測試結果的數量（全部、通過、修復中、測試中）。
//! 任務樹與「建立任務」小視窗共用快取（10 分鐘內不重抓，按重新整理才重讀）；視圖的篩選能在本機套用就不另外打請求。
//!
//! 右鍵選單：C# `TaskMenu.Build`。`@hbc` 的 TaskMenuArea 綁在 hb-ui-common 的 `TaskMenuState` global 上，
//! 整個工作台視窗共用一份（由外框分派）；為了不跟別頁互相蓋掉，projects.slint 自己放一份同樣的選單，
//! 內容綁在 `ProjectsPageData`（欄位名稱跟 TaskMenuState 一樣），用 `wire_task_menu!` 接到 hb-ui-common 的 TaskMenu。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use chrono::NaiveDate;
use hb_api::{BatchResult, KeyFinder, ProjectDetail, ProjectTask, ProjectView, TaskKind, TaskKinds};
use hb_core::{TaskTree, TaskTreeNode, ViewFilter};
use hb_ui_common::inputs::{self, StatusMessage};
use hb_ui_common::{KeyCopy, TaskMenuHooks, TaskText, apply_template, batch_create, delete_task};
use serde_json::{Map, Value, json};
use slint::{ComponentHandle, SharedString};

use super::hours::w2_common::{self, PageCell, md, model, project_items, spawn, strings};
use crate::{PageArg, ProjectsPageData, ProjectsRow, WorkCtx};

const ALL_VIEW: &str = "";
pub(crate) const PRIORITIES: [&str; 5] = ["緊急", "高", "中", "低", "無"];
pub(crate) const BATCH_TYPES: [&str; 3] = ["一般", "測試", "測試結果"];
pub(crate) const TYPE_FILTERS: [&str; 3] = ["全部類型", "一般", "測試"];

/// 任務多（超過這個數）時預設只展開最上層。
const OPEN_ALL_LIMIT: usize = 150;

fn eq(a: &str, b: &str) -> bool {
    a.to_uppercase() == b.to_uppercase()
}

fn contains_ic(text: &str, q: &str) -> bool {
    text.to_uppercase().contains(&q.to_uppercase())
}

/// `Uri.EscapeDataString`（只留 RFC 3986 的非保留字元）。
pub(crate) fn escape_data_string(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 篩選條件（畫面上的勾選與輸入）。
#[derive(Debug, Clone, Default)]
pub(crate) struct Filters {
    /// 0 全部類型、1 一般、2 測試
    pub type_index: i32,
    pub mine: bool,
    pub overdue_only: bool,
    pub hide_done: bool,
    pub leaf_only: bool,
    pub filter: String,
}

/// 清單的一列（還沒轉成 Slint）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ListRow {
    pub task: ProjectTask,
    pub depth: i32,
    pub has_children: bool,
    pub open: bool,
}

/// 收合狀態：使用者收起、點開的優先；有篩選或任務不多時全部展開；不然只展開最上層。
#[derive(Debug, Clone, Default)]
pub(crate) struct Expansion {
    pub collapsed: Vec<String>,
    pub expanded: Vec<String>,
}

impl Expansion {
    fn has(list: &[String], key: &str) -> bool {
        list.iter().any(|k| eq(k, key))
    }

    pub fn is_open(&self, key: &str, depth: i32, filtering: bool, total: usize) -> bool {
        !Self::has(&self.collapsed, key)
            && (Self::has(&self.expanded, key) || filtering || total <= OPEN_ALL_LIMIT || depth == 0)
    }

    pub fn toggle(&mut self, key: &str, open: bool) {
        self.collapsed.retain(|k| !eq(k, key));
        self.expanded.retain(|k| !eq(k, key));
        if open {
            self.collapsed.push(key.to_owned());
        } else {
            self.expanded.push(key.to_owned());
        }
    }
}

/// C# `Render` 的篩選與組樹：回傳 (要畫的列、不含測試結果的總數、測試任務 → 底下的測試結果)。
pub(crate) fn build_rows(
    tasks: &[ProjectTask],
    view_tasks: Option<&[ProjectTask]>,
    f: &Filters,
    me: Option<&str>,
    today: NaiveDate,
    expansion: &Expansion,
) -> (Vec<ListRow>, usize, HashMap<String, Vec<ProjectTask>>) {
    let base = view_tasks.unwrap_or(tasks);
    // 跟網頁的列表一樣不列測試結果（在測試任務後面標數量）
    let source: Vec<&ProjectTask> = base.iter().filter(|t| t.kind() != TaskKind::TestResult).collect();
    let mut results: HashMap<String, Vec<ProjectTask>> = HashMap::new();
    let mut seen: Vec<String> = Vec::new();
    for t in base.iter().chain(tasks.iter()).filter(|t| t.kind() == TaskKind::TestResult) {
        let Some(parent) = &t.parent else { continue };
        if seen.iter().any(|k| eq(k, &t.key)) {
            continue;
        }
        seen.push(t.key.clone());
        results.entry(parent.to_uppercase()).or_default().push(t.clone());
    }
    let mut shown: Vec<&ProjectTask> = source.clone();
    if f.type_index == 1 {
        shown.retain(|t| t.kind() == TaskKind::Normal);
    } else if f.type_index == 2 {
        shown.retain(|t| t.kind() == TaskKind::Test);
    }
    if f.mine
        && let Some(me) = me
    {
        shown.retain(|t| t.assignees.as_ref().is_some_and(|a| a.iter().any(|x| eq(x, me))));
    }
    if f.overdue_only {
        shown.retain(|t| TaskTree::is_overdue(t, today)); // 上層沒逾期的，逾期的子任務放到最上層
    }
    let include_done = !f.hide_done;
    let filter = f.filter.trim();
    let mut rows = Vec::new();
    if f.leaf_only {
        // 沒有子任務的（用整個專案判斷，不受篩選影響），平鋪列出
        let parents: Vec<String> = tasks
            .iter()
            .filter(|t| t.kind() != TaskKind::TestResult)
            .filter_map(|t| t.parent.as_ref().map(|p| p.to_uppercase()))
            .collect();
        for t in shown {
            if parents.contains(&t.key.to_uppercase()) || t.is_category {
                continue;
            }
            if !include_done && matches!(t.status_category.as_str(), "DONE" | "CANCELLED") {
                continue;
            }
            if !filter.is_empty() && !contains_ic(&t.key, filter) && !contains_ic(&t.title, filter) {
                continue;
            }
            rows.push(ListRow { task: t.clone(), depth: 0, has_children: false, open: false });
        }
    } else {
        let roots = TaskTree::build(shown.into_iter().cloned(), include_done, Some(filter));
        let total = base.len();
        fn add(
            rows: &mut Vec<ListRow>,
            nodes: &[TaskTreeNode],
            depth: i32,
            e: &Expansion,
            filtering: bool,
            total: usize,
        ) {
            for n in nodes {
                let has_children = !n.children.is_empty();
                let open = has_children && e.is_open(&n.task.key, depth, filtering, total);
                rows.push(ListRow { task: n.task.clone(), depth, has_children, open });
                if open {
                    add(rows, &n.children, depth + 1, e, filtering, total);
                }
            }
        }
        add(&mut rows, &roots, 0, expansion, !filter.is_empty(), total);
    }
    (rows, source.len(), results)
}

/// 摘要文字。
pub(crate) fn summary(total: usize, shown: usize) -> String {
    if total == 0 {
        "這個專案還沒有任務".to_owned()
    } else if shown == 0 {
        "沒有符合的任務".to_owned()
    } else {
        format!("顯示 {shown} 個（共 {total} 個）")
    }
}

/// 測試任務後面的數量與提示（全部、通過、修復中、測試中）。
pub(crate) fn results_text(results: &[ProjectTask]) -> (String, String) {
    let n = |bucket: &str| {
        results.iter().filter(|r| TaskKinds::bucket(&r.status, Some(&r.status_category), None) == bucket).count()
    };
    let testing = n("testing") + n("retest");
    (
        format!("  🐞{} ✓{} 🔧{} 🧪{}", results.len(), n("pass"), n("fixing"), testing),
        format!("測試結果 全部 {}、通過 {}、修復中 {}、測試中 {}", results.len(), n("pass"), n("fixing"), testing),
    )
}

/// 批次修改的結果訊息與是否有失敗。
pub(crate) fn batch_message(results: &[BatchResult], what: &str) -> (String, bool) {
    let failed: Vec<&BatchResult> = results.iter().filter(|r| r.error.is_some()).collect();
    let requested =
        results.iter().filter(|r| r.error.is_none() && r.message.as_deref().is_some_and(|m| !m.is_empty())).count();
    let mut text = format!("{} 個{what}", results.len() - failed.len());
    if requested > 0 {
        text.push_str(&format!("（其中 {requested} 個送出變更申請）"));
    }
    if !failed.is_empty() {
        text.push_str(&format!(
            "；{} 個失敗：{}",
            failed.len(),
            failed
                .iter()
                .take(5)
                .map(|f| format!("{} {}", f.task, f.error.as_deref().unwrap_or("")))
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    (text, !failed.is_empty())
}

struct Page {
    ctx: WorkCtx,
    /// 專案下拉的 key（跟畫面的順序一樣）；None 是還沒建過（C# `_project.ItemsSource is null`）
    project_keys: RefCell<Option<Vec<String>>>,
    views: RefCell<HashMap<String, Vec<ProjectView>>>,
    /// 視圖下拉的項目（第一個是全部任務）
    view_options: RefCell<Vec<ProjectView>>,
    selected: RefCell<Vec<String>>,
    expansion: RefCell<Expansion>,
    tasks: RefCell<Vec<ProjectTask>>,
    view_tasks: RefCell<Option<Vec<ProjectTask>>>,
    detail: RefCell<Option<Rc<ProjectDetail>>>,
    /// 畫面上的列（右鍵、勾選、收合時對回任務）
    rows: RefCell<Vec<ListRow>>,
    version: Cell<u64>,
}

thread_local! {
    static PAGE: PageCell<Page> = const { PageCell::new() };
}

fn current() -> Option<Rc<Page>> {
    PAGE.with(|p| p.get())
}

impl Page {
    fn selected_project(&self) -> Option<String> {
        let window = self.ctx.window()?;
        let i = window.global::<ProjectsPageData>().get_project_index();
        self.project_keys.borrow().as_ref()?.get(usize::try_from(i).ok()?).cloned()
    }

    fn show(&self, message: Option<StatusMessage>) {
        if let Some(window) = self.ctx.window() {
            let (text, tone) = w2_common::status(message.as_ref());
            let g = window.global::<ProjectsPageData>();
            g.set_status_text(text);
            g.set_status_tone(tone);
        }
    }

    fn set_summary(&self, text: &str) {
        if let Some(window) = self.ctx.window() {
            window.global::<ProjectsPageData>().set_summary(text.into());
        }
    }

    fn filters(&self) -> Filters {
        let Some(window) = self.ctx.window() else { return Filters::default() };
        let g = window.global::<ProjectsPageData>();
        Filters {
            type_index: g.get_type_index(),
            mine: g.get_mine(),
            overdue_only: g.get_overdue_only(),
            hide_done: g.get_hide_done(),
            leaf_only: g.get_leaf_only(),
            filter: g.get_filter().to_string(),
        }
    }

    fn update_batch(&self) {
        if let Some(window) = self.ctx.window() {
            let g = window.global::<ProjectsPageData>();
            let n = self.selected.borrow().len();
            g.set_batch_visible(n > 0);
            g.set_batch_text(format!("已選 {n} 個").into());
        }
    }

    fn is_selected(&self, key: &str) -> bool {
        self.selected.borrow().iter().any(|k| eq(k, key))
    }
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        project_keys: RefCell::new(None),
        views: RefCell::new(HashMap::new()),
        view_options: RefCell::new(Vec::new()),
        selected: RefCell::new(Vec::new()),
        expansion: RefCell::new(Expansion::default()),
        tasks: RefCell::new(Vec::new()),
        view_tasks: RefCell::new(None),
        detail: RefCell::new(None),
        rows: RefCell::new(Vec::new()),
        version: Cell::new(0),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    let engine = ctx.engine.clone();
    let g = window.global::<ProjectsPageData>();
    g.set_hide_done(engine.config().tree_hide_done);
    g.set_type_items(strings(TYPE_FILTERS));
    g.set_priorities(strings(PRIORITIES));
    g.set_batch_types(strings(BATCH_TYPES));

    g.on_project_selected(|_| {
        if let Some(page) = current() {
            spawn("切換專案", load(page, false));
        }
    });
    g.on_view_selected(|_| {
        if let Some(page) = current() {
            spawn("套用視圖", apply_view(page));
        }
    });
    g.on_filters_changed(|| {
        if let Some(page) = current() {
            render(&page);
        }
    });
    g.on_hide_done_toggled(|on| {
        let Some(page) = current() else { return };
        page.ctx.engine.config_mut().tree_hide_done = on;
        page.ctx.engine.config().save();
        render(&page);
    });
    g.on_refresh(|| {
        if let Some(page) = current() {
            spawn("重新整理專案", load(page, true));
        }
    });
    g.on_create(|| {
        if let Some(page) = current() {
            page.ctx.show_page("tasks", Some(PageArg::Text("new".into())));
        }
    });
    g.on_batch_create(|| {
        let Some(page) = current() else { return };
        let Some(key) = page.selected_project() else { return };
        let parent = {
            let selected = page.selected.borrow();
            if selected.len() == 1 { selected.first().cloned() } else { None }
        };
        batch_create::open(
            &page.ctx.engine,
            &key,
            parent.as_deref(),
            Some(Rc::new(|| {
                if let Some(page) = current() {
                    spawn("重新整理專案", load(page, true));
                }
            })),
        );
    });
    g.on_open_web(|| {
        let Some(page) = current() else { return };
        if let Some(key) = page.selected_project() {
            hb_win::open(&page.ctx.engine.web_url(&format!("/projects/{}", escape_data_string(&key))));
        }
    });
    g.on_toggle_row(|i| {
        let Some(page) = current() else { return };
        let Some(row) = usize::try_from(i).ok().and_then(|i| page.rows.borrow().get(i).cloned()) else { return };
        page.expansion.borrow_mut().toggle(&row.task.key, row.open);
        render(&page);
    });
    g.on_check_row(|i, checked| {
        let Some(page) = current() else { return };
        let Some(key) = usize::try_from(i).ok().and_then(|i| page.rows.borrow().get(i).map(|r| r.task.key.clone()))
        else {
            return;
        };
        {
            let mut selected = page.selected.borrow_mut();
            selected.retain(|k| !eq(k, &key));
            if checked {
                selected.push(key);
            }
        }
        page.update_batch();
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
    g.on_apply_status(|| {
        let Some(page) = current() else { return };
        let Some(window) = page.ctx.window() else { return };
        let g = window.global::<ProjectsPageData>();
        let statuses = g.get_batch_statuses();
        let Some(status) =
            usize::try_from(g.get_batch_status_index()).ok().and_then(|i| slint::Model::row_data(&statuses, i))
        else {
            return;
        };
        let status = status.to_string();
        spawn("批次改狀態", batch(page, "status", status.clone(), format!("狀態改成「{status}」")));
    });
    g.on_apply_priority(|| {
        let Some(page) = current() else { return };
        let Some(window) = page.ctx.window() else { return };
        let i = window.global::<ProjectsPageData>().get_batch_priority_index();
        let Some(p) = usize::try_from(i).ok().and_then(|i| PRIORITIES.get(i)) else { return };
        spawn("批次改優先級", batch(page, "priority", (*p).to_owned(), format!("優先級改成「{p}」")));
    });
    g.on_apply_type(|| {
        let Some(page) = current() else { return };
        let Some(window) = page.ctx.window() else { return };
        let i = window.global::<ProjectsPageData>().get_batch_type_index();
        let Some(t) = usize::try_from(i).ok().and_then(|i| BATCH_TYPES.get(i)) else { return };
        spawn("批次改類型", batch(page, "type", (*t).to_owned(), format!("類型改成「{t}」")));
    });
    g.on_apply_template(|| {
        let Some(page) = current() else { return };
        let Some(key) = page.selected_project() else { return };
        let tasks = page.selected.borrow().clone();
        if tasks.is_empty() {
            return;
        }
        apply_template::open(
            &page.ctx.engine,
            &key,
            tasks,
            None,
            Box::new(|applied| {
                if !applied {
                    return;
                }
                if let Some(page) = current() {
                    page.selected.borrow_mut().clear();
                    spawn("重新整理專案", load(page, true));
                }
            }),
        );
    });
    g.on_clear_selection(|| {
        if let Some(page) = current() {
            page.selected.borrow_mut().clear();
            render(&page);
        }
    });

    // 右鍵：計時、改狀態、釘選、查看、刪除…
    let hooks = TaskMenuHooks {
        open: Rc::new(|key| {
            if let Some(page) = current() {
                page.ctx.show_page("tasks", Some(PageArg::Text(key.to_owned())));
            }
        }),
        delete: Rc::new(|key| {
            let Some(page) = current() else { return };
            let (title, children) = {
                let tasks = page.tasks.borrow();
                let title = tasks.iter().find(|t| eq(&t.key, key)).map(|t| t.title.clone()).unwrap_or_default();
                let children = tasks.iter().filter(|c| c.parent.as_deref().is_some_and(|p| eq(p, key))).count() as i32;
                (title, children)
            };
            let key = key.to_owned();
            spawn("刪除任務", async move {
                let window = page.ctx.window();
                let owner = window.as_ref().map(|w| w.window());
                if delete_task::confirm(&page.ctx.engine, &key, &title, Some(children), owner).await {
                    load(page.clone(), true).await;
                }
            });
        }),
        template_applied: Some(Rc::new(|| {
            if let Some(page) = current() {
                spawn("重新整理專案", load(page, true));
            }
        })),
    };
    let _menu = hb_ui_common::wire_task_menu!(window, ProjectsPageData, engine, hooks);
}

/// arg：專案代碼（例 F2P）時切到那個專案。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = current() else { return };
    let first = page.project_keys.borrow().is_none();
    let wanted = arg.as_ref().and_then(PageArg::as_text).map(str::to_owned);
    render_projects(&page, wanted.as_deref());
    if first || wanted.is_some() {
        load(page, false).await;
    }
}

fn render_projects(page: &Page, wanted: Option<&str>) {
    let selected = wanted.map(str::to_owned).or_else(|| page.selected_project());
    let engine = &page.ctx.engine;
    let items = project_items(engine, None);
    let keys: Vec<String> = items.iter().map(|p| p.key.to_string()).collect();
    let running: Vec<String> = engine.running_keys().iter().map(|k| KeyFinder::project_of(k).to_owned()).collect();
    let index = selected
        .as_deref()
        .and_then(|s| keys.iter().position(|k| eq(k, s)))
        .or_else(|| keys.iter().position(|k| running.iter().any(|r| r == k)))
        .unwrap_or(0);
    let Some(window) = page.ctx.window() else { return };
    let g = window.global::<ProjectsPageData>();
    g.set_projects(model(items));
    g.set_project_index(index as i32);
    if keys.is_empty() {
        g.set_summary("還沒有讀到專案，稍後再試或按重新整理".into());
    }
    *page.project_keys.borrow_mut() = Some(keys);
}

async fn load(page: Rc<Page>, fresh: bool) {
    let Some(project) = page.selected_project() else { return };
    page.version.set(page.version.get() + 1);
    let version = page.version.get();
    page.show(None);
    page.set_summary("讀取中…");
    let engine = page.ctx.engine.clone();
    if fresh {
        engine.forget_project_tree(&project);
    }
    let result: Result<(), String> = async {
        let tasks = engine.project_tree(&project, false).await.map_err(|e| e.0)?;
        let detail = engine.get_project(&project).await.map_err(|e| e.message().to_owned())?;
        let cached = page.views.borrow().get(&project.to_uppercase()).cloned();
        let views = match cached {
            Some(v) if !fresh => v,
            _ => {
                let v = match engine.api().list_views(&project).await {
                    Ok(v) => v,
                    Err(e) => {
                        log::error!("讀取 {project} 的視圖失敗: {}", e.0);
                        Vec::new()
                    }
                };
                page.views.borrow_mut().insert(project.to_uppercase(), v.clone());
                v
            }
        };
        if version != page.version.get() {
            return Ok(());
        }
        *page.tasks.borrow_mut() = tasks;
        page.selected.borrow_mut().clear();
        let Some(window) = page.ctx.window() else { return Ok(()) };
        let g = window.global::<ProjectsPageData>();
        let previous = {
            let options = page.view_options.borrow();
            usize::try_from(g.get_view_index()).ok().and_then(|i| options.get(i)).map(|v| v.id.clone())
        };
        let mut options = vec![ProjectView {
            id: ALL_VIEW.to_owned(),
            name: "全部任務".to_owned(),
            r#type: String::new(),
            is_default: false,
            filter: Vec::new(),
            has_other_conditions: false,
        }];
        options.extend(views);
        let index = previous.and_then(|p| options.iter().position(|v| v.id == p)).unwrap_or(0);
        g.set_views(strings(options.iter().map(|v| v.name.clone())));
        g.set_view_index(index as i32);
        *page.view_options.borrow_mut() = options;
        g.set_batch_statuses(strings(detail.statuses.iter().map(|s| s.name.clone())));
        g.set_batch_status_index(-1);
        *page.detail.borrow_mut() = Some(detail);
        Ok(())
    }
    .await;
    match result {
        Ok(()) => {
            if version == page.version.get() {
                apply_view(page).await;
            }
        }
        Err(message) => {
            if version != page.version.get() {
                return;
            }
            log::error!("讀取 {project} 的任務失敗: {message}");
            page.set_summary("");
            page.show(Some(StatusMessage::error(format!("讀取失敗：{message}"))));
        }
    }
}

/// 套用視圖：條件都認得就在本機篩，不然請伺服器用 view 篩（只有這種情況多一個請求）。
async fn apply_view(page: Rc<Page>) {
    let view = {
        let Some(window) = page.ctx.window() else { return };
        let i = window.global::<ProjectsPageData>().get_view_index();
        usize::try_from(i).ok().and_then(|i| page.view_options.borrow().get(i).cloned())
    };
    let Some(view) = view.filter(|v| v.id != ALL_VIEW) else {
        *page.view_tasks.borrow_mut() = None;
        render(&page);
        return;
    };
    let me = page.ctx.engine.user().map(|u| u.username);
    let local = ViewFilter::apply(page.tasks.borrow().iter(), &view, me.as_deref());
    let need_server = local.is_none();
    *page.view_tasks.borrow_mut() = local;
    if need_server && let Some(project) = page.selected_project() {
        let version = page.version.get();
        page.set_summary("套用視圖中…");
        match page.ctx.engine.api().list_project_tasks(&project, Some(&view.id)).await {
            Ok(filtered) => {
                if version != page.version.get() {
                    return;
                }
                *page.view_tasks.borrow_mut() = Some(filtered);
            }
            Err(e) => {
                log::error!("套用視圖 {} 失敗: {}", view.name, e.0);
                page.show(Some(StatusMessage::error(format!("套用視圖失敗：{}", e.0))));
            }
        }
    }
    render(&page);
}

fn render(page: &Page) {
    let Some(window) = page.ctx.window() else { return };
    let g = window.global::<ProjectsPageData>();
    let me = page.ctx.engine.user().map(|u| u.username);
    let today = hb_core::time::today();
    let (rows, total, results) = {
        let tasks = page.tasks.borrow();
        let view_tasks = page.view_tasks.borrow();
        build_rows(&tasks, view_tasks.as_deref(), &page.filters(), me.as_deref(), today, &page.expansion.borrow())
    };
    let detail = page.detail.borrow().clone();
    let slint_rows: Vec<ProjectsRow> = rows
        .iter()
        .map(|r| {
            let t = &r.task;
            let (results_text, results_tip) = match results.get(&t.key.to_uppercase()) {
                Some(list) if t.kind() == TaskKind::Test => results_text(list),
                _ => (String::new(), String::new()),
            };
            let people: Vec<String> = t
                .assignees
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|a| {
                    detail
                        .as_ref()
                        .and_then(|d| d.members.iter().find(|m| eq(&m.username, a)).map(|m| m.display_name.clone()))
                        .unwrap_or_else(|| a.clone())
                })
                .collect();
            let overdue = TaskTree::is_overdue(t, today);
            ProjectsRow {
                key: t.key.as_str().into(),
                title: TaskText::title(&t.title, &t.test).into(),
                tip: format!("{} {}\n點編號複製、點名稱看詳細", t.key, t.title).into(),
                depth: r.depth,
                has_children: r.has_children,
                open: r.open,
                checked: page.is_selected(&t.key),
                is_category: t.is_category,
                results: results_text.into(),
                results_tip: results_tip.into(),
                done: matches!(t.status_category.as_str(), "DONE" | "CANCELLED"),
                status: t.status.as_str().into(),
                in_progress: t.status_category == "IN_PROGRESS",
                assignees: if people.is_empty() { "—".into() } else { people.join("、").into() },
                assignees_tip: if people.is_empty() { "未指派".into() } else { people.join("、").into() },
                due: t.planned_end.map(md).unwrap_or_default().into(),
                due_tip: match t.planned_end {
                    None => SharedString::new(),
                    Some(_) if overdue => "預計結束已過".into(),
                    Some(_) => "預計結束".into(),
                },
                overdue,
            }
        })
        .collect();
    g.set_summary(summary(total, rows.len()).into());
    g.set_rows(model(slint_rows));
    *page.rows.borrow_mut() = rows;
    page.update_batch();
}

/// 批次修改（C# `BatchAsync`）：最多 100 個，逐筆成功或失敗。
async fn batch(page: Rc<Page>, field: &'static str, value: String, what: String) {
    let Some(project) = page.selected_project() else { return };
    let items: Vec<Map<String, Value>> = page
        .selected
        .borrow()
        .iter()
        .take(100)
        .map(|k| {
            let mut m = Map::new();
            m.insert("task".into(), json!(k));
            m.insert(field.into(), json!(value));
            m
        })
        .collect();
    if items.is_empty() {
        return;
    }
    let results: Rc<RefCell<Vec<BatchResult>>> = Rc::default();
    let api = page.ctx.engine.api();
    let busy = {
        let page = page.clone();
        move |busy: bool| {
            if let Some(window) = page.ctx.window() {
                window.global::<ProjectsPageData>().set_busy_button(if busy { field } else { "" }.into());
            }
        }
    };
    let shown = {
        let page = page.clone();
        move |m: Option<StatusMessage>| page.show(m)
    };
    let out = results.clone();
    let ok = inputs::run(
        busy,
        shown,
        || async move {
            *out.borrow_mut() = w2_common::mcp(api.update_tasks(&items).await)?;
            Ok(())
        },
        None,
    )
    .await;
    if !ok {
        return;
    }
    let (text, error) = batch_message(&results.borrow(), &what);
    log::info!("批次修改 {project}：{text}");
    page.selected.borrow_mut().clear();
    page.ctx.engine.after_external_change().await;
    load(page.clone(), true).await;
    page.show(Some(StatusMessage { text, error }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::TestInfo;

    fn task(key: &str, parent: Option<&str>, category: &str) -> ProjectTask {
        ProjectTask::new(key, format!("任務 {key}"), "狀態", category, parent.map(str::to_owned), None, None)
    }

    fn keys(rows: &[ListRow]) -> Vec<(&str, i32)> {
        rows.iter().map(|r| (r.task.key.as_str(), r.depth)).collect()
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 12).unwrap()
    }

    #[test]
    fn 樹狀_不列測試結果並在測試任務標數量() {
        let mut test = task("P-2", None, "IN_PROGRESS");
        test.test = TestInfo::new(TaskKind::Test, None, None, 0, None);
        let mut r1 = task("P-3", Some("P-2"), "DONE");
        r1.test = TestInfo::new(TaskKind::TestResult, None, None, 0, None);
        r1.status = "已完成".into();
        let mut r2 = task("P-4", Some("P-2"), "IN_PROGRESS");
        r2.test = TestInfo::new(TaskKind::TestResult, None, None, 0, None);
        r2.status = "修復中".into();
        let tasks = vec![task("P-1", None, "TODO"), task("P-5", Some("P-1"), "TODO"), test, r1, r2];
        let (rows, total, results) =
            build_rows(&tasks, None, &Filters::default(), None, today(), &Expansion::default());
        assert_eq!(vec![("P-1", 0), ("P-5", 1), ("P-2", 0)], keys(&rows));
        assert_eq!(3, total);
        let list = results.get("P-2").unwrap();
        assert_eq!(
            ("  🐞2 ✓1 🔧1 🧪0".to_owned(), "測試結果 全部 2、通過 1、修復中 1、測試中 0".to_owned()),
            results_text(list)
        );
        assert_eq!("顯示 3 個（共 3 個）", summary(total, rows.len()));
    }

    #[test]
    fn 只顯示沒有子任務的_平鋪並看整個專案() {
        let mut cat = task("P-9", None, "TODO");
        cat.is_category = true;
        let tasks =
            vec![task("P-1", None, "TODO"), task("P-2", Some("P-1"), "DONE"), task("P-3", Some("P-1"), "TODO"), cat];
        let f = Filters { leaf_only: true, hide_done: true, ..Filters::default() };
        let (rows, _, _) = build_rows(&tasks, None, &f, None, today(), &Expansion::default());
        assert_eq!(vec![("P-3", 0)], keys(&rows));
        let f = Filters { leaf_only: true, filter: "p-2".into(), ..Filters::default() };
        let (rows, _, _) = build_rows(&tasks, None, &f, None, today(), &Expansion::default());
        assert_eq!(vec![("P-2", 0)], keys(&rows));
    }

    #[test]
    fn 只看我的與逾期() {
        let mut a = task("P-1", None, "TODO");
        a.assignees = Some(vec!["Harry".into()]);
        a.planned_end = NaiveDate::from_ymd_opt(2026, 11, 1);
        let mut b = task("P-2", None, "TODO");
        b.planned_end = NaiveDate::from_ymd_opt(2026, 11, 30);
        b.assignees = Some(vec!["harry".into()]);
        let tasks = vec![a, b, task("P-3", None, "TODO")];
        let f = Filters { mine: true, ..Filters::default() };
        let (rows, _, _) = build_rows(&tasks, None, &f, Some("HARRY"), today(), &Expansion::default());
        assert_eq!(vec![("P-1", 0), ("P-2", 0)], keys(&rows));
        let f = Filters { overdue_only: true, ..Filters::default() };
        let (rows, _, _) = build_rows(&tasks, None, &f, None, today(), &Expansion::default());
        assert_eq!(vec![("P-1", 0)], keys(&rows));
    }

    #[test]
    fn 任務多時只展開最上層_點開的記住() {
        let mut tasks =
            vec![task("P-1", None, "TODO"), task("P-2", Some("P-1"), "TODO"), task("P-3", Some("P-2"), "TODO")];
        for i in 10..170 {
            tasks.push(task(&format!("P-{i}"), None, "TODO"));
        }
        let mut e = Expansion::default();
        let (rows, _, _) = build_rows(&tasks, None, &Filters::default(), None, today(), &e);
        assert_eq!(&[("P-1", 0), ("P-2", 1), ("P-10", 0)], &keys(&rows)[..3]);
        assert!(!rows[1].open);
        e.toggle("p-2", false);
        let (rows, _, _) = build_rows(&tasks, None, &Filters::default(), None, today(), &e);
        assert_eq!(&[("P-1", 0), ("P-2", 1), ("P-3", 2)], &keys(&rows)[..3]);
        e.toggle("P-1", true);
        let (rows, _, _) = build_rows(&tasks, None, &Filters::default(), None, today(), &e);
        assert_eq!(&[("P-1", 0), ("P-10", 0)], &keys(&rows)[..2]);
    }

    #[test]
    fn 批次結果訊息() {
        let r = |t: &str, e: Option<&str>, m: Option<&str>| BatchResult {
            task: t.into(),
            error: e.map(Into::into),
            message: m.map(Into::into),
        };
        let results =
            vec![r("P-1", None, None), r("P-2", None, Some("已送出變更申請")), r("P-3", Some("沒有權限"), None)];
        assert_eq!(
            ("2 個狀態改成「完成」（其中 1 個送出變更申請）；1 個失敗：P-3 沒有權限".to_owned(), true),
            batch_message(&results, "狀態改成「完成」")
        );
        assert_eq!(("1 個優先級改成「高」".to_owned(), false), batch_message(&results[..1], "優先級改成「高」"));
        assert_eq!("這個專案還沒有任務", summary(0, 0));
        assert_eq!("沒有符合的任務", summary(3, 0));
    }

    #[test]
    fn 網址跳脫() {
        assert_eq!("F2P", escape_data_string("F2P"));
        assert_eq!("A%20B%2F%E5%B0%88", escape_data_string("A B/專"));
    }
}
