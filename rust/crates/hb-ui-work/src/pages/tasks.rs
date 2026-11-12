//! 工作台「我的任務」頁（對應 C# `UI/Work/TasksPage.xaml(.cs)、TasksPage.*.cs`）：
//! 分頁清單、搜尋、詳細（直接改狀態、日期、工時）、子任務、聊天、新增任務。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。
//!
//! 檔案對應：清單、樹狀、看板、篩選在這裡（TasksPage.xaml.cs）；`tasks/schedule.rs`（Schedule.cs）、`calendar.rs`、
//! `gantt.rs`、`columns.rs`、`detail.rs`（xaml.cs 的詳細）、`testing.rs`、`extras.rs`、`note.rs`、`create.rs`（xaml.cs 的新增任務）、
//! `vm.rs`（檔尾的 view model）。

mod calendar;
mod columns;
mod create;
mod detail;
mod extras;
mod gantt;
mod note;
mod schedule;
mod testing;
mod vm;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate, NaiveDateTime};
use hb_api::{KeyFinder, ProjectInfo, SearchHit, TaskDetail, TaskSummary, WorkMath};
use hb_backend::config::{AppConfig, TaskColumnConfig};
use hb_core::{CiSet, HandlerId, MyTaskProject, MyTaskTree, StatusFilter, TaskOrder, TimerEngine};
use hb_ui_common::{KeyCopy, TaskMenuHooks, TaskText};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::shared::{self, icon_item, no_icon, string_model};
use crate::{PageArg, TaskMenuState, TasksPageData, TkCard, TkCheckItem, TkRow, TkTreeRow, WorkCtx};

use vm::{RowVm, TreeItem};

const PAGE_KEY: &str = "tasks";
const SEARCH_TAB: &str = "search";
/// 已完成分頁：不在我的任務那一份裡，切過來才讀（my_tasks tab=done，選期間）。
const DONE_TAB: &str = "done";
const DONE_RANGES: [(&str, &str); 4] =
    [("7", "最近 7 天"), ("30", "最近 30 天"), ("90", "最近 90 天"), ("all", "全部")];
/// 顯示方式（跟 view-index 同順序）。
const VIEWS: [&str; 5] = ["list", "tree", "kanban", "calendar", "gantt"];
/// 已完成的那份幾分鐘內不重讀。
const DONE_REFRESH: Duration = Duration::from_secs(5 * 60);

thread_local! {
    /// 這個工作台視窗的「我的任務」頁（init 時建立；視窗重建時換新的）。
    static PAGE: RefCell<Option<Rc<Page>>> = const { RefCell::new(None) };
}

fn page() -> Option<Rc<Page>> {
    PAGE.with(|p| p.borrow().clone())
}

pub fn init(ctx: &WorkCtx) {
    let page = Page::new(ctx.clone());
    page.wire();
    PAGE.with(|p| *p.borrow_mut() = Some(page));
}

/// arg：任務編號（直接打開那個任務）、"new"（新增）、"new:F2P-5"（新增 F2P-5 的子任務）、分頁名稱（overdue…）
/// 或顯示方式（list、tree、kanban、calendar、gantt）。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = page() else { return };
    page.render_list(true);
    let Some(arg) = arg.as_ref().and_then(PageArg::as_text).map(str::to_owned) else { return };
    if arg == "new" {
        page.clone().show_create(None);
    } else if arg.len() > 4
        && arg.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("new:"))
        && let Some(parent) = KeyFinder::parse(arg.get(4..))
    {
        page.clone().show_create(Some(parent));
    } else if let Some(i) = VIEWS.iter().position(|v| *v == arg) {
        page.select_view(i);
    } else if page.tab_keys().contains(&arg) {
        page.select_tab(&arg);
    } else if let Some(key) = KeyFinder::parse(Some(&arg)) {
        page.clone().open_task(key).await;
    }
}

/// 狀態篩選選單的一項對應的動作。
#[derive(Debug, Clone)]
enum FilterAction {
    None,
    Category(String),
    Status(String),
    Clear,
}

/// 頁面狀態（C# TasksPage 的欄位）。
struct State {
    tab: String,
    /// list、tree、kanban、calendar、gantt
    view: String,
    /// 只看這個專案（名稱）；None＝全部
    project: Option<String>,
    done_tasks: Vec<TaskSummary>,
    done_days: String,
    done_fetched: Option<Instant>,
    loading_done: bool,
    loading_status_order: bool,
    /// 看板計時中卡片的編號（每秒更新）
    running_cards: Vec<String>,
    list_rows: Vec<RowVm>,
    shown: Vec<TaskSummary>,
    selected_key: Option<String>,
    tree_projects: Vec<MyTaskProject>,
    tree_items: Vec<TreeItem>,
    /// 樹狀檢視裡收合的專案（p:名稱）與任務
    collapsed: CiSet,
    status_filter: StatusFilter,
    category_options: Vec<(&'static str, &'static str, usize)>,
    status_options: Vec<(String, usize)>,
    status_total: usize,
    filter_actions: Vec<FilterAction>,
    search_hits: Vec<SearchHit>,
    search_label: Option<String>,
    project_names: Vec<String>,
    project_icons_shown: bool,
    project_box_ready: bool,
    columns: Vec<TaskColumnConfig>,
    col_draft: Vec<TaskColumnConfig>,

    // 詳細
    detail: Option<TaskDetail>,
    detail_version: u64,
    loading_detail: bool,
    child_hours_for: Option<String>,
    tree_for: Option<String>,
    /// 上層任務 → 它的子任務（打開任務時順手記下，同一層移動不用多打請求）
    children_of: HashMap<String, Vec<(String, String, String)>>,
    result_filter: String,
    result_filter_for: Option<String>,
    result_bucket: String,
    moving_sibling: bool,
    note_append_for: Option<String>,
    deps_for: Option<String>,
    deps_rows: Vec<Option<(hb_api::DependencyItem, bool)>>,
    dep_quick: Vec<String>,
    attach_for: Option<String>,
    attachments: Vec<hb_api::AttachmentItem>,
    attach_web: Option<String>,
    docs_for: Option<String>,
    history_for: Option<String>,
    history_cursor: Option<String>,
    history_lines: Vec<crate::TkLine>,
    /// 測試結果的批次子狀態（〔⇢ 已更新到測試機〕用）
    test_batch: Option<hb_api::TestSubStatus>,

