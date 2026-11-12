//! 對應 C# `UI/QuickCreateWindow.xaml(.cs)`：偵測到沒有任務的新工作時，從提示按「建立任務」開的小視窗。
//! 專案自動選好，使用者選專案樹上的位置、狀態、預計起訖、預計工時（必填，開視窗時先請 Claude 估）。
//! 建好後（預設）指定給那個 Claude session，之後就自動為它計時。
//!
//! 預先建立（C# `Prewarm`）：C# 先在螢幕外把視窗顯示、畫好；Slint 的 `show()` 一定會搶走焦點（使用者正在別的程式打字時會被打斷），
//! 所以這裡的 [`QuickCreate::prewarm`] 只建好元件、讀好專案與任務樹，[`QuickCreate::present`] 時才顯示。

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Instant;

use futures_util::FutureExt;
use futures_util::future::{AbortHandle, Abortable, LocalBoxFuture, Shared};
use hb_api::{ProjectInfo, ProjectTask};
use hb_claude::{ClaudeEstimator, ClaudeMonitor};
use hb_core::{NewWorkSuggestion, TaskTree, TaskTreeNode, TimerEngine};
use serde_json::{Map, Value, json};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::dialog;
use crate::ui::{QuickCreateWindow, TreeRow};
use crate::{date_range, inputs, window_util};

/// 預計工時的小時數（可加「小時」「h」）；空白或不是正數回 None。
pub fn parse_estimate(text: &str) -> Option<f64> {
    let t = text.trim().trim_end_matches(['h', 'H']).replace("小時", "");
    t.trim().parse::<f64>().ok().filter(|h| *h > 0.0)
}

/// 小時數的顯示（C# `ToString("0.##")`）。
pub fn hours_text(h: f64) -> String {
    let s = format!("{:.2}", (h * 100.0).round() / 100.0);
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// 專案樹的一個節點（C# TreeNodeVm）。
#[derive(Debug, Clone)]
struct Node {
    task: ProjectTask,
    depth: i32,
    expanded: bool,
    children: Vec<Node>,
}

impl Node {
    /// expandAll：全部展開；否則只展開第一層與選中任務的上層。
    fn from(n: &TaskTreeNode, expand_all: bool, picked_ancestors: &HashSet<String>, depth: i32) -> Self {
        Node {
            task: n.task.clone(),
            depth,
            expanded: expand_all || depth == 0 || picked_ancestors.contains(&n.task.key.to_uppercase()),
            children: n.children.iter().map(|c| Node::from(c, expand_all, picked_ancestors, depth + 1)).collect(),
        }
    }
}

/// 樹攤平成看得到的列；回傳每列對應的節點路徑（索引）。
fn flatten(nodes: &[Node], path: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, n) in nodes.iter().enumerate() {
        path.push(i);
        out.push(path.clone());
        if n.expanded {
            flatten(&n.children, path, out);
        }
        path.pop();
    }
}

fn node_at<'a>(nodes: &'a mut [Node], path: &[usize]) -> Option<&'a mut Node> {
    let (first, rest) = path.split_first()?;
    let n = nodes.get_mut(*first)?;
    if rest.is_empty() { Some(n) } else { node_at(&mut n.children, rest) }
}

fn node_ref<'a>(nodes: &'a [Node], path: &[usize]) -> Option<&'a Node> {
    let (first, rest) = path.split_first()?;
    let n = nodes.get(*first)?;
    if rest.is_empty() { Some(n) } else { node_ref(&n.children, rest) }
}

fn eq_ic(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.to_uppercase() == b.to_uppercase()
}

/// 某任務的所有上層（含自己），由近到遠。
fn ancestors_of(tasks: &[ProjectTask], key: &str) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    let mut k = Some(key.to_owned());
    while let Some(cur) = k {
        if list.iter().any(|x| eq_ic(x, &cur)) {
            break;
        }
        list.push(cur.clone());
        k = tasks.iter().find(|t| eq_ic(&t.key, &cur)).and_then(|t| t.parent.clone());
        if !tasks.iter().any(|t| eq_ic(&t.key, &cur)) {
            break;
        }
    }
    list
}