    // 新增任務
    creating_child_of: Option<String>,
    /// 新任務的上層，None＝最上層
    picked_parent: Option<String>,
    last_created: Option<String>,
    keep_parent_on_project_change: bool,
    tree_version: u64,
    create_projects: Option<Vec<ProjectInfo>>,
    project_tasks: Vec<hb_api::ProjectTask>,
    pick_roots: Vec<hb_core::TaskTreeNode>,
    pick_rows: Vec<vm::PickRowVm>,
    pick_toggled: HashMap<String, bool>,

    // 日曆、甘特
    schedule: schedule::ScheduleState,

    /// 確認對話框的回覆
    confirm: Option<tokio::sync::oneshot::Sender<i32>>,
}

pub(crate) struct Page {
    ctx: WorkCtx,
    st: RefCell<State>,
    tick: slint::Timer,
    me: Weak<Page>,
    changed_id: Cell<Option<HandlerId>>,
    rows_model: Rc<VecModel<TkRow>>,
    tree_model: Rc<VecModel<TkTreeRow>>,
    card_models: [Rc<VecModel<TkCard>>; 3],
}

impl Drop for Page {
    fn drop(&mut self) {
        if let Some(id) = self.changed_id.take() {
            self.ctx.engine.events().changed.unsubscribe(id);
        }
    }
}

/// 接一個 TasksPageData 的回呼到 Page 的方法（page 不在了就什麼都不做）。
macro_rules! on {
    ($d:expr, $page:expr, $setter:ident, |$p:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = $page.me.clone();
        $d.$setter(move |$($arg),*| match weak.upgrade() {
            Some($p) => $body,
            None => Default::default(),
        });
    }};
}
pub(crate) use on;

impl Page {
    fn new(ctx: WorkCtx) -> Rc<Page> {
        let config = ctx.engine.config().clone();
        let view =
            if VIEWS.contains(&config.task_view.as_str()) { config.task_view.clone() } else { "list".to_owned() };
        let state = State {
            tab: TimerEngine::ALL_TAB.to_owned(),
            view,
            project: Some(config.task_project.clone()).filter(|p| !p.is_empty()),
            done_tasks: Vec::new(),
            done_days: "30".to_owned(),
            done_fetched: None,
            loading_done: false,
            loading_status_order: false,
            running_cards: Vec::new(),
            list_rows: Vec::new(),
            shown: Vec::new(),
            selected_key: None,
            tree_projects: Vec::new(),
            tree_items: Vec::new(),
            collapsed: CiSet::new(),
            status_filter: StatusFilter::new(),
            category_options: Vec::new(),
            status_options: Vec::new(),
            status_total: 0,
            filter_actions: Vec::new(),
            search_hits: Vec::new(),
            search_label: None,
            project_names: Vec::new(),
            project_icons_shown: false,
            project_box_ready: false,
            columns: hb_core::TaskColumns::normalize(config.task_columns.iter()),
            col_draft: Vec::new(),
            detail: None,
            detail_version: 0,
            loading_detail: false,
            child_hours_for: None,
            tree_for: None,
            children_of: HashMap::new(),
            result_filter: String::new(),
            result_filter_for: None,
            result_bucket: String::new(),
            moving_sibling: false,
            note_append_for: None,
            deps_for: None,
            deps_rows: Vec::new(),
            dep_quick: Vec::new(),
            attach_for: None,
            attachments: Vec::new(),
            attach_web: None,
            docs_for: None,
            history_for: None,
            history_cursor: None,
            history_lines: Vec::new(),
            test_batch: None,
            creating_child_of: None,
            picked_parent: None,
            last_created: None,
            keep_parent_on_project_change: false,
            tree_version: 0,
            create_projects: None,
            project_tasks: Vec::new(),
            pick_roots: Vec::new(),
            pick_rows: Vec::new(),
            pick_toggled: HashMap::new(),
            schedule: schedule::ScheduleState::new(Local::now().date_naive()),
            confirm: None,
        };
        Rc::new_cyclic(|me| Page {
            ctx,
            st: RefCell::new(state),
            tick: slint::Timer::default(),
            me: me.clone(),
            changed_id: Cell::new(None),
            rows_model: Rc::new(VecModel::default()),
            tree_model: Rc::new(VecModel::default()),
            card_models: [Rc::new(VecModel::default()), Rc::new(VecModel::default()), Rc::new(VecModel::default())],
        })
    }

    fn rc(&self) -> Option<Rc<Page>> {
        self.me.upgrade()
    }

    fn engine(&self) -> &TimerEngine {
        &self.ctx.engine
    }

    /// 改設定並存檔（C# `_engine.Config.X = …; _engine.Config.Save()`）。
    fn save_config(&self, f: impl FnOnce(&mut AppConfig)) {
        f(&mut self.engine().config_mut());
        self.engine().config().save();
    }

    /// 在 TasksPageData 上做事（視窗關了就略過）。
    fn with_data<R>(&self, f: impl FnOnce(&TasksPageData) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let d = w.global::<TasksPageData>();
        Some(f(&d))
    }

    fn is_visible(&self) -> bool {
        self.ctx.is_page_visible(PAGE_KEY)
    }

    fn today() -> NaiveDate {
        Local::now().date_naive()
    }

    fn now() -> NaiveDateTime {
        Local::now().naive_local()
    }

    /// 綁定回呼、設定初始值（C# 建構子）。
    fn wire(self: &Rc<Self>) {
        let Some(w) = self.ctx.window() else { return };
        let d = w.global::<TasksPageData>();
        let config = self.engine().config().clone();

        d.set_rows(ModelRc::from(self.rows_model.clone()));
        d.set_tree_rows(ModelRc::from(self.tree_model.clone()));
        d.set_todo_cards(ModelRc::from(self.card_models[0].clone()));
        d.set_doing_cards(ModelRc::from(self.card_models[1].clone()));
        d.set_done_cards(ModelRc::from(self.card_models[2].clone()));
        d.set_done_days_labels(string_model(DONE_RANGES.iter().map(|r| r.1)));
        d.set_done_days_index(1);
        d.set_sort_labels(string_model(TaskOrder::SORTS.iter().map(|s| s.1)));
        d.set_sort_index(TaskOrder::SORTS.iter().position(|s| s.0 == config.task_sort).unwrap_or(0) as i32);
        d.set_view_index(VIEWS.iter().position(|v| *v == self.st.borrow().view).unwrap_or(0) as i32);
        d.set_leaf_only(config.task_leaf_only);
        d.set_hide_done(config.task_hide_done);
        d.set_c_hide_done(config.tree_hide_done);
        d.set_today(hb_ui_common::date_range::today_int());
        d.set_d_type_items(string_model(
            [hb_api::TaskKind::Normal, hb_api::TaskKind::Test, hb_api::TaskKind::TestResult]
                .iter()
                .map(|k| hb_api::TaskKinds::label(*k)),
        ));
        d.set_d_bucket_labels(string_model(testing::BUCKETS.iter().map(|b| b.1)));
        d.set_d_dep_templates(string_model(hb_api::HayakuApi::DEPENDENCY_TEMPLATES.iter().map(|t| t.1)));
        self.apply_columns();

        on!(d, self, on_reload, |p| p.reload());
        on!(d, self, on_new_task, |p| p.show_create(None));
        on!(d, self, on_tab_selected, |p, i| {
            if let Some(tab) = p.tab_keys().get(i as usize).cloned() {
                p.select_tab(&tab);
            }
        });
        on!(d, self, on_search_accepted, |p, text| p.search(text.to_string()));
        on!(d, self, on_search_escape, |p| p.search_escape());
        on!(d, self, on_view_selected, |p, i| p.select_view(i as usize));
        on!(d, self, on_sort_selected, |p, i| {
            if let Some(s) = TaskOrder::SORTS.get(i as usize) {
                p.save_config(|c| c.task_sort = s.0.to_owned());
                p.render_list(true);
            }
        });
        on!(d, self, on_status_filter_opening, |p| p.render_filter_menu());
        on!(d, self, on_status_filter_toggle, |p, i| p.toggle_filter(i as usize));
        on!(d, self, on_project_selected, |p, i| p.select_project(i as usize));
        on!(d, self, on_done_days_selected, |p, i| p.select_done_days(i as usize));
        on!(d, self, on_leaf_only_toggled, |p, v| {
            p.save_config(|c| c.task_leaf_only = v);
            p.render_list(true);
        });
        on!(d, self, on_hide_done_toggled, |p, v| {
            p.save_config(|c| c.task_hide_done = v);
            p.render_list(true);
        });
        on!(d, self, on_collapse_all, |p| p.collapse_all());
        on!(d, self, on_row_open, |p, key| p.open_key(&key));
        on!(d, self, on_row_timer, |p, key| p.row_timer(key.to_string()));
        on!(d, self, on_copy_key, |p, key| KeyCopy::copy(p.engine(), &key));
        on!(d, self, on_tree_toggle, |p, i| p.toggle_tree(i as usize));
        on!(d, self, on_close_dialog, |p| p.close_dialog());
        on!(d, self, on_dialog_key, |p, e| p.dialog_key(&e));
        on!(d, self, on_confirm_picked, |p, i| p.confirm_picked(i));

        self.wire_columns(&d);
        self.wire_schedule(&d);
        self.wire_detail(&d);
        self.wire_create(&d);

        // 右鍵選單：計時、改狀態、釘選、查看、複製編號、網頁開啟、刪除（C# ShowRowMenu）
        let hooks = {
            let (w1, w2, w3) = (self.me.clone(), self.me.clone(), self.me.clone());
            TaskMenuHooks {
                open: Rc::new(move |key: &str| {
                    if let Some(p) = w1.upgrade() {
                        p.open_key(key);
                    }
                }),
                delete: Rc::new(move |key: &str| {
                    if let Some(p) = w2.upgrade() {
                        let key = key.to_owned();
                        shared::spawn("刪除任務", async move { p.delete_from_menu(key).await });
                    }
                }),
                // 詳細開著就重讀，子任務才看得到
                template_applied: Some(Rc::new(move || {
                    let Some(p) = w3.upgrade() else { return };
                    let Some(key) = p.ctx.window().map(|w| w.global::<TaskMenuState>().get_key().to_string()) else {
                        return;
                    };
                    if p.st.borrow().detail.as_ref().is_some_and(|d| d.key == key) {
                        p.open_key(&key);
                    }
                })),
            }
        };
        self.ctx.set_task_menu_hooks(PAGE_KEY, hooks);

        let weak = self.me.clone();
        let id = self.engine().events().changed.subscribe(move |_| {
            if let Some(p) = weak.upgrade()
                && p.is_visible()
            {
                p.render_list(true);
            }
        });
        self.changed_id.set(Some(id));
    }

    // ───────────── 分頁、篩選 ─────────────