/// 「建立在」的文字。
pub fn parent_text(tasks: &[ProjectTask], parent: Option<&str>) -> String {
    let Some(parent) = parent else { return "最上層（不屬於任何任務）".to_owned() };
    ancestors_of(tasks, parent)
        .iter()
        .rev()
        .map(|k| tasks.iter().find(|t| eq_ic(&t.key, k)).map_or(k.clone(), |t| format!("{} {}", t.key, t.title)))
        .collect::<Vec<_>>()
        .join(" › ")
}

/// 視窗關掉時的結果（C# `CreatedKey`、`CreatedTitle`、`Assigned`）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuickCreateResult {
    /// 建好的任務編號；沒建立就關掉是 None。
    pub created_key: Option<String>,
    pub created_title: String,
    /// 有沒有指定給 session（App 用來決定提示文字）。
    pub assigned: bool,
}

#[derive(Default)]
struct State {
    projects: Vec<ProjectInfo>,
    tasks: Vec<ProjectTask>,
    nodes: Vec<Node>,
    rows: Vec<Vec<usize>>,
    parent: Option<String>,
    keep_parent: bool,
    version: u64,
    estimate_abort: Option<AbortHandle>,
    estimate_seq: u64,
    estimate_edited: bool,
    prewarmed: bool,
    presented: bool,
    closed: bool,
    tree_render_ms: u128,
    result: QuickCreateResult,
}

type ClosedHandler = Box<dyn Fn(&QuickCreateResult)>;

struct Inner {
    engine: TimerEngine,
    suggestion: NewWorkSuggestion,
    ui: QuickCreateWindow,
    id: Cell<u64>,
    state: RefCell<State>,
    opening: Cell<Instant>,
    loading: RefCell<Option<Shared<LocalBoxFuture<'static, ()>>>>,
    closed_handlers: RefCell<Vec<ClosedHandler>>,
}

/// 建立任務小視窗。同一個 session 只開一個：用 [`QuickCreate::find`] 找。
#[derive(Clone)]
pub struct QuickCreate(Rc<Inner>);

thread_local! {
    static OPEN: RefCell<Vec<(String, Weak<Inner>)>> = const { RefCell::new(Vec::new()) };
}

impl QuickCreate {
    /// 這個 session 已經有（預先建好或開著的）視窗。
    pub fn find(session_id: &str) -> Option<QuickCreate> {
        OPEN.with(|o| {
            let mut list = o.borrow_mut();
            list.retain(|(_, w)| w.strong_count() > 0);
            list.iter().find(|(s, _)| s == session_id).and_then(|(_, w)| w.upgrade()).map(QuickCreate)
        })
    }