    /// 目前看得到的分頁代號：全部、進行中…、已完成，有搜尋時加搜尋結果。
    fn tab_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = TimerEngine::TASK_VIEW_TABS.iter().map(|t| t.0.to_owned()).collect();
        keys.push(DONE_TAB.to_owned());
        if self.st.borrow().search_label.is_some() {
            keys.push(SEARCH_TAB.to_owned());
        }
        keys
    }

    fn select_tab(&self, tab: &str) {
        self.st.borrow_mut().tab = tab.to_owned();
        self.render_list(false);
    }

    fn select_view(&self, index: usize) {
        let Some(view) = VIEWS.get(index) else { return };
        self.st.borrow_mut().view = (*view).to_owned();
        self.with_data(|d| d.set_view_index(index as i32));
        self.save_config(|c| c.task_view = (*view).to_owned());
        self.render_list(true);
    }

    fn select_project(&self, index: usize) {
        let project = if index == 0 { None } else { self.st.borrow().project_names.get(index - 1).cloned() };
        self.st.borrow_mut().project = project.clone();
        self.save_config(|c| c.task_project = project.unwrap_or_default());
        self.render_list(true);
    }

    fn select_done_days(&self, index: usize) {
        let Some(range) = DONE_RANGES.get(index) else { return };
        {
            let mut st = self.st.borrow_mut();
            st.done_days = range.0.to_owned();
            st.done_tasks.clear();
            st.done_fetched = None;
        }
        self.render_list(false);
    }

    fn reload(&self) {
        self.st.borrow_mut().schedule_done_reset(); // 看板的已完成欄也重讀
        let engine = self.engine().clone();
        shared::spawn("重新整理", async move { engine.after_external_change().await });
    }

    /// 看板模式（搜尋時除外）。
    fn kanban_on(st: &State) -> bool {
        st.view == "kanban" && st.tab != SEARCH_TAB
    }

    fn column_visible(st: &State, key: &str) -> bool {
        st.columns.iter().any(|c| c.key == key && c.visible)
    }

    fn project_info(&self, project_name: &str, key: &str) -> Option<ProjectInfo> {
        self.engine().project_of(project_name).or_else(|| self.engine().project_of(KeyFinder::project_of(key)))
    }

    // ───────────── 清單 ─────────────

    /// C# RenderList：分頁數量、清單、樹狀、看板、日曆、甘特一起重畫。
    fn render_list(&self, keep_selection: bool) {
        let Some(w) = self.ctx.window() else { return };
        let d = w.global::<TasksPageData>();
        let engine = self.engine().clone();
        let running = engine.running_keys();
        let today = Self::today();
        let leaf_only = engine.config().task_leaf_only;
        let hide_done = engine.config().task_hide_done;
        let code_only = engine.config().task_project_code_only;
        let mut load_done = false;
        let mut load_status_order: Option<Vec<String>> = None;
        let mut st = self.st.borrow_mut();
        let kanban = Self::kanban_on(&st);

        // 分頁標題與數量
        let mut labels: Vec<String> = TimerEngine::TASK_VIEW_TABS
            .iter()
            .map(|(tab, label)| {
                vm::tab_label(label, engine.task_list(tab).iter().filter(|t| !leaf_only || t.is_leaf()).count())
            })
            .collect();
        labels.push(if !st.done_tasks.is_empty() && st.done_fetched.is_some() {
            format!("已完成 {}", st.done_tasks.len())
        } else {
            "已完成".to_owned()
        });
        if let Some(label) = &st.search_label {
            labels.push(label.clone());
        }
        let mut keys: Vec<&str> = TimerEngine::TASK_VIEW_TABS.iter().map(|t| t.0).collect();
        keys.push(DONE_TAB);
        if st.search_label.is_some() {
            keys.push(SEARCH_TAB);
        }
        if !shared::same_strings(&d.get_tab_labels(), &labels) {
            d.set_tab_labels(string_model(&labels));
        }
        d.set_tab_index(keys.iter().position(|k| *k == st.tab).map_or(0, |i| i as i32));
        d.set_done_days_visible(st.tab == DONE_TAB && !kanban);
        if st.tab == DONE_TAB && !kanban && st.done_fetched.is_none_or(|t| t.elapsed() > DONE_REFRESH) {
            load_done = true;
        }

        let sort = TaskOrder::SORTS.get(d.get_sort_index().max(0) as usize).map_or("default", |s| s.0);
        let rows: Vec<RowVm>;
        let mut shown: Vec<TaskSummary> = Vec::new();
        let mut message: String;
        if st.tab == SEARCH_TAB {
            Self::render_status_filters(&mut st, &d, &[]);
            let mut hits = st.search_hits.clone();
            match sort {
                "key" => hits.sort_by(|a, b| {
                    hb_core::ci_set::cmp_ic(KeyFinder::project_of(&a.key), KeyFinder::project_of(&b.key))
                        .then_with(|| vm::key_number(&a.key).cmp(&vm::key_number(&b.key)))
                }),
                "timing" => hits.sort_by_key(|h| std::cmp::Reverse(running.contains(&h.key))),
                _ => {}
            }
            rows = hits
                .iter()
                .map(|h| {
                    let mut r = RowVm::new(&h.key, &h.title);
                    r.project_text = vm::project_text(code_only, &h.project, &h.key);
                    r.project = self.project_info(&h.project, &h.key);
                    r.timing = running.contains(&h.key);
                    r
                })
                .collect();
            message = if rows.is_empty() { "找不到符合的任務".to_owned() } else { String::new() };
        } else {
            // 看板看全部（待處理＋進行中＋已完成），其他照分頁；再套用專案、狀態、只看沒有子任務的篩選
            let lists = engine.task_lists();
            let all: Vec<TaskSummary> = if kanban {
                let mut seen = CiSet::new();
                lists
                    .values()
                    .flat_map(|l| l.iter())
                    .chain(if hide_done { [].iter() } else { st.done_tasks.iter() })
                    .filter(|t| seen.insert(t.key.clone()))
                    .cloned()
                    .collect()
            } else if st.tab == DONE_TAB {
                st.done_tasks.clone()
            } else {
                engine.task_list(&st.tab).as_ref().clone()
            };
            let everything: Vec<TaskSummary> =
                lists.values().flat_map(|l| l.iter()).chain(st.done_tasks.iter()).cloned().collect();
            self.render_project_box(&mut st, &d, &everything, !lists.is_empty());
            let project = st.project.clone();
            let tasks: Vec<TaskSummary> = all
                .iter()
                .filter(|t| project.as_ref().is_none_or(|p| &t.project == p) && (!leaf_only || t.is_leaf()))
                .cloned()
                .collect();
            Self::render_status_filters(&mut st, &d, &tasks);
            let filter = st.status_filter.clone();
            let rank = |t: &TaskSummary| engine.status_rank(&t.key, &t.status);
            shown = TaskOrder::sort(tasks.into_iter().filter(|t| filter.matches(t)), sort, &running, Some(&rank));
            if sort == "status" {
                load_status_order = Some(shown.iter().map(|t| t.key.clone()).collect());
            }
            let project_column = Self::column_visible(&st, "project");
            let list_view = st.view == "list";
            rows = shown
                .iter()
                .map(|t| RowVm {
                    key: t.key.clone(),
                    title: TaskText::title_of(t),
                    context: vm::context_text(t, list_view && project_column),
                    status: t.status.clone(),
                    planned_start: t.planned_start,
                    planned_end: t.planned_end,
                    timing: running.contains(&t.key) || t.timing,
                    done: vm::is_done_category(&t.status_category),
                    priority: t.priority.clone(),
                    project_text: vm::project_text(code_only, &t.project, &t.key),
                    project: self.project_info(&t.project, &t.key),
                    elapsed: String::new(),
                })
                .collect();
            message = if !rows.is_empty() {
                String::new()
            } else if lists.is_empty() || st.tab == DONE_TAB && st.loading_done {
                "讀取中…".to_owned()
            } else if !st.status_filter.is_empty() {
                "沒有符合狀態篩選的任務".to_owned()
            } else if leaf_only && !all.is_empty() {
                "沒有「沒有子任務」的任務（取消勾選看上層任務）".to_owned()
            } else {
                match st.tab.as_str() {
                    "overdue" => "沒有逾期的任務 🎉".to_owned(),
                    "should_start" => "沒有該開始還沒開始的任務".to_owned(),
                    "test" => "沒有指派給你的測試任務或測試結果".to_owned(),
                    DONE_TAB => {
                        let label = DONE_RANGES.iter().find(|r| r.0 == st.done_days).map_or("", |r| r.1);
                        format!("{label}沒有完成的任務")
                    }
                    _ => "這裡沒有任務".to_owned(),
                }
            };
        }

        // 清單（保留選取）
        let previous = if keep_selection {
            st.selected_key.clone().or_else(|| st.detail.as_ref().map(|x| x.key.clone()))
        } else {
            None
        };
        st.list_rows = rows;
        st.shown = shown.clone();
        Self::update_row_elapsed(&mut st, &engine);
        let elapsed_on = Self::column_visible(&st, "elapsed");
        let tk_rows: Vec<TkRow> = st.list_rows.iter().map(|r| tk_row(r, today, elapsed_on)).collect();
        self.rows_model.set_vec(tk_rows);
        let selected = previous.and_then(|k| st.list_rows.iter().position(|r| r.key.eq_ignore_ascii_case(&k)));
        st.selected_key = selected.and_then(|i| st.list_rows.get(i).map(|r| r.key.clone()));
        d.set_selected_index(selected.map_or(-1, |i| i as i32));

        // 樹狀、看板、日曆、甘特：搜尋結果沒有上下層、狀態類別與日期，照樣用清單
        let searching = st.tab == SEARCH_TAB;
        let tree = st.view == "tree" && !searching;
        let calendar = st.view == "calendar" && !searching;
        let gantt = st.view == "gantt" && !searching;
        let mode = if tree {
            1
        } else if kanban {
            2
        } else if calendar {
            3
        } else if gantt {
            4
        } else {
            0
        };
        d.set_mode(mode);
        d.set_tabs_visible(!kanban); // 看板不分分頁
        if tree {
            self.render_tree(&mut st, &d, &shown);
        } else {
            st.tree_items.clear();
            self.tree_model.set_vec(Vec::new());
        }
        if kanban {
            if let Some(m) = self.render_kanban(&mut st, &d, &shown, hide_done) {
                message = m;
            }
            if !hide_done && st.done_fetched.is_none_or(|t| t.elapsed() > DONE_REFRESH) {
                load_done = true;
            }
        } else {
            for m in &self.card_models {
                m.set_vec(Vec::new());
            }
        }
        let view = if calendar {
            Some("calendar")
        } else if gantt {
            Some("gantt")
        } else {
            None
        };
        if view.is_some() && !shown.is_empty() {
            message.clear();
        }
        d.set_list_message(message.into());
        self.update_filter_highlights(&st, &d);
        drop(st);
        self.render_schedule(view, shown);
        self.update_tick();

        if load_done && let Some(p) = self.rc() {
            shared::spawn("讀取已完成的任務", async move { p.load_done().await });
        }
        if let Some(keys) = load_status_order
            && let Some(p) = self.rc()
        {
            shared::spawn("讀取狀態順序", async move { p.load_status_order(keys).await });
        }
    }

    /// 依狀態排序要用專案設定的狀態順序：還沒讀過的專案讀完再重排一次。
    async fn load_status_order(self: Rc<Self>, keys: Vec<String>) {
        if self.st.borrow().loading_status_order {
            return;
        }
        self.st.borrow_mut().loading_status_order = true;
        let loaded = self.engine().load_projects_of(keys.iter().map(String::as_str)).await;
        self.st.borrow_mut().loading_status_order = false;
        if loaded {
            self.render_list(true);
        }
    }

    /// 已完成分頁、看板的已完成欄：my_tasks 的已完成分頁（最近完成的在最上面，一個請求），期間照「已完成」分頁選的；
    /// 5 分鐘內不重讀。
    async fn load_done(self: Rc<Self>) {
        let days = {
            let mut st = self.st.borrow_mut();
            if st.loading_done {
                return;
            }
            st.loading_done = true;
            st.done_fetched = Some(Instant::now());
            st.done_days.clone()
        };
        match self.engine().api().my_done_tasks(&days).await {
            Ok(list) => self.st.borrow_mut().done_tasks = list,
            Err(e) => log::error!("讀取已完成的任務失敗: {}", e.0),
        }
        self.st.borrow_mut().loading_done = false;
        self.render_list(true);
    }

    /// 專案下拉：「全部專案」＋指派給我的任務出現過的專案（圖示＋名稱）；選的專案不見了就回到全部。
    fn render_project_box(&self, st: &mut State, d: &TasksPageData, tasks: &[TaskSummary], lists_loaded: bool) {
        let mut names: Vec<String> = Vec::new();
        for t in tasks {
            if !t.project.is_empty() && !names.contains(&t.project) {
                names.push(t.project.clone());
            }
        }
        names.sort();
        if let Some(p) = &st.project
            && !names.contains(p)
            && lists_loaded
        {
            st.project = None;
        }
        let index = st.project.as_ref().and_then(|p| names.iter().position(|n| n == p)).map_or(0, |i| i + 1);
        let icons_ready = !self.engine().projects().is_empty();
        let same = names == st.project_names && icons_ready == st.project_icons_shown && st.project_box_ready;
        if same && d.get_project_index() == index as i32 {
            return;
        }
        if !same {
            let mut labels = vec!["全部專案".to_owned()];
            labels.extend(names.iter().cloned());
            let mut icons = vec![no_icon()];
            icons.extend(names.iter().map(|n| icon_item(self.engine().project_of(n).as_ref(), n)));
            d.set_project_labels(string_model(&labels));
            d.set_project_icons(shared::vec_model(icons));
            st.project_names = names;
            st.project_icons_shown = icons_ready;
            st.project_box_ready = true;
        }
        d.set_project_index(index as i32);
    }

    /// 狀態篩選的選項：這個分頁裡出現的狀態類別，加上各專案自己的狀態（處理中、待客戶回應…）與數量。
    /// 選的項目在新清單裡不見了（換分頁、任務改狀態）就拿掉。搜尋結果沒有狀態，不顯示。
    fn render_status_filters(st: &mut State, d: &TasksPageData, tasks: &[TaskSummary]) {
        st.category_options = TaskOrder::categories(tasks.iter());
        st.status_options = TaskOrder::statuses(tasks.iter());
        st.status_total = tasks.len();
        let categories: Vec<&str> = st.category_options.iter().map(|c| c.0).collect();
        let statuses: Vec<String> = st.status_options.iter().map(|s| s.0.clone()).collect();
        st.status_filter.keep_only(categories, statuses.iter().map(String::as_str));
        d.set_status_filter_visible(!tasks.is_empty() || !st.status_filter.is_empty());
    }

    /// 有套用條件的篩選控制項亮起來，一眼看得出清單被篩過；多選寫出選了幾個。
    fn update_filter_highlights(&self, st: &State, d: &TasksPageData) {
        d.set_status_filter_active(!st.status_filter.is_empty());
        d.set_project_active(st.project.is_some());
        d.set_status_filter_text(vm::status_filter_text(st.status_filter.count()).into());
    }

    /// 狀態篩選的選單：類別與各專案的狀態可以混著勾，勾了馬上套用、選單不關。
    fn render_filter_menu(&self) {
        let mut st = self.st.borrow_mut();
        let mut items: Vec<TkCheckItem> = Vec::new();
        let mut actions: Vec<FilterAction> = Vec::new();
        let item = |kind: i32, text: String, checked: bool, enabled: bool| TkCheckItem {
            kind,
            text: text.into(),
            checked,
            enabled,
        };
        items.push(item(0, format!("狀態類別（共 {} 個任務）", st.status_total), false, false));
        actions.push(FilterAction::None);
        for (category, label, count) in &st.category_options {
            items.push(item(1, format!("{label}　{count}"), st.status_filter.categories.contains(category), true));
            actions.push(FilterAction::Category((*category).to_owned()));
        }
        if !st.status_options.is_empty() {
            items.push(item(2, String::new(), false, false));
            actions.push(FilterAction::None);
            items.push(item(0, "各專案的狀態".to_owned(), false, false));
            actions.push(FilterAction::None);
            for (status, count) in &st.status_options {
                items.push(item(1, format!("{status}　{count}"), st.status_filter.statuses.contains(status), true));
                actions.push(FilterAction::Status(status.clone()));
            }
        }
        items.push(item(2, String::new(), false, false));
        actions.push(FilterAction::None);
        items.push(item(3, "清除篩選（全部狀態）".to_owned(), false, !st.status_filter.is_empty()));
        actions.push(FilterAction::Clear);
        st.filter_actions = actions;
        drop(st);
        self.with_data(|d| d.set_status_filter_items(shared::vec_model(items)));
    }

    fn toggle_filter(&self, index: usize) {
        {
            let mut st = self.st.borrow_mut();
            let Some(action) = st.filter_actions.get(index).cloned() else { return };
            let f = &mut st.status_filter;
            match action {
                FilterAction::Category(c) => {
                    if !f.categories.remove(&c) {
                        f.categories.insert(c);
                    }
                }
                FilterAction::Status(s) => {
                    if !f.statuses.shift_remove(&s) {
                        f.statuses.insert(s);
                    }
                }
                FilterAction::Clear => f.clear(),
                FilterAction::None => return,
            }
        }
        self.render_list(true);
        self.render_filter_menu();
    }

    // ───────────── 樹狀檢視 ─────────────

    /// 依專案分組、照上下層排成樹狀清單；收合過的節點重畫時維持收合。
    fn render_tree(&self, st: &mut State, d: &TasksPageData, shown: &[TaskSummary]) {
        st.tree_projects = MyTaskTree::build(shown.iter());
        d.set_collapse_text(vm::collapse_button_text(&st.tree_projects, &st.collapsed).into());
        self.refresh_tree_items(st);
    }

    fn refresh_tree_items(&self, st: &mut State) {
        st.tree_items = vm::flatten_tree(&st.tree_projects, &st.list_rows, &st.collapsed);
        let today = Self::today();
        let rows: Vec<TkTreeRow> = st.tree_items.iter().map(|i| tk_tree_row(i, today)).collect();
        self.tree_model.set_vec(rows);
    }

    /// 展開或收合（C# ToggleTreeNode）。
    fn toggle_tree(&self, index: usize) {
        let mut st = self.st.borrow_mut();
        let Some(item) = st.tree_items.get(index).cloned() else { return };
        match item {
            TreeItem::Project { name, .. } => {
                let k = vm::project_collapse_key(&name);
                if !st.collapsed.remove(&k) {
                    st.collapsed.insert(k);
                }
            }
            TreeItem::Task { key, children, .. } if children > 0 => {
                if !st.collapsed.remove(&key) {
                    st.collapsed.insert(key);
                }
            }
            _ => return,
        }
        self.refresh_tree_items(&mut st);
        let text = vm::collapse_button_text(&st.tree_projects, &st.collapsed);
        drop(st);
        self.with_data(|d| d.set_collapse_text(text.into()));
    }

    /// 全部收合（有展開的就全部收起來）或全部展開；收合狀態記著，別人改了任務重畫也不會又整個展開。
    fn collapse_all(&self) {
        {
            let mut st = self.st.borrow_mut();
            let projects = std::mem::take(&mut st.tree_projects);
            vm::collapse_all(&projects, &mut st.collapsed);
            st.tree_projects = projects;
        }
        self.render_list(true);
    }

    // ───────────── 看板 ─────────────

    /// 回傳要蓋過清單訊息的文字（選了專案但沒有任務）。
    fn render_kanban(
        &self,
        st: &mut State,
        d: &TasksPageData,
        shown: &[TaskSummary],
        hide_done: bool,
    ) -> Option<String> {
        let today = Self::today();
        let mut columns: [Vec<TkCard>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        st.running_cards.clear();
        let entries = self.engine().entries();
        let (day, now) = (today.and_hms_opt(0, 0, 0).unwrap_or_default(), Self::now());
        for t in shown {
            let Some(row) = st.list_rows.iter().find(|r| r.key.eq_ignore_ascii_case(&t.key)) else { continue };
            let running = row.timing;
            let elapsed = if running {
                WorkMath::clock(WorkMath::task_minutes(entries.iter(), &t.key, day, now))
            } else {
                String::new()
            };
            if running {
                st.running_cards.push(t.key.clone());
            }
            let column = vm::kanban_column(&t.status_category);
            columns[column].push(TkCard {
                key: t.key.as_str().into(),
                project: t.project.as_str().into(),
                path: t.path.as_str().into(),
                title: row.title.as_str().into(),
                status: t.status.as_str().into(),
                category: column as i32,
                due_text: row.due_text(today).into(),
                due_tone: row.due_tone(today),
                running,
                elapsed: elapsed.into(),
                show_timer: !(row.done && !running), // 已完成的不用計時按鈕
                tip: vm::card_tip(t).into(),
            });
        }
        d.set_done_column_visible(!hide_done);
        d.set_todo_count(columns[0].len().to_string().into());
        d.set_doing_count(columns[1].len().to_string().into());
        d.set_done_count(if st.loading_done && st.done_tasks.is_empty() {
            "…".into()
        } else {
            columns[2].len().to_string().into()
        });
        for (model, cards) in self.card_models.iter().zip(columns) {
            model.set_vec(cards);
        }
        if shown.is_empty() {
            return st.project.as_ref().map(|p| format!("「{p}」沒有指派給你的任務"));
        }
        None
    }

    /// 看板的計時中卡片、清單的「今天計時」欄：有計時中的才每秒更新。
    fn update_tick(&self) {
        let ticking = {
            let st = self.st.borrow();
            let mode = self.with_data(|d| d.get_mode()).unwrap_or(0);
            let list_ticking =
                mode == 0 && Self::column_visible(&st, "elapsed") && st.list_rows.iter().any(|r| r.timing);
            self.is_visible() && (Self::kanban_on(&st) && !st.running_cards.is_empty() || list_ticking)
        };
        if ticking {
            if !self.tick.running() {
                let weak = self.me.clone();
                self.tick.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
                    if let Some(p) = weak.upgrade() {
                        p.update_elapsed();
                    }
                });
            }
        } else {
            self.tick.stop();
        }
    }

    fn update_elapsed(&self) {
        if !self.is_visible() {
            self.tick.stop();
            return;
        }
        let engine = self.engine().clone();
        let mut st = self.st.borrow_mut();
        let entries = engine.entries();
        let now = Self::now();
        let day = now.date().and_hms_opt(0, 0, 0).unwrap_or_default();
        for key in &st.running_cards {
            let elapsed: SharedString = WorkMath::clock(WorkMath::task_minutes(entries.iter(), key, day, now)).into();
            for model in &self.card_models {
                for i in 0..model.row_count() {
                    if let Some(mut c) = model.row_data(i)
                        && c.key.eq_ignore_ascii_case(key)
                        && c.elapsed != elapsed
                    {
                        c.elapsed = elapsed.clone();
                        model.set_row_data(i, c);
                    }
                }
            }
        }
        if Self::update_row_elapsed(&mut st, &engine) {
            for (i, r) in st.list_rows.iter().enumerate() {
                if let Some(mut row) = self.rows_model.row_data(i)
                    && row.elapsed != r.elapsed.as_str()
                {
                    row.elapsed = r.elapsed.as_str().into();
                    self.rows_model.set_row_data(i, row);
                }
            }
        }
    }

    /// 清單的「今天計時」；欄位沒顯示時不算（回傳 false）。
    fn update_row_elapsed(st: &mut State, engine: &TimerEngine) -> bool {
        if !Self::column_visible(st, "elapsed") {
            return false;
        }
        let entries = engine.entries();
        let now = Self::now();
        let day = now.date().and_hms_opt(0, 0, 0).unwrap_or_default();
        for r in &mut st.list_rows {
            let minutes = WorkMath::task_minutes(entries.iter(), &r.key, day, now);
            r.elapsed = if minutes > 0.0 { WorkMath::clock(minutes) } else { String::new() };
        }
        true
    }

    // ───────────── 搜尋、列上的動作 ─────────────

    fn search(&self, text: String) {
        let q = text.trim().to_owned();
        if q.is_empty() {
            return;
        }
        if let Some(key) = KeyFinder::parse(Some(&q))
            && key.eq_ignore_ascii_case(&q)
        {
            self.open_key(&key);
            return;
        }
        self.with_data(|d| d.set_list_message("搜尋中…".into()));
        let Some(p) = self.rc() else { return };
        shared::spawn("搜尋任務", async move {
            let hits = match p.engine().api().search_tasks(&q).await {
                Ok(h) => h,
                Err(e) => {
                    log::error!("搜尋任務失敗: {}", e.0);
                    Vec::new()
                }
            };
            {
                let mut st = p.st.borrow_mut();
                st.search_label = Some(vm::search_tab_label(&q, hits.len()));
                st.search_hits = hits;
                st.tab = SEARCH_TAB.to_owned();
            }
            p.render_list(false);
        });
    }

    fn search_escape(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.tab = TimerEngine::ALL_TAB.to_owned();
            st.search_label = None;
        }
        self.with_data(|d| d.set_search_text(SharedString::new()));
        self.render_list(false);
    }

    fn open_key(&self, key: &str) {
        if let Some(p) = self.rc() {
            self.st.borrow_mut().selected_key = Some(key.to_owned());
            let key = key.to_owned();
            shared::spawn("開啟任務", async move { p.open_task(key).await });
        }
    }

    fn row_timer(&self, key: String) {
        let Some(p) = self.rc() else { return };
        let running = self.engine().running_keys().contains(&key);
        shared::spawn("計時", async move {
            if running {
                p.engine().pause_task(&key).await;
            } else {
                p.engine().start_task(&key).await;
            }
            if p.st.borrow().detail.as_ref().is_some_and(|d| d.key == key) {
                p.render_timer_button();
            }
        });
    }

    /// 右鍵選單的刪除：問過再刪；詳細開著那一個就關掉。
    async fn delete_from_menu(self: Rc<Self>, key: String) {
        let (title, children) = {
            let st = self.st.borrow();
            let summary = st.shown.iter().find(|t| t.key.eq_ignore_ascii_case(&key));
            let title = st
                .list_rows
                .iter()
                .find(|r| r.key.eq_ignore_ascii_case(&key))
                .map(|r| r.title.clone())
                .or_else(|| summary.map(TaskText::title_of))
                .unwrap_or_default();
            // 樹狀、日曆、甘特知道子任務數；清單、看板不知道（C# 傳 null）
            let children = if matches!(st.view.as_str(), "tree" | "calendar" | "gantt") {
                summary.and_then(|t| t.child_count).or_else(|| {
                    st.tree_items.iter().find_map(|i| match i {
                        TreeItem::Task { key: k, children, .. } if k.eq_ignore_ascii_case(&key) => {
                            Some(*children as i32)
                        }
                        _ => None,
                    })
                })
            } else {
                None
            };
            (title, children)
        };
        let owner = self.ctx.window();
        let ok = hb_ui_common::delete_task::confirm(
            self.engine(),
            &key,
            &title,
            children,
            owner.as_ref().map(|w| w.window()),
        )
        .await;
        if !ok {
            return;
        }
        if self.st.borrow().detail.as_ref().is_some_and(|d| d.key.eq_ignore_ascii_case(&key)) {
            self.close_dialog();
        }
        self.render_list(false);
    }

    // ───────────── 彈窗、確認 ─────────────

    /// 關掉彈窗：正在讀取的任務作廢，正在編輯的欄位先存（C# Keyboard.ClearFocus 會觸發 LostFocus 送出）。
    fn close_dialog(&self) {
        let Some((dialog, title, estimate)) = self.with_data(|d| (d.get_dialog(), d.get_d_title(), d.get_d_estimate()))
        else {
            return;
        };
        if dialog == 0 {
            return;
        }
        if dialog == 1 {
            self.commit_title(title.to_string());
            self.commit_estimate(estimate.to_string());
        }
        self.st.borrow_mut().detail_version += 1;
        self.with_data(|d| d.set_dialog(0));
    }

    /// 頁面裡的確認對話框（C# MessageBox.Show）：回傳按了第幾個按鈕（Esc、點外面＝最後一個）。
    async fn confirm(&self, title: &str, text: &str, buttons: &[&str]) -> i32 {
        let (tx, rx) = tokio::sync::oneshot::channel();
        if let Some(old) = self.st.borrow_mut().confirm.replace(tx) {
            let _ = old.send(-1);
        }
        self.with_data(|d| {
            d.set_confirm_title(title.into());
            d.set_confirm_text(text.into());
            d.set_confirm_buttons(string_model(buttons));
            d.set_confirm_visible(true);
        });
        rx.await.unwrap_or(-1)
    }

    fn confirm_picked(&self, index: i32) {
        self.with_data(|d| d.set_confirm_visible(false));
        if let Some(tx) = self.st.borrow_mut().confirm.take() {
            let _ = tx.send(index);
        }
    }
}