    /// 建好視窗（還不顯示）並開始讀專案；提示跳出時已經先讀好專案與任務樹的話，馬上就會填好。
    pub fn new(engine: &TimerEngine, suggestion: &NewWorkSuggestion) -> Option<QuickCreate> {
        let ui = dialog::create("建立任務", QuickCreateWindow::new)?;
        ui.set_heading(format!("建立任務 · {}", suggestion.unit_name).into());
        ui.set_hide_done(engine.config().tree_hide_done);
        ui.set_sub_text(
            match &suggestion.title {
                Some(t) => format!("Claude 正在改程式：{t}"),
                None => "Claude 正在改程式，但對應不到任務".to_owned(),
            }
            .into(),
        );
        ui.set_sub_tip(suggestion.unit_path.as_str().into());
        ui.set_new_title(suggestion.title.as_deref().unwrap_or("").trim_end_matches('…').into());
        let today = date_range::today_int();
        ui.set_today(today);
        ui.set_plan_start(today);
        ui.set_plan_end(today);
        let me = QuickCreate(Rc::new(Inner {
            engine: engine.clone(),
            suggestion: suggestion.clone(),
            ui,
            id: Cell::new(0),
            state: RefCell::default(),
            opening: Cell::new(Instant::now()),
            loading: RefCell::new(None),
            closed_handlers: RefCell::default(),
        }));
        me.update_parent_text();
        me.wire();
        let id = window_util::keep(&me.0.ui, None, me.0.clone());
        me.0.id.set(id);
        OPEN.with(|o| o.borrow_mut().push((suggestion.session_id.clone(), Rc::downgrade(&me.0))));
        let theme_weak = me.0.ui.as_weak();
        crate::theme::track(move |dark, reduce| match theme_weak.upgrade() {
            Some(ui) => {
                crate::apply_theme(&ui, dark, reduce);
                window_util::set_dark_title_bar(ui.window(), dark);
                true
            }
            None => false,
        });
        // 讀專案（Shared：Present 時估工時要等它讀完）
        let weak = Rc::downgrade(&me.0);
        let load: Shared<LocalBoxFuture<'static, ()>> = async move {
            if let Some(inner) = weak.upgrade() {
                QuickCreate(inner).load_projects().await;
            }
        }
        .boxed_local()
        .shared();
        *me.0.loading.borrow_mut() = Some(load.clone());
        dialog::spawn(load);
        Some(me)
    }

    pub fn session_id(&self) -> &str {
        &self.0.suggestion.session_id
    }

    /// 給使用者看到了沒（預先建好、還沒顯示的不算）。
    pub fn presented(&self) -> bool {
        self.0.state.borrow().presented
    }

    pub fn result(&self) -> QuickCreateResult {
        self.0.state.borrow().result.clone()
    }

    /// 視窗關掉時（建好或取消）呼叫。
    pub fn on_closed(&self, f: impl Fn(&QuickCreateResult) + 'static) {
        self.0.closed_handlers.borrow_mut().push(Box::new(f));
    }

    /// 提示跳出、資料讀好之後趁空檔先建好（見模組說明：只建元件、讀資料，不顯示）。
    pub fn prewarm(&self) {
        let watch = Instant::now();
        self.0.state.borrow_mut().prewarmed = true;
        log::info!("建立任務視窗預先建好（{} ms）", watch.elapsed().as_millis());
    }

    /// 顯示在工作列工時的正上方（anchor：實體像素）；第一次顯示時才請 Claude 估預計工時（預先建好、但沒按的不會去問）。
    pub fn present(&self, anchor: hb_win::Rect, align_left: bool) {
        let inner = &self.0;
        inner.opening.set(Instant::now());
        let ui = &inner.ui;
        window_util::place_near(ui.window(), anchor, align_left);
        if let Err(e) = ui.show() {
            log::error!("顯示建立任務視窗失敗: {e}");
            return;
        }
        window_util::set_tool_window(ui.window());
        window_util::set_dark_title_bar(ui.window(), crate::theme::current().0);
        window_util::place_near(ui.window(), anchor, align_left);
        ui.window().request_redraw();
        window_util::activate(ui.window());
        ui.invoke_focus_title();
        if inner.state.borrow().presented {
            return;
        }
        inner.state.borrow_mut().presented = true;
        // 排在繪製之後：量到的是按下去到畫面出來的時間
        let weak = Rc::downgrade(inner);
        slint::Timer::single_shot(std::time::Duration::ZERO, move || {
            let Some(inner) = weak.upgrade() else { return };
            let s = inner.state.borrow();
            let tree = if s.rows.is_empty() && s.tasks.is_empty() {
                "專案樹讀取中）".to_owned()
            } else {
                format!("專案樹 {} 筆已讀好、建樹 {} ms）", s.tasks.len(), s.tree_render_ms)
            };
            log::info!(
                "建立任務視窗 {} ms 顯示（{}，{tree}",
                inner.opening.get().elapsed().as_millis(),
                if s.prewarmed { "已預先建好" } else { "現建" }
            );
        });
        // 預先建好後過了一陣子才按：專案樹在背景重讀，有變才重畫
        if let Some(project) = self.selected_project()
            && inner.engine.cached_project_tree(&project.key).is_some_and(|(_, stale)| stale)
        {
            let version = inner.state.borrow().version;
            let me = self.clone();
            dialog::spawn(async move { me.refresh_tree(&project.key, version).await });
        }
        let me = self.clone();
        dialog::spawn(async move {
            let loading = me.0.loading.borrow().clone();
            if let Some(l) = loading {
                l.await; // 專案名稱也給 Claude 參考
            }
            me.estimate(false).await;
        });
    }

    /// 拉到前面（已經顯示時）。
    pub fn activate(&self) {
        let _ = self.0.ui.show();
        window_util::activate(self.0.ui.window());
    }

    /// 關掉（沒顯示過的預先建立視窗逾時收掉也用這個）。
    pub fn close(&self) {
        let inner = &self.0;
        {
            let mut s = inner.state.borrow_mut();
            if s.closed {
                return;
            }
            s.closed = true;
            if let Some(a) = s.estimate_abort.take() {
                a.abort();
            }
        }
        let _ = inner.ui.hide();
        let result = self.result();
        for f in inner.closed_handlers.borrow().iter() {
            f(&result);
        }
        window_util::release(inner.id.get());
    }

    fn selected_project(&self) -> Option<ProjectInfo> {
        let i = usize::try_from(self.0.ui.get_project()).ok()?;
        self.0.state.borrow().projects.get(i).cloned()
    }

    fn wire(&self) {
        let ui = &self.0.ui;
        let weak = Rc::downgrade(&self.0);
        let with = move |f: &dyn Fn(QuickCreate)| {
            if let Some(inner) = weak.upgrade() {
                f(QuickCreate(inner));
            }
        };
        {
            let with = with.clone();
            ui.window().on_close_requested(move || {
                with(&|me| me.close());
                slint::CloseRequestResponse::HideWindow
            });
        }
        {
            let with = with.clone();
            ui.on_cancel(move || with(&|me| me.close()));
        }
        {
            let with = with.clone();
            ui.on_project_selected(move |_| {
                with(&|me| {
                    let Some(project) = me.selected_project() else { return };
                    {
                        let mut s = me.0.state.borrow_mut();
                        if !s.keep_parent {
                            s.parent = None; // 換專案時上層歸零
                        }
                        s.keep_parent = false;
                    }
                    dialog::spawn(async move { me.load_tree(&project.key).await });
                })
            });
        }
        {
            let with = with.clone();
            ui.on_filter_edited(move |_| with(&|me| me.render_tree(false)));
        }
        {
            let with = with.clone();
            ui.on_hide_done_toggled(move |on| {
                with(&|me| {
                    {
                        let mut config = me.0.engine.config_mut();
                        config.tree_hide_done = on;
                        config.save();
                    }
                    me.render_tree(false);
                })
            });
        }
        {
            let with = with.clone();
            ui.on_pick_root(move || with(&|me| me.pick_parent(None)));
        }
        {
            let with = with.clone();
            ui.on_pick_row(move |i| {
                with(&|me| {
                    let key = {
                        let s = me.0.state.borrow();
                        usize::try_from(i)
                            .ok()
                            .and_then(|i| s.rows.get(i))
                            .and_then(|p| node_ref(&s.nodes, p))
                            .map(|n| n.task.key.clone())
                    };
                    if let Some(key) = key {
                        me.pick_parent(Some(key));
                    }
                })
            });
        }
        {
            let with = with.clone();
            ui.on_toggle_row(move |i| {
                with(&|me| {
                    {
                        let mut s = me.0.state.borrow_mut();
                        let Some(path) = usize::try_from(i).ok().and_then(|i| s.rows.get(i).cloned()) else { return };
                        if let Some(n) = node_at(&mut s.nodes, &path) {
                            n.expanded = !n.expanded;
                        }
                    }
                    me.show_rows();
                })
            });
        }
        {
            let with = with.clone();
            ui.on_estimate_edited(move |_| with(&|me| me.0.state.borrow_mut().estimate_edited = true));
        }
        {
            let with = with.clone();
            ui.on_ask_claude(move || {
                with(&|me| dialog::spawn(async move { me.estimate(true).await }));
            });
        }
        ui.on_create(move || with(&|me| dialog::spawn(async move { me.create().await })));
    }

    async fn load_projects(&self) {
        let inner = &self.0;
        match inner.engine.creatable_projects().await {
            Ok(projects) => {
                let names: Vec<SharedString> = projects.iter().map(|p| p.name.as_str().into()).collect();
                inner.ui.set_projects(ModelRc::new(VecModel::from(names)));
                let target = inner.engine.guess_project(&projects, &inner.suggestion);
                inner.state.borrow_mut().projects = projects.clone();
                let Some(target) = target else {
                    inner.ui.set_tree_message("沒有可以建立任務的專案".into());
                    return;
                };
                // 上次在這個資料夾選的上層，只在同一個專案時帶入
                {
                    let mut s = inner.state.borrow_mut();
                    s.parent = if inner.suggestion.project.as_deref().is_some_and(|p| eq_ic(p, &target.key)) {
                        inner.suggestion.parent.clone()
                    } else {
                        None
                    };
                    s.keep_parent = true;
                }
                let index = projects.iter().position(|p| p.key == target.key).map_or(-1, |i| i as i32);
                inner.ui.set_project(index);
                // C# 改 SelectedItem 會觸發 SelectionChanged；Slint 從程式設定不會，這裡自己呼叫
                inner.ui.invoke_project_selected(index);
            }
            Err(e) => {
                log::error!("讀取專案清單失敗: {}", e.message());
                inner.ui.set_error(format!("讀不到專案清單：{}", e.message()).into());
            }
        }
    }

    /// 讀專案的全部任務與狀態清單，畫出專案樹。
    async fn load_tree(&self, project_key: &str) {
        let inner = &self.0;
        let version = {
            let mut s = inner.state.borrow_mut();
            s.version += 1;
            s.version
        };
        // 預先讀好的直接用（不閃「讀取中…」）
        let cached = inner.engine.cached_project_tree(project_key);
        if cached.is_none() {
            inner.ui.set_tree_message("讀取中…".into());
            inner.ui.set_rows(ModelRc::default());
        }
        let load = inner.engine.project_tree(project_key, false);
        let result: Result<(Vec<ProjectTask>, std::rc::Rc<hb_api::ProjectDetail>), String> = async {
            let tasks = match &cached {
                Some((tasks, _)) => tasks.clone(),
                None => load.await.map_err(|e| e.0)?,
            };
            let project = inner.engine.get_project(project_key).await.map_err(|e| e.message().to_owned())?;
            Ok((tasks, project))
        }
        .await;
        if version != inner.state.borrow().version {
            return;
        }
        match result {
            Ok((tasks, project)) => {
                let names: Vec<SharedString> = project.statuses.iter().map(|s| s.name.as_str().into()).collect();
                inner.ui.set_statuses(ModelRc::new(VecModel::from(names)));
                // Claude 已經在改程式了，預設「進行中」類的狀態
                let pick = project
                    .statuses
                    .iter()
                    .position(|s| s.category == "IN_PROGRESS")
                    .or_else(|| project.statuses.iter().position(|s| s.is_default))
                    .or(if project.statuses.is_empty() { None } else { Some(0) });
                inner.ui.set_status(pick.map_or(-1, |i| i as i32));
                {
                    let mut s = inner.state.borrow_mut();
                    if s.parent.as_deref().is_some_and(|p| !tasks.iter().any(|t| eq_ic(&t.key, p))) {
                        s.parent = None;
                    }
                    s.tasks = tasks;
                }
                let render = Instant::now();
                self.render_tree(true);
                let ms = render.elapsed().as_millis();
                inner.state.borrow_mut().tree_render_ms = ms;
                if cached.is_none() {
                    log::info!(
                        "建立任務視窗：專案樹 {} 筆，開啟後 {} ms 顯示（建樹 {ms} ms）",
                        inner.state.borrow().tasks.len(),
                        inner.opening.get().elapsed().as_millis()
                    );
                }
                if cached.is_some_and(|(_, stale)| stale) {
                    self.refresh_tree(project_key, version).await;
                }
            }
            Err(message) => {
                log::error!("讀取 {project_key} 的任務樹失敗: {message}");
                inner.state.borrow_mut().tasks.clear();
                inner.ui.set_tree_message(format!("讀不到專案的任務：{message}").into());
            }
        }
        self.update_parent_text();
    }

    /// 先顯示的是有點舊的專案樹：在背景重讀，有變才重畫（沒變就不動，畫面不會跳）。
    async fn refresh_tree(&self, project_key: &str, version: u64) {
        let inner = &self.0;
        let fresh = match inner.engine.project_tree(project_key, true).await {
            Ok(t) => t,
            Err(e) => {
                log::error!("重讀 {project_key} 的任務樹失敗（先用剛才讀到的）: {}", e.0);
                return;
            }
        };
        let row = |t: &ProjectTask| (t.key.clone(), t.title.clone(), t.status.clone(), t.parent.clone());
        {
            let mut s = inner.state.borrow_mut();
            if version != s.version || fresh.iter().map(row).eq(s.tasks.iter().map(row)) {
                return;
            }
            if s.parent.as_deref().is_some_and(|p| !fresh.iter().any(|t| eq_ic(&t.key, p))) {
                s.parent = None;
            }
            s.tasks = fresh;
        }
        self.render_tree(false);
        self.update_parent_text();
    }

    fn render_tree(&self, scroll_to_picked: bool) {
        let inner = &self.0;
        let filter = inner.ui.get_filter().trim().to_owned();
        let hide_done = inner.ui.get_hide_done();
        {
            let mut s = inner.state.borrow_mut();
            let roots = TaskTree::build(s.tasks.iter().cloned(), !hide_done, Some(&filter));
            let expand_all = !filter.is_empty() || s.tasks.len() <= 150;
            let picked: HashSet<String> = match &s.parent {
                Some(p) => ancestors_of(&s.tasks, p).into_iter().map(|k| k.to_uppercase()).collect(),
                None => HashSet::new(),
            };
            s.nodes = roots.iter().map(|r| Node::from(r, expand_all, &picked, 0)).collect();
            let message = if s.tasks.is_empty() {
                "這個專案還沒有任務，新增的會在最上層"
            } else if roots.is_empty() {
                "沒有符合的任務"
            } else {
                ""
            };
            inner.ui.set_tree_message(message.into());
        }
        self.show_rows();
        if scroll_to_picked {
            let index = {
                let s = inner.state.borrow();
                s.parent.as_ref().and_then(|p| {
                    s.rows.iter().position(|path| node_ref(&s.nodes, path).is_some_and(|n| eq_ic(&n.task.key, p)))
                })
            };
            if let Some(i) = index {
                let weak = inner.ui.as_weak();
                slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                    if let Some(ui) = weak.upgrade() {
                        ui.invoke_scroll_to_row(i as i32);
                    }
                });
            }
        }
    }

    /// 依目前的展開狀態攤平，放進 Slint。
    fn show_rows(&self) {
        let inner = &self.0;
        let mut s = inner.state.borrow_mut();
        let mut rows = Vec::new();
        flatten(&s.nodes, &mut Vec::new(), &mut rows);
        let parent = s.parent.clone();
        let model: Vec<TreeRow> = rows
            .iter()
            .filter_map(|p| node_ref(&s.nodes, p))
            .map(|n| TreeRow {
                key: n.task.key.as_str().into(),
                title: n.task.title.as_str().into(),
                suffix: if n.task.is_category {
                    format!("  · 分類 · {}", n.task.status)
                } else {
                    format!("  · {}", n.task.status)
                }
                .into(),
                depth: n.depth,
                has_children: !n.children.is_empty(),
                expanded: n.expanded,
                picked: parent.as_deref().is_some_and(|p| eq_ic(p, &n.task.key)),
                dim: matches!(n.task.status_category.as_str(), "DONE" | "CANCELLED"),
            })
            .collect();
        s.rows = rows;
        inner.ui.set_rows(ModelRc::new(VecModel::from(model)));
        inner.ui.set_root_picked(parent.is_none());
    }

    fn update_parent_text(&self) {
        let s = self.0.state.borrow();
        self.0.ui.set_parent_text(parent_text(&s.tasks, s.parent.as_deref()).into());
    }

    fn pick_parent(&self, key: Option<String>) {
        self.0.state.borrow_mut().parent = key;
        self.show_rows();
        self.update_parent_text();
    }

    /// 請 Claude 依任務名稱與 session 開頭的指令估預計工時。開視窗時自動問一次（使用者已經自己填了就不覆蓋），
    /// 按「✦ 問 Claude」重問會直接帶入。
    async fn estimate(&self, manual: bool) {
        let inner = &self.0;
        let ui = &inner.ui;
        let title = ui.get_new_title().trim().to_owned();
        if title.is_empty() {
            ui.set_estimate_hint(if manual { "先填名稱，Claude 才能估" } else { "" }.into());
            return;
        }
        let (abort, registration) = AbortHandle::new_pair();
        let seq = {
            let mut s = inner.state.borrow_mut();
            if s.closed {
                return;
            }
            if let Some(a) = s.estimate_abort.replace(abort) {
                a.abort();
            }
            s.estimate_seq += 1;
            s.estimate_seq
        };
        ui.set_estimating(true);
        ui.set_estimate_hint("Claude 估算中…".into());
        let project = match self.selected_project() {
            Some(p) => format!("{}（資料夾 {}）", p.name, inner.suggestion.unit_name),
            None => inner.suggestion.unit_name.clone(),
        };
        let file = inner.suggestion.session_file.clone();
        let work = async move {
            let prompts = match file {
                Some(f) => {
                    hb_backend::rt::bg(async move { ClaudeMonitor::read_opening_prompts(std::path::Path::new(&f)) })
                        .await
                }
                None => None,
            };
            ClaudeEstimator::estimate(&title, Some(&project), prompts.as_deref()).await
        };
        let hours = match Abortable::new(work, registration).await {
            Ok(h) => h,
            Err(_) => return, // 又重問了，或視窗關了
        };
        {
            let mut s = inner.state.borrow_mut();
            if s.estimate_seq != seq || s.closed {
                return;
            }
            s.estimate_abort = None;
        }
        ui.set_estimating(false);
        let Some(h) = hours else {
            ui.set_estimate_hint("問不到 Claude，請自己填".into());
            return;
        };
        let text = hours_text(h);
        if inner.state.borrow().estimate_edited && !manual && !ui.get_estimate().trim().is_empty() {
            ui.set_estimate_hint(format!("Claude 估 {text} 小時（保留你填的）").into());
            return;
        }
        ui.set_estimate(text.into());
        inner.state.borrow_mut().estimate_edited = false;
        ui.set_estimate_hint("Claude 估的，可以改".into());
    }

    /// 回應裡沒有編號時：到專案裡找剛建好的（同名稱、同上層、編號最大的那個）。
    async fn find_created(&self, project_key: &str, title: &str, parent: Option<&str>) -> Option<String> {
        match self.0.engine.api().list_project_tasks(project_key, None).await {
            Ok(tasks) => tasks
                .into_iter()
                .filter(|t| {
                    t.title == title
                        && match (&t.parent, parent) {
                            (None, None) => true,
                            (Some(a), Some(b)) => eq_ic(a, b),
                            _ => false,
                        }
                })
                .max_by_key(|t| t.key.rsplit('-').next().and_then(|n| n.parse::<i64>().ok()).unwrap_or(0))
                .map(|t| t.key),
            Err(e) => {
                log::error!("找剛建立的任務失敗: {}", e.0);
                None
            }
        }
    }

    async fn create(&self) {
        let inner = &self.0;
        let ui = &inner.ui;
        if ui.get_busy() {
            return;
        }
        let Some(project) = self.selected_project() else {
            ui.set_error("請選專案".into());
            return;
        };
        let title = ui.get_new_title().trim().to_owned();
        if title.is_empty() {
            ui.set_error("請填名稱".into());
            ui.invoke_focus_title();
            return;
        }
        let estimate_text = ui.get_estimate().to_string();
        let Some(estimate) = parse_estimate(&estimate_text) else {
            ui.set_error(
                if estimate_text.trim().is_empty() {
                    "請填預計工時（可以按「✦ 問 Claude」）"
                } else {
                    "預計工時請填小時數"
                }
                .into(),
            );
            ui.invoke_focus_estimate();
            return;
        };
        let parent = inner.state.borrow().parent.clone();
        let mut fields = Map::new();
        fields.insert("project".into(), json!(project.key));
        fields.insert("title".into(), json!(title));
        fields.insert("assignees".into(), json!(["me"]));
        fields.insert("estimatedHours".into(), json!(estimate));
        if let Some(p) = &parent {
            fields.insert("parent".into(), json!(p));
        }
        let status_index = ui.get_status();
        if let Some(status) = usize::try_from(status_index).ok().and_then(|i| ui.get_statuses().iter().nth(i)) {
            fields.insert("status".into(), Value::String(status.to_string()));
        }
        if let Some(start) = date_range::from_int(ui.get_plan_start()) {
            fields.insert("plannedStart".into(), json!(inputs::date_text(Some(start))));
        }
        if let Some(end) = date_range::from_int(ui.get_plan_end()) {
            fields.insert("plannedEnd".into(), json!(inputs::date_text(Some(end))));
        }

        let key = Rc::new(RefCell::new(String::new()));
        let weak = ui.as_weak();
        let ok = {
            let key = key.clone();
            let api = inner.engine.api();
            inputs::run(
                |busy| {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_busy(busy);
                    }
                },
                |message| {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_error(message.map(|m| m.text).unwrap_or_default().into());
                    }
                },
                move || async move {
                    *key.borrow_mut() = api.create_task(fields).await?;
                    Ok(())
                },
                None,
            )
            .await
        };
        if !ok {
            return;
        }
        let engine = &inner.engine;
        engine.remember_folder(&inner.suggestion.unit_path, &project.key, parent.as_deref());
        engine.forget_project_tree(&project.key);
        let mut created = key.borrow().clone();
        if created.is_empty() {
            created = self.find_created(&project.key, &title, parent.as_deref()).await.unwrap_or_default();
        }
        let created_key = (!created.is_empty()).then_some(created);
        {
            let mut s = inner.state.borrow_mut();
            s.result.created_key = created_key.clone();
            s.result.created_title = title.clone();
        }
        let assign = ui.get_assign();
        if let Some(k) = &created_key
            && assign
        {
            inner.state.borrow_mut().result.assigned = true;
            engine.assign_session_task(&inner.suggestion.session_id, k, true).await;
        } else if created_key.is_none() && assign {
            log::error!("建立「{title}」後找不到新任務的編號，沒辦法指定給 Claude session");
        }
        let e = engine.clone();
        dialog::spawn(async move { e.after_external_change().await });
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::TestInfo;

    #[test]
    fn 預計工時解析() {
        assert_eq!(Some(4.0), parse_estimate("4"));
        assert_eq!(Some(1.5), parse_estimate(" 1.5h "));
        assert_eq!(Some(2.0), parse_estimate("2 小時"));
        assert_eq!(None, parse_estimate(""));
        assert_eq!(None, parse_estimate("0"));
        assert_eq!(None, parse_estimate("-1"));
        assert_eq!(None, parse_estimate("兩小時"));
    }

    #[test]
    fn 小時顯示() {
        assert_eq!("2", hours_text(2.0));
        assert_eq!("1.5", hours_text(1.5));
        assert_eq!("0.33", hours_text(1.0 / 3.0));
    }

    fn t(key: &str, parent: Option<&str>) -> ProjectTask {
        ProjectTask {
            key: key.into(),
            title: format!("名稱{key}"),
            status: "處理中".into(),
            status_category: "IN_PROGRESS".into(),
            parent: parent.map(str::to_owned),
            planned_start: None,
            planned_end: None,
            is_category: false,
            assignees: None,
            priority: None,
            test: TestInfo::default(),
        }
    }

    #[test]
    fn 建立在_上層路徑() {
        let tasks = vec![t("A-1", None), t("A-2", Some("A-1")), t("A-3", Some("a-2"))];
        assert_eq!("最上層（不屬於任何任務）", parent_text(&tasks, None));
        assert_eq!("A-1 名稱A-1 › A-2 名稱A-2 › A-3 名稱A-3", parent_text(&tasks, Some("A-3")));
        assert_eq!("X-9", parent_text(&tasks, Some("X-9")));
    }

    #[test]
    fn 樹_只展開第一層與選中的上層() {
        let tasks = vec![t("A-1", None), t("A-2", Some("A-1")), t("A-3", Some("A-2")), t("A-4", Some("A-3"))];
        let roots = TaskTree::build(tasks.iter().cloned(), true, None);
        let picked: HashSet<String> = HashSet::new();
        let nodes: Vec<Node> = roots.iter().map(|r| Node::from(r, false, &picked, 0)).collect();
        let mut rows = Vec::new();
        flatten(&nodes, &mut Vec::new(), &mut rows);
        // A-1 展開（第一層）→ 看得到 A-2；A-2 沒展開
        assert_eq!(2, rows.len());
        let picked: HashSet<String> = ancestors_of(&tasks, "A-3").into_iter().map(|k| k.to_uppercase()).collect();
        let nodes: Vec<Node> = roots.iter().map(|r| Node::from(r, false, &picked, 0)).collect();
        let mut rows = Vec::new();
        flatten(&nodes, &mut Vec::new(), &mut rows);
        assert_eq!(4, rows.len());
    }
}