impl State {
    fn schedule_done_reset(&mut self) {
        self.done_fetched = None;
    }
}

fn tk_row(r: &RowVm, today: NaiveDate, elapsed_on: bool) -> TkRow {
    let icon = r.project.as_ref().map(|p| hb_ui_common::ProjectIcons::icon(Some(p), &r.project_text));
    let icon = icon.unwrap_or_else(|| hb_ui_common::ProjectIcons::icon(None, &r.project_text));
    TkRow {
        key: r.key.as_str().into(),
        title: r.title.as_str().into(),
        context: r.context.as_str().into(),
        status: r.status.as_str().into(),
        priority_text: r.priority_text().into(),
        start_text: r.start_text(today).into(),
        start_hint: r.planned_start.is_none(),
        due_text: r.due_text(today).into(),
        due_tone: r.due_tone(today),
        project_text: r.project_text.as_str().into(),
        glyph: icon.glyph.into(),
        tint: icon.color.to_slint(),
        has_project: !r.project_text.is_empty(),
        timing: r.timing,
        done: r.done,
        elapsed: if elapsed_on { r.elapsed.as_str().into() } else { SharedString::new() },
    }
}

fn tk_tree_row(item: &TreeItem, today: NaiveDate) -> TkTreeRow {
    match item {
        TreeItem::Project { name, count, expanded } => TkTreeRow {
            is_project: true,
            name: name.as_str().into(),
            count_text: format!("  {count}").into(),
            expanded: *expanded,
            has_children: true,
            opacity: 1.0,
            ..Default::default()
        },
        TreeItem::Task { key, title, row, depth, children, expanded } => TkTreeRow {
            is_project: false,
            key: key.as_str().into(),
            title: title.as_str().into(),
            depth: *depth,
            expanded: *expanded,
            has_children: *children > 0,
            sub_text: item.sub_text().into(),
            tip: item.tip().into(),
            has_row: row.is_some(),
            opacity: item.text_opacity(),
            row: row.as_ref().map(|r| tk_row(r, today, false)).unwrap_or_default(),
            ..Default::default()
        },
    }
}
