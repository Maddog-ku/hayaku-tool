//! 對應 C# `UI/PanelWindow.xaml.cs`：點工作列上的工時後往上彈出的面板（計時／任務／今天／通知四個分頁）；失去焦點就收起來。
//! 任務分頁、任務選單、通知篩選、今天分頁的摘要、開始任務的遠端搜尋在 [`work`]（C# PanelWindow.Work.cs）；
//! 釘選托盤、待我審核、測試卡片在 [`pins`]（C# PanelWindow.Pins.cs）。

mod pins;
mod work;

pub use pins::PinRowVm;
pub use work::MyTaskRowVm;

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use chrono::{Local, NaiveDateTime};
use hb_api::{DateTimeOffset, KeyFinder, WorkMath};
use hb_core::TimerEngine;
use hb_core::ci_set::{cmp_ic, eq_ic};
use hb_ui_common::DayTimelineData;
use hb_ui_common::task_picker::PickItem;
use hb_ui_common::view_models::{MeetingRowVm, NotificationRowVm, ReminderRowVm, RepoRowVm, TaskRowVm, TodoRowVm};
use hb_ui_work::PageArg;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::popup::{self, PopupKind};
use crate::slide::{Edge, WindowSlide};
use crate::theme_manager::ThemeManager;
use crate::ui::{DayTimelineModel, MeetingRow, PanelTab, PanelWindow, PickRow, ReminderRow, RepoRow, TaskRow, TodoRow};

/// 面板的寬（C# Width 432 扣掉兩邊 6 的陰影邊；Rust 版視窗就是卡片）。
pub const PANEL_WIDTH: f32 = 432.0;
const TAB_NAMES: [&str; 4] = ["Timer", "Tasks", "Today", "Notify"];
/// 從工作列長出來、滑回去的時間（完整走一趟；中途反轉時依剩下的距離縮短）。
const OPEN_DURATION: Duration = Duration::from_millis(280);
const CLOSE_DURATION: Duration = Duration::from_millis(200);

pub(crate) fn spawn(fut: impl Future<Output = ()> + 'static) {
    if let Err(e) = slint::spawn_local(fut) {
        log::error!("無法在 UI 執行緒執行背景工作: {e}");
    }
}

pub(crate) fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

pub(crate) fn now_offset() -> DateTimeOffset {
    Local::now().fixed_offset()
}

/// 「99+」：徽章最多顯示到 99。
pub fn badge_text(n: usize) -> String {
    match n {
        0 => String::new(),
        1..=99 => n.to_string(),
        _ => "99+".to_owned(),
    }
}

/// 面板會變的狀態。
#[derive(Default)]
pub(crate) struct State {
    anchor: hb_win::Rect,
    align_left: bool,
    /// 開著選擇任務、刪除任務這類小視窗：失去焦點不收起
    pub(crate) dialog_open: bool,
    hiding: bool,
    animating: bool,
    render_queued: bool,
    shown: bool,
    stay_open: bool,
    hidden_at: Option<Instant>,
    start_focused: bool,

    pub(crate) running: Vec<TaskRowVm>,
    pub(crate) paused: Vec<TaskRowVm>,
    all_tasks: Vec<PickItem>,
    start_items: Vec<PickItem>,
    repos: Vec<RepoRowVm>,
    pub(crate) pins: Vec<PinRowVm>,
    pub(crate) my_tasks: Vec<MyTaskRowVm>,
    meetings: Vec<MeetingRowVm>,
    todos: Vec<TodoRowVm>,
    pub(crate) notifications: Vec<NotificationRowVm>,

    pub(crate) task_tab: String,
    pub(crate) notify_filter: String,
    pub(crate) discussions_loading: bool,
    pub(crate) discussions_error: Option<String>,
    /// 討論的來源：space（空間評論）、chat（聊天），None 是全部（跟網頁一樣是伺服器篩的）。
    pub(crate) discussion_source: Option<String>,
    /// 開始任務的遠端搜尋結果（key 不分大小寫）。
    search_cache: HashMap<String, Vec<PickItem>>,
    pub(crate) test_card_target: &'static str,
}

type Handlers<F> = RefCell<Vec<Rc<F>>>;
type WorkspaceHandler = dyn Fn(&str, Option<PageArg>);

pub(crate) struct PanelInner {
    pub(crate) engine: TimerEngine,
    pub(crate) ui: PanelWindow,
    tick: slint::Timer,
    search_delay: slint::Timer,
    /// 從工作列長出來、滑回去
    slide: WindowSlide,
    pub(crate) s: RefCell<State>,
    settings_requested: Handlers<dyn Fn()>,
    time_entries_requested: Handlers<dyn Fn()>,
    workspace_requested: Handlers<WorkspaceHandler>,
    pub(crate) me: Weak<PanelInner>,
    running_model: Rc<VecModel<TaskRow>>,
    paused_model: Rc<VecModel<TaskRow>>,
}

/// 面板。App 持有一個；開關只是顯示／隱藏。
#[derive(Clone)]
pub struct Panel(Rc<PanelInner>);

/// 在回呼裡拿面板（面板沒了就不做）。
macro_rules! with_panel {
    ($me:expr, |$p:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = $me.clone();
        move |$($arg),*| {
            if let Some($p) = weak.upgrade() {
                $body
            }
        }
    }};
}
pub(crate) use with_panel;

impl Panel {
    pub fn new(engine: TimerEngine) -> Option<Panel> {
        let ui = match popup::create(PopupKind::Panel, PanelWindow::new) {
            Ok(ui) => ui,
            Err(e) => {
                log::error!("建立面板失敗: {e}");
                return None;
            }
        };
        crate::track_theme!(ui);
        let running_model = Rc::new(VecModel::default());
        let paused_model = Rc::new(VecModel::default());
        ui.set_running(ModelRc::from(running_model.clone()));
        ui.set_paused(ModelRc::from(paused_model.clone()));
        let slide = WindowSlide::new();
        let weak_ui = ui.as_weak();
        slide.set_on_reveal(move || {
            if let Some(u) = weak_ui.upgrade() {
                u.set_repaint_token(u.get_repaint_token().wrapping_add(1));
            }
        });
        let inner = Rc::new_cyclic(|me| PanelInner {
            engine: engine.clone(),
            ui,
            tick: slint::Timer::default(),
            search_delay: slint::Timer::default(),
            slide,
            s: RefCell::new(State {
                task_tab: TimerEngine::ALL_TAB.to_owned(),
                notify_filter: "all".to_owned(),
                test_card_target: "fix",
                ..Default::default()
            }),
            settings_requested: RefCell::new(Vec::new()),
            time_entries_requested: RefCell::new(Vec::new()),
            workspace_requested: RefCell::new(Vec::new()),
            me: me.clone(),
            running_model,
            paused_model,
        });
        // 分頁：任務的徽章是逾期（紅）、今天是待辦與會議（灰）、通知是未讀（橘）
        let tabs: Vec<PanelTab> = [("計時", 1), ("任務", 0), ("今天", 1), ("通知", 2)]
            .iter()
            .map(|(label, kind)| PanelTab { label: (*label).into(), badge: SharedString::new(), badge_kind: *kind })
            .collect();
        inner.ui.set_tabs(ModelRc::new(VecModel::from(tabs)));
        inner.wire();
        inner.wire_pins();
        inner.init_work();

        let me = inner.me.clone();
        engine.events().changed.subscribe(with_panel!(me, |p, _a| p.queue_render()));
        let me = inner.me.clone();
        // 列表裡的顏色是 Slint 依 Theme 算的，換主題不用重建；跟 C# 一樣照樣重畫一次
        ThemeManager::on_changed(with_panel!(me, |p| p.queue_render()));
        Some(Panel(inner))
    }

    pub fn on_settings_requested(&self, f: impl Fn() + 'static) {
        self.0.settings_requested.borrow_mut().push(Rc::new(f));
    }

    pub fn on_time_entries_requested(&self, f: impl Fn() + 'static) {
        self.0.time_entries_requested.borrow_mut().push(Rc::new(f));
    }

    /// 要開工作台：頁面、參數（任務編號、日期、"new"…）。
    pub fn on_workspace_requested(&self, f: impl Fn(&str, Option<PageArg>) + 'static) {
        self.0.workspace_requested.borrow_mut().push(Rc::new(f));
    }

    /// 上一次收起的時間；用來避免「點工作列 → 收起 → 又被同一下點擊打開」。
    pub fn hidden_at(&self) -> Option<Instant> {
        self.0.s.borrow().hidden_at
    }

    /// 失去焦點也不收起（--panel 啟動時用，方便檢查畫面）。
    pub fn set_stay_open(&self, stay: bool) {
        self.0.s.borrow_mut().stay_open = stay;
    }

    /// 面板開著（收起動畫進行中不算）。
    pub fn is_open(&self) -> bool {
        self.0.is_open()
    }

    pub fn show_at(&self, anchor: hb_win::Rect, align_left: bool) {
        self.0.show_at(anchor, align_left);
    }

    pub fn hide_panel(&self) {
        self.0.hide_panel();
    }

    /// 直接切到某個分頁：Timer、Tasks、Today、Notify。
    pub fn select_tab(&self, tab: &str) {
        let index = match tab {
            "Tasks" => 1,
            "Today" => 2,
            "Notify" => 3,
            _ => 0,
        };
        self.0.ui.set_tab(index);
        self.0.tab_checked(index);
    }

    /// 啟動後閒置時先把面板的資料建好，第一次打開才不會卡一下（C# Prewarm）。
    pub fn prewarm(&self) {
        if self.0.s.borrow().shown {
            return;
        }
        self.0.render();
    }

    /// 定期清暫存：任務搜尋的快取。
    pub fn trim_caches(&self) {
        self.0.s.borrow_mut().search_cache.clear();
    }

    pub fn window(&self) -> &slint::Window {
        self.0.ui.window()
    }
}

impl PanelInner {
    pub(crate) fn weak(&self) -> Weak<PanelInner> {
        self.me.clone()
    }

    fn is_open(&self) -> bool {
        let s = self.s.borrow();
        s.shown && !s.hiding
    }

    pub(crate) fn request_workspace(&self, page: &str, arg: Option<PageArg>) {
        self.hide_panel();
        let handlers: Vec<_> = self.workspace_requested.borrow().clone();
        for h in handlers {
            h(page, arg.clone());
        }
    }

    fn wire(&self) {
        let ui = &self.ui;
        let me = self.weak();
        ui.on_key_pressed(move |e| me.upgrade().is_some_and(|p| p.key_pressed(&e)));
        ui.on_tab_selected(with_panel!(self.me, |p, i| p.tab_checked(i)));
        ui.on_login(with_panel!(self.me, |p| {
            let e = p.engine.clone();
            spawn(async move { e.login().await });
        }));
        // 計時
        ui.on_running_open(with_panel!(self.me, |p, i| p.task_row_open(true, i)));
        ui.on_paused_open(with_panel!(self.me, |p, i| p.task_row_open(false, i)));
        ui.on_running_act(with_panel!(self.me, |p, i| p.task_action(true, i)));
        ui.on_paused_act(with_panel!(self.me, |p, i| p.task_action(false, i)));
        ui.on_running_more(with_panel!(self.me, |p, i, x, y| p.task_row_menu(true, i, x, y)));
        ui.on_paused_more(with_panel!(self.me, |p, i, x, y| p.task_row_menu(false, i, x, y)));
        ui.on_running_status(with_panel!(self.me, |p, i, x, y| p.task_row_status(true, i, x, y)));
        ui.on_paused_status(with_panel!(self.me, |p, i, x, y| p.task_row_status(false, i, x, y)));
        ui.on_start_edited(with_panel!(self.me, |p, _t| p.filter_start()));
        ui.on_start_focus(with_panel!(self.me, |p, focused| {
            p.s.borrow_mut().start_focused = focused;
            if focused {
                p.filter_start();
            } else {
                // C#：滑鼠在清單上（點選中）不收起；Slint 點清單不會拿走輸入框的焦點，這裡直接收起
                p.ui.set_start_popup_open(false);
            }
        }));
        ui.on_start_pick(with_panel!(self.me, |p, i| {
            p.ui.set_start_index(i);
            p.start_selected();
        }));
        ui.on_start_accept(with_panel!(self.me, |p| p.start_selected()));
        ui.on_repo_pick(with_panel!(self.me, |p, i| p.repo_pick(i as usize)));
        // 今天
        ui.on_meeting_open(with_panel!(self.me, |p, i| {
            let item = p.s.borrow().meetings.get(i as usize).map(|m| m.item.clone());
            if let Some(m) = item {
                hb_win::open(&p.engine.meeting_url(&m.id));
            }
        }));
        ui.on_meeting_join(with_panel!(self.me, |p, i| p.join_meeting(i as usize)));
        ui.on_todo_done(with_panel!(self.me, |p, i| p.todo_checked(i as usize)));
        ui.on_capture(with_panel!(self.me, |p| p.capture()));
        // 通知
        ui.on_notif_open(with_panel!(self.me, |p, i| {
            let item = p.s.borrow().notifications.get(i as usize).map(|n| n.item.clone());
            if let Some(n) = item {
                let e = p.engine.clone();
                spawn(async move { e.open_notification(&n).await });
            }
        }));
        ui.on_mark_all(with_panel!(self.me, |p| {
            p.ui.set_mark_all_enabled(false);
            let e = p.engine.clone();
            spawn(async move { e.mark_notifications_read(None).await });
        }));
        ui.on_open_notifications(with_panel!(self.me, |p| hb_win::open(&p.engine.web_url("/notifications"))));
        // 底部
        ui.on_clock_in(with_panel!(self.me, |p| p.clock_in()));
        ui.on_clock_out(with_panel!(self.me, |p| p.clock_out()));
        ui.on_entries(with_panel!(self.me, |p| {
            p.hide_panel();
            let handlers: Vec<_> = p.time_entries_requested.borrow().clone();
            for h in handlers {
                h();
            }
        }));
        ui.on_refresh(with_panel!(self.me, |p| {
            let e = p.engine.clone();
            spawn(async move { e.refresh_all(true).await });
        }));
        ui.on_settings(with_panel!(self.me, |p| {
            p.hide_panel();
            let handlers: Vec<_> = p.settings_requested.borrow().clone();
            for h in handlers {
                h();
            }
        }));

        // 失去焦點就收起來（C# Deactivated）；大小變了重新擺位置（C# SizeChanged）
        let me = self.weak();
        ui.window().on_winit_window_event(move |_w, event| {
            match event {
                WindowEvent::Focused(false) => {
                    let me = me.clone();
                    slint::Timer::single_shot(Duration::ZERO, move || {
                        if let Some(p) = me.upgrade() {
                            p.deactivated();
                        }
                    });
                }
                WindowEvent::Resized(_) => {
                    let me = me.clone();
                    slint::Timer::single_shot(Duration::ZERO, move || {
                        if let Some(p) = me.upgrade()
                            && p.s.borrow().shown
                        {
                            p.log_size("大小改變");
                            p.reposition();
                        }
                    });
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }

    fn deactivated(&self) {
        self.ui.set_start_popup_open(false);
        let (dialog, stay) = {
            let s = self.s.borrow();
            (s.dialog_open, s.stay_open)
        };
        if !dialog && !stay {
            self.hide_panel();
        }
    }

    /// C# PreviewKeyDown：Alt＋數字切換托盤上的任務；Esc 先收起開始任務的清單，再收起面板。
    fn key_pressed(&self, e: &slint::private_unstable_api::re_exports::KeyEvent) -> bool {
        if self.handle_pin_hotkey(e) {
            return true;
        }
        if e.text.as_str() != "\u{1b}" {
            return false;
        }
        if self.ui.get_start_popup_open() {
            self.ui.set_start_popup_open(false);
        } else {
            self.hide_panel();
        }
        true
    }

    /// 資料變了要重畫：同一輪的多次變動合併成一次；長出來的動畫進行中先不重建清單，等動畫結束再畫。
    pub(crate) fn queue_render(&self) {
        {
            let mut s = self.s.borrow_mut();
            if !s.shown || s.render_queued {
                return;
            }
            s.render_queued = true;
        }
        let me = self.weak();
        slint::Timer::single_shot(Duration::ZERO, move || {
            let Some(p) = me.upgrade() else { return };
            if p.s.borrow().animating {
                return; // 動畫結束時會補畫
            }
            p.s.borrow_mut().render_queued = false;
            if p.s.borrow().shown {
                p.render();
            }
        });
    }

    fn show_at(&self, anchor: hb_win::Rect, align_left: bool) {
        {
            let mut s = self.s.borrow_mut();
            s.anchor = anchor;
            s.align_left = align_left;
            s.hiding = false;
        }
        let reduce = ThemeManager::reduce_motion();
        let was_shown = self.s.borrow().shown;
        if !was_shown {
            // 內容先藏著，視窗長出來時再淡入、清單依序浮現
            self.ui.invoke_hide_content();
            self.ui.invoke_reset_stagger();
        }
        self.render();
        if !was_shown {
            // 從工作列那一側長出來：先擺在藏起來的位置（已經有原生視窗時連裁切一起設好），再顯示
            self.place(if reduce { 1.0 } else { 0.0 });
            if let Err(e) = self.ui.show() {
                log::error!("顯示面板失敗: {e}");
                return;
            }
            popup::apply_styles(self.ui.window(), PopupKind::Panel);
            self.s.borrow_mut().shown = true;
            // 第一次顯示才有原生視窗：馬上補上裁切（還沒畫到螢幕上）；
            // 事件迴圈還沒開始時連原生視窗都還沒有，沒辦法播，直接出現在定位
            let ready = popup::hwnd(self.ui.window()).is_some();
            self.place(if reduce || !ready { 1.0 } else { self.slide.progress() });
        } else {
            self.reposition();
        }
        hb_ui_common::window_util::activate(self.ui.window());
        // 動畫結束才開始同步：同步一開始就會觸發重畫，跟動畫搶時間就會卡
        let me = self.weak();
        let completed = move || {
            let Some(p) = me.upgrade() else { return };
            let queued = {
                let mut s = p.s.borrow_mut();
                s.animating = false;
                std::mem::take(&mut s.render_queued)
            };
            if queued {
                p.render();
            }
            if p.is_open() {
                let e = p.engine.clone();
                spawn(async move { e.refresh_all(false).await });
            }
        };
        if reduce || popup::hwnd(self.ui.window()).is_none() {
            self.slide.jump(1.0);
            self.ui.invoke_show_content();
            self.ui.invoke_play_stagger();
            completed();
        } else {
            self.s.borrow_mut().animating = true;
            self.ui.invoke_show_content();
            // 清單晚一點浮現：跟視窗長出來疊在一起
            let me = self.weak();
            slint::Timer::single_shot(Duration::from_millis(90), move || {
                if let Some(p) = me.upgrade()
                    && p.is_open()
                {
                    p.ui.invoke_play_stagger();
                }
            });
            self.slide.run(1.0, OPEN_DURATION, completed);
        }
        let me = self.weak();
        self.tick.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
            if let Some(p) = me.upgrade() {
                p.update_numbers();
                // 計時中的狀態點呼吸：只在面板開著、有計時時每秒切換一次（沒在計時、「顯示動畫」關閉時不動，免得一直重畫）
                if !p.s.borrow().running.is_empty() && !ThemeManager::reduce_motion() {
                    p.ui.set_pulse(!p.ui.get_pulse());
                }
            }
        });
    }

    pub(crate) fn hide_panel(&self) {
        {
            let mut s = self.s.borrow_mut();
            if !s.shown || s.hiding {
                return;
            }
            s.hidden_at = Some(Instant::now());
            s.animating = false;
            s.render_queued = false;
        }
        self.tick.stop();
        self.ui.set_start_popup_open(false);
        if ThemeManager::reduce_motion() {
            self.slide.stop();
            let _ = self.ui.hide();
            self.s.borrow_mut().shown = false;
            return;
        }
        // 滑回工作列，播完才藏起來；途中又被打開時由 show_at 接手（這裡的收尾不會執行）
        self.s.borrow_mut().hiding = true;
        let me = self.weak();
        self.slide.run(0.0, CLOSE_DURATION, move || {
            let Some(p) = me.upgrade() else { return };
            if !p.s.borrow().hiding {
                return;
            }
            let _ = p.ui.hide();
            let mut s = p.s.borrow_mut();
            s.hiding = false;
            s.shown = false;
        });
    }

    /// 視窗的目標範圍、裁切與方向（實體像素）。
    fn geometry(&self) -> (hb_win::Rect, hb_win::Rect, Edge, f32) {
        let (anchor, align_left) = {
            let s = self.s.borrow();
            (s.anchor, s.align_left)
        };
        let scale = popup::scale_at(anchor.left, anchor.top - 1);
        let estimate = ((PANEL_WIDTH * scale).round() as i32, (self.ui.get_pref_h() * scale).round() as i32);
        let size = match popup::outer_size(self.ui.window()) {
            Some((w, h)) if (h - estimate.1).abs() <= 2 => (w, h),
            _ => estimate,
        };
        let (target, clip, edge) = popup::above_anchor(anchor, align_left, size, scale);
        (target, clip, edge, scale)
    }

    /// 擺到進度 p 的位置（0 藏在工作列那一側、1 在定位）。
    fn place(&self, p: f32) {
        let (target, clip, edge, scale) = self.geometry();
        let hwnd = popup::hwnd(self.ui.window());
        self.slide.prepare(hwnd, target, clip, edge, popup::corner_radius(scale), p);
        if hwnd.is_none() {
            let (x, y) = self.slide.position();
            self.ui.window().set_position(slint::PhysicalPosition::new(x, y));
        }
    }

    /// 診斷用（面板跑版：一圈深色空白、底部錯位）：記下原生視窗、Slint 視窗與內容高度，對不上時才看得出是哪個沒跟上。
    fn log_size(&self, reason: &str) {
        let window = self.ui.window();
        let slint_size = window.size();
        let scale = window.scale_factor();
        let content = (self.ui.get_pref_h() * scale).round() as i32;
        let native = popup::outer_size(window);
        log::info!(
            "面板{reason}：原生視窗 {native:?}，Slint 視窗 {}x{}，內容高度 {content}（縮放 {scale}）",
            slint_size.width,
            slint_size.height
        );
    }

    /// C# Reposition：從工作列工時正上方出現；動畫途中大小變了就換目標，接著播。
    fn reposition(&self) {
        let (target, clip, edge, _) = self.geometry();
        self.slide.retarget(popup::hwnd(self.ui.window()), target, clip, edge);
        if popup::hwnd(self.ui.window()).is_none() {
            self.ui.window().set_position(slint::PhysicalPosition::new(target.left, target.top));
        }
    }

    // ───────────── 顯示 ─────────────

    pub(crate) fn render(&self) {
        let engine = &self.engine;
        let panel_height = engine.config().panel_height;
        self.ui.set_content_height(if panel_height > 0 { panel_height as f32 } else { 0.0 });

        let logged_in = engine.logged_in();
        let initial_syncing = engine.initial_syncing();
        self.ui.set_initial_syncing(initial_syncing);
        self.ui.set_show_login(!(logged_in || initial_syncing));
        self.ui.set_login_enabled(!engine.logging_in());
        self.ui.set_login_text(if engine.logging_in() { "等待瀏覽器登入…" } else { "登入 Hayaku" }.into());
        self.ui.set_main_visible(logged_in && !initial_syncing);
        let enabled = logged_in && !initial_syncing;
        self.ui.set_clock_in_enabled(enabled);
        self.ui.set_clock_out_enabled(enabled);
        self.ui.set_entries_enabled(enabled);
        let synced = engine.last_synced();
        self.ui.set_sync_text(sync_text(engine.syncing(), engine.auto_sync_paused(), synced).into());
        self.ui.set_sync_tip(
            if engine.auto_sync_paused() {
                "不在上班時間，暫停自動同步（打開面板或按 ⟳ 仍會同步）；可在設定 › 一般關掉"
            } else {
                ""
            }
            .into(),
        );
        self.ui.set_refresh_enabled(!engine.syncing());
        self.ui.set_error_text(engine.error().unwrap_or_default().into());

        let now = now();
        self.ui.set_user_text(engine.user().map_or_else(|| "Hayaku".to_owned(), |u| u.display_name).into());
        let phase = WorkMath::phase(engine.schedule().as_ref(), now, engine.state().clocked_out).text;
        self.ui.set_status_text(phase.into());

        self.render_pins();
        self.render_timer(now);
        self.render_tasks();
        self.render_reviews();
        self.render_today();
        self.render_today_extras();
        self.render_notifications();
        self.update_numbers();
    }

    fn task_row(vm: &TaskRowVm) -> TaskRow {
        TaskRow {
            key: vm.key.as_str().into(),
            title: vm.title.as_str().into(),
            parent_prefix: vm.parent_prefix().into(),
            icon: vm.icon().into(),
            icon_color: vm.icon_color().code(),
            icon_tip: vm.icon_tip().into(),
            status: if vm.status_visible() {
                vm.status.clone().unwrap_or_default().into()
            } else {
                SharedString::new()
            },
            elapsed: vm.elapsed.as_str().into(),
            action_glyph: vm.action_glyph().into(),
            action_tip: vm.action_tip().into(),
            not_busy: vm.not_busy(),
            running: vm.running,
        }
    }

    fn render_timer(&self, now: NaiveDateTime) {
        let engine = &self.engine;
        let running = engine.running_keys();
        let entries = engine.entries();
        // 依任務分組（不分大小寫，保留第一次出現的順序）：最後一筆的名稱、最後結束的時間（計時中算最大）
        struct Group {
            key: String,
            title: String,
            running: bool,
            last_end: Option<DateTimeOffset>,
        }
        let mut groups: Vec<Group> = Vec::new();
        for e in entries.iter() {
            let end = e.ended_at;
            match groups.iter_mut().find(|g| eq_ic(&g.key, &e.task)) {
                Some(g) => {
                    g.title = e.title.clone();
                    g.last_end = max_end(g.last_end, end);
                }
                None => groups.push(Group {
                    key: e.task.clone(),
                    title: e.title.clone(),
                    running: running.contains(&e.task),
                    last_end: end,
                }),
            }
        }
        let mut running_rows: Vec<&Group> = groups.iter().filter(|g| g.running).collect();
        running_rows.sort_by(|a, b| cmp_ic(&a.key, &b.key));
        let mut paused_rows: Vec<&Group> = groups.iter().filter(|g| !g.running).collect();
        // OrderByDescending(LastEnd)：計時中（沒有結束時間）算最大；穩定排序
        paused_rows.sort_by_key(|g| std::cmp::Reverse(end_key(g.last_end)));
        let running_vms: Vec<TaskRowVm> = running_rows
            .iter()
            .map(|t| TaskRowVm {
                key: t.key.clone(),
                title: t.title.clone(),
                running: true,
                is_claude: engine.is_claude_owned(&t.key),
                status: engine.status_of(&t.key),
                parent_title: engine.parent_title_of(&t.key),
                ..Default::default()
            })
            .collect();
        let paused_vms: Vec<TaskRowVm> = paused_rows
            .iter()
            .map(|t| TaskRowVm {
                key: t.key.clone(),
                title: t.title.clone(),
                status: engine.status_of(&t.key),
                parent_title: engine.parent_title_of(&t.key),
                ..Default::default()
            })
            .collect();
        self.running_model.set_vec(running_vms.iter().map(Self::task_row).collect::<Vec<_>>());
        self.paused_model.set_vec(paused_vms.iter().map(Self::task_row).collect::<Vec<_>>());

        let all_tasks: Vec<PickItem> = engine
            .my_tasks()
            .iter()
            .map(|t| PickItem {
                key: t.key.clone(),
                title: t.title.clone(),
                detail: format!("{} · {}", t.project, t.status),
            })
            .collect();

        // 主資料夾裡可能有很多專案：只列活動中或今天有用過的，都沒有就列主資料夾
        let idle = engine.config().claude_idle_minutes;
        let repos: Vec<RepoRowVm> = {
            let claude = engine.claude();
            let mut units: Vec<_> =
                claude.repos().filter(|r| r.active || r.last_activity.date() == now.date()).collect();
            units.sort_by(|a, b| b.active.cmp(&a.active).then(b.last_activity.cmp(&a.last_activity)));
            if units.is_empty() {
                units = claude.repos().filter(|r| r.root_path.is_none()).collect();
                units.sort_by(|a, b| a.name().cmp(b.name()));
            }
            units.into_iter().map(|r| RepoRowVm::from(r, now, idle)).collect()
        };
        self.ui.set_repos(ModelRc::new(VecModel::from(
            repos
                .iter()
                .map(|r| RepoRow {
                    name: r.name.as_str().into(),
                    path: r.path.as_str().into(),
                    status_text: r.status_text.as_str().into(),
                    status_color: r.status_color.code(),
                    task_text: r.task_text.as_str().into(),
                    task_tip: r.task_tip.as_str().into(),
                    pick_visible: r.pick_visible,
                })
                .collect::<Vec<_>>(),
        )));
        let mut s = self.s.borrow_mut();
        s.running = running_vms;
        s.paused = paused_vms;
        s.all_tasks = all_tasks;
        s.repos = repos;
    }

    fn render_today(&self) {
        let engine = &self.engine;
        let offset_now = now_offset();
        let meetings: Vec<MeetingRowVm> =
            engine.meetings().iter().map(|m| MeetingRowVm::from(m, &offset_now)).collect();
        self.ui.set_meetings(ModelRc::new(VecModel::from(
            meetings
                .iter()
                .map(|m| MeetingRow {
                    time: m.time.as_str().into(),
                    time_color: m.time_color.code(),
                    title: m.title.as_str().into(),
                    detail: m.detail.as_str().into(),
                    join_visible: m.join_visible,
                    joining: false,
                })
                .collect::<Vec<_>>(),
        )));
        let todos: Vec<TodoRowVm> = engine.todos().iter().cloned().map(TodoRowVm::new).collect();
        self.ui.set_todos(ModelRc::new(VecModel::from(
            todos
                .iter()
                .map(|t| TodoRow { text: t.text().into(), detail: t.detail().into(), busy: t.busy })
                .collect::<Vec<_>>(),
        )));
        let reminders: Vec<ReminderRow> = engine
            .reminders()
            .iter()
            .filter(|r| r.at.is_none_or(|at| at > offset_now))
            .take(5)
            .map(|r| {
                let vm = ReminderRowVm { item: r.clone() };
                ReminderRow { time: vm.time().into(), text: vm.text().into() }
            })
            .collect();
        self.ui.set_reminders(ModelRc::new(VecModel::from(reminders)));
        let pending = engine.meetings().iter().filter(|m| m.end_at > offset_now).count() + todos.len();
        self.set_tab_badge(2, badge_text(pending));
        let mut s = self.s.borrow_mut();
        s.meetings = meetings;
        s.todos = todos;
    }

    /// 分頁上的徽章：1 任務（逾期）、2 今天、3 通知。
    pub(crate) fn set_tab_badge(&self, index: usize, text: String) {
        let model = self.ui.get_tabs();
        if let Some(mut tab) = model.row_data(index)
            && tab.badge.as_str() != text
        {
            tab.badge = text.into();
            model.set_row_data(index, tab);
        }
    }

    fn update_numbers(&self) {
        let engine = &self.engine;
        if !engine.logged_in() {
            return;
        }
        // 上班中只給「下班」，下班後（或還沒上班）只給「上班」；放在每秒更新裡，跨過上下班時間會自己切換
        let on_duty = engine.on_duty();
        self.ui.set_clock_in_visible(!on_duty);
        self.ui.set_clock_out_visible(on_duty);

        let now = now();
        let today = now.date();
        let day = today.and_hms_opt(0, 0, 0).unwrap_or(now);
        let entries = engine.entries();
        let schedule = engine.schedule();
        let worked = WorkMath::union_minutes(entries.iter(), day, now);
        let expected = schedule.as_ref().map_or(0, |s| s.expected_minutes) as f64;
        self.ui.set_total_text(WorkMath::hours_text(worked).into());
        self.ui.set_expected_text(
            if expected > 0.0 { format!(" / {}", WorkMath::hours_text(expected)) } else { String::new() }.into(),
        );
        self.ui.set_progress(if expected > 0.0 { (worked / expected).min(1.0) as f32 } else { 0.0 });
        self.ui.set_summary_detail(
            if !WorkMath::is_filled(worked, expected) {
                format!("還差 {}", WorkMath::hours_minutes(expected - worked))
            } else {
                WorkMath::hours_minutes(worked)
            }
            .into(),
        );
        let data = DayTimelineData::new(today, schedule.as_ref().map(|s| s.working.as_slice()), &entries, now, true);
        self.ui.set_timeline(hb_ui_common::day_timeline_model!(DayTimelineModel, data));

        let mut s = self.s.borrow_mut();
        let State { running, paused, .. } = &mut *s;
        for (vm, model) in [(running, &self.running_model), (paused, &self.paused_model)] {
            for (i, vm) in vm.iter_mut().enumerate() {
                let elapsed = WorkMath::clock(WorkMath::task_minutes(entries.iter(), &vm.key, day, now));
                if vm.elapsed != elapsed {
                    vm.elapsed = elapsed;
                    if let Some(mut row) = model.row_data(i) {
                        row.elapsed = vm.elapsed.as_str().into();
                        model.set_row_data(i, row);
                    }
                }
            }
        }
        drop(s);
        self.update_pin_elapsed(now);
    }

    fn tab_checked(&self, tab: i32) {
        self.ui.set_start_popup_open(false);
        if TAB_NAMES.get(tab as usize) == Some(&"Notify") && self.s.borrow().notify_filter == "discussions" {
            self.load_discussions(false);
        }
    }

    // ───────────── 計時 ─────────────

    fn row_vm(&self, running: bool, index: i32) -> Option<TaskRowVm> {
        let s = self.s.borrow();
        let list = if running { &s.running } else { &s.paused };
        list.get(index as usize).cloned()
    }

    fn set_row_busy(&self, running: bool, key: &str, busy: bool) {
        let model = if running { &self.running_model } else { &self.paused_model };
        let mut s = self.s.borrow_mut();
        let list = if running { &mut s.running } else { &mut s.paused };
        if let Some((i, vm)) = list.iter_mut().enumerate().find(|(_, v)| v.key == key) {
            vm.busy = busy;
            if i < model.row_count() {
                model.set_row_data(i, Self::task_row(vm));
            }
        }
    }

    fn task_action(&self, running: bool, index: i32) {
        let Some(vm) = self.row_vm(running, index) else { return };
        self.set_row_busy(running, &vm.key, true);
        let me = self.weak();
        let engine = self.engine.clone();
        spawn(async move {
            if vm.running {
                engine.pause_task(&vm.key).await;
            } else {
                engine.start_task(&vm.key).await;
            }
            if let Some(p) = me.upgrade() {
                p.set_row_busy(running, &vm.key, false);
            }
        });
    }

    fn task_row_open(&self, running: bool, index: i32) {
        if let Some(vm) = self.row_vm(running, index) {
            self.request_workspace("tasks", Some(PageArg::Text(vm.key)));
        }
    }

    fn task_row_menu(&self, running: bool, index: i32, x: f32, y: f32) {
        if let Some(vm) = self.row_vm(running, index) {
            let anchor = crate::menu::to_screen(self.ui.window(), x, y, 1.0, 1.0);
            self.show_task_menu(vm.key, anchor);
        }
    }

    fn task_row_status(&self, running: bool, index: i32, x: f32, y: f32) {
        if let Some(vm) = self.row_vm(running, index) {
            let anchor = crate::menu::to_screen(self.ui.window(), x, y, 1.0, 1.0);
            self.show_status_menu(vm.key, anchor);
        }
    }

    /// 依輸入篩選我的任務；輸入的編號不在清單裡時，也提供「直接開始」。
    pub(crate) fn filter_start(&self) {
        let q = self.ui.get_start_text().trim().to_owned();
        let pins = self.engine.pins();
        let (items, local_count) = {
            let s = self.s.borrow();
            let remote = s.search_cache.get(&q.to_uppercase()).cloned().unwrap_or_default();
            start_candidates(&q, &pins, &s.all_tasks, &remote)
        };
        self.queue_remote_search(&q, local_count);
        let count = items.len();
        self.ui.set_start_items(ModelRc::new(VecModel::from(
            items
                .iter()
                .map(|i| PickRow {
                    key: i.key.as_str().into(),
                    title: i.title.as_str().into(),
                    detail: i.detail.as_str().into(),
                })
                .collect::<Vec<_>>(),
        )));
        self.ui.set_start_index(if count > 0 { 0 } else { -1 });
        let focused = self.s.borrow().start_focused;
        self.ui.set_start_popup_open(focused && count > 0);
        self.s.borrow_mut().start_items = items;
    }

    fn start_selected(&self) {
        let index = self.ui.get_start_index();
        let key = {
            let s = self.s.borrow();
            let selected = if self.ui.get_start_popup_open() && index >= 0 {
                s.start_items.get(index as usize).map(|i| i.key.clone())
            } else {
                None
            };
            selected.or_else(|| KeyFinder::parse(Some(self.ui.get_start_text().as_str())))
        };
        let Some(key) = key else { return };
        self.ui.set_start_popup_open(false);
        self.ui.set_start_text(SharedString::new());
        self.ui.invoke_clear_start_focus();
        let engine = self.engine.clone();
        spawn(async move { engine.start_task(&key).await });
    }

    /// 本機清單找不到幾筆時，停頓一下就去 Hayaku 搜尋（任務編號或名稱）。
    fn queue_remote_search(&self, q: &str, local_count: usize) {
        self.search_delay.stop();
        if q.chars().count() < 2 || local_count >= 6 || self.s.borrow().search_cache.contains_key(&q.to_uppercase()) {
            return;
        }
        let me = self.weak();
        self.search_delay.start(slint::TimerMode::SingleShot, Duration::from_millis(400), move || {
            if let Some(p) = me.upgrade() {
                let q = p.ui.get_start_text().trim().to_owned();
                p.search_remote(q);
            }
        });
    }

    fn search_remote(&self, q: String) {
        if q.chars().count() < 2 || self.s.borrow().search_cache.contains_key(&q.to_uppercase()) {
            return;
        }
        let api = self.engine.api();
        let me = self.weak();
        spawn(async move {
            let items = match api.search_tasks(&q).await {
                Ok(hits) => hits
                    .into_iter()
                    .map(|h| PickItem { key: h.key, title: h.title, detail: format!("{} · 搜尋結果", h.project) })
                    .collect(),
                Err(e) => {
                    log::error!("搜尋任務失敗: {e}");
                    Vec::new()
                }
            };
            let Some(p) = me.upgrade() else { return };
            p.s.borrow_mut().search_cache.insert(q.to_uppercase(), items);
            if p.ui.get_start_text().trim() == q && p.s.borrow().start_focused {
                p.filter_start();
            }
        });
    }

    // ───────────── 今天 ─────────────

    fn join_meeting(&self, index: usize) {
        let Some(item) = self.s.borrow().meetings.get(index).map(|m| m.item.clone()) else { return };
        let model = self.ui.get_meetings();
        if let Some(mut row) = model.row_data(index) {
            row.joining = true;
            model.set_row_data(index, row);
        }
        let engine = self.engine.clone();
        spawn(async move { engine.join_meeting(&item).await });
    }

    fn todo_checked(&self, index: usize) {
        let item = {
            let mut s = self.s.borrow_mut();
            let Some(t) = s.todos.get_mut(index) else { return };
            t.busy = true;
            t.item.clone()
        };
        let model = self.ui.get_todos();
        if let Some(mut row) = model.row_data(index) {
            row.busy = true;
            model.set_row_data(index, row);
        }
        let engine = self.engine.clone();
        spawn(async move { engine.complete_todo(&item).await });
    }

    fn capture(&self) {
        let text = self.ui.get_capture_text().trim().to_owned();
        if text.is_empty() {
            return;
        }
        self.ui.set_capture_enabled(false);
        let todo = self.ui.get_capture_todo();
        let me = self.weak();
        let engine = self.engine.clone();
        spawn(async move {
            engine.quick_capture(&text, todo).await;
            let Some(p) = me.upgrade() else { return };
            p.ui.set_capture_enabled(true);
            if p.engine.error().is_none() {
                p.ui.set_capture_text(SharedString::new());
            }
        });
    }

    // ───────────── 底部 ─────────────

    fn clock_in(&self) {
        let me = self.weak();
        let engine = self.engine.clone();
        spawn(async move {
            let tasks = if engine.resume_candidates().is_empty() {
                let Some(p) = me.upgrade() else { return };
                let picked = p.pick_tasks("上班要開始計時哪些任務？（可多選）", true).await;
                match picked {
                    Some(keys) if !keys.is_empty() => Some(keys),
                    _ => return,
                }
            } else {
                None
            };
            let Some(p) = me.upgrade() else { return };
            p.ui.set_clock_in_text("處理中…".into());
            p.ui.set_clock_in_enabled(false);
            engine.clock_in(tasks).await;
            p.ui.set_clock_in_text("上班".into());
            p.ui.set_clock_in_enabled(true);
        });
    }

    fn clock_out(&self) {
        self.ui.set_clock_out_text("處理中…".into());
        self.ui.set_clock_out_enabled(false);
        let me = self.weak();
        let engine = self.engine.clone();
        spawn(async move {
            engine.clock_out().await;
            if let Some(p) = me.upgrade() {
                p.ui.set_clock_out_text("下班".into());
                p.ui.set_clock_out_enabled(true);
            }
        });
    }

    fn repo_pick(&self, index: usize) {
        let Some(vm) = self.s.borrow().repos.get(index).cloned() else { return };
        let me = self.weak();
        spawn(async move {
            let Some(p) = me.upgrade() else { return };
            let prompt = format!("「{}」這個 Claude session 在做哪個任務？", vm.name);
            if let Some(keys) = p.pick_tasks(&prompt, false).await
                && let Some(key) = keys.first()
            {
                p.engine.pick_task_for_repo(&vm.path, key).await;
            }
        });
    }

    /// C# `ShowDialogSafe(new TaskPickerWindow(…))`：開著選擇視窗時面板失去焦點也不收起，關掉後拉回面板。
    async fn pick_tasks(&self, prompt: &str, multi: bool) -> Option<Vec<String>> {
        self.s.borrow_mut().dialog_open = true;
        let tasks = self.engine.my_tasks();
        let picked = hb_ui_common::task_picker::pick(&tasks, prompt, multi).await;
        self.s.borrow_mut().dialog_open = false;
        if self.s.borrow().shown {
            hb_ui_common::window_util::activate(self.ui.window());
        }
        picked
    }
}

/// 面板底部的同步狀態（C# SyncText）。
pub fn sync_text(syncing: bool, auto_paused: bool, last_synced: Option<NaiveDateTime>) -> String {
    let hm = |t: NaiveDateTime| t.format("%H:%M").to_string();
    if syncing {
        "同步中…".to_owned()
    } else if auto_paused {
        format!("{} 同步 · 暫停自動", last_synced.map(hm).unwrap_or_default())
    } else if let Some(t) = last_synced {
        format!("{} 已同步", hm(t))
    } else {
        String::new()
    }
}

fn max_end(a: Option<DateTimeOffset>, b: Option<DateTimeOffset>) -> Option<DateTimeOffset> {
    // 計時中（沒有結束時間）算最大
    match (a, b) {
        (None, _) | (_, None) => None,
        (Some(x), Some(y)) => Some(x.max(y)),
    }
}

/// 排序用：None（計時中）最大。
fn end_key(t: Option<DateTimeOffset>) -> (bool, Option<DateTimeOffset>) {
    (t.is_none(), t)
}

/// C# `FilterStart` 的清單：釘選的任務排在最前面，打字時依編號、名稱、說明篩選，最多 12 筆；
/// 輸入兩個字以上再補遠端搜尋結果；輸入的編號不在清單裡時最前面加「直接開始這個任務」。
/// 回傳（清單, 本機符合的筆數）。
pub fn start_candidates(
    q: &str,
    pins: &[hb_api::PinItem],
    all_tasks: &[PickItem],
    remote: &[PickItem],
) -> (Vec<PickItem>, usize) {
    let pinned: Vec<PickItem> = pins
        .iter()
        .filter(|p| p.can_time)
        .map(|p| PickItem {
            key: p.key.clone(),
            title: p.title.clone(),
            detail: format!("📌 {} · {}", p.project, p.status),
        })
        .collect();
    let candidates: Vec<PickItem> = pinned
        .iter()
        .cloned()
        .chain(all_tasks.iter().filter(|t| !pinned.iter().any(|p| eq_ic(&p.key, &t.key))).cloned())
        .collect();
    let upper = q.to_uppercase();
    let contains = |s: &str| s.to_uppercase().contains(&upper);
    let mut items: Vec<PickItem> = if q.is_empty() {
        candidates.into_iter().take(12).collect()
    } else {
        candidates
            .into_iter()
            .filter(|t| contains(&t.key) || contains(&t.title) || contains(&t.detail))
            .take(12)
            .collect()
    };
    if q.chars().count() >= 2 {
        let room = 12usize.saturating_sub(items.len());
        let extra: Vec<PickItem> =
            remote.iter().filter(|r| !items.iter().any(|i| eq_ic(&i.key, &r.key))).take(room).cloned().collect();
        items.extend(extra);
    }
    let local = items.len();
    if let Some(typed) = KeyFinder::parse(Some(q))
        && !items.iter().any(|i| eq_ic(&i.key, &typed))
    {
        items.insert(
            0,
            PickItem {
                key: typed, title: "直接開始這個任務".to_owned(), detail: "不在我的任務清單裡".to_owned()
            },
        );
    }
    (items, local)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn pick(key: &str, title: &str) -> PickItem {
        PickItem { key: key.to_owned(), title: title.to_owned(), detail: "F2P · 進行中".to_owned() }
    }

    fn pin(key: &str, can_time: bool) -> hb_api::PinItem {
        hb_api::PinItem {
            key: key.to_owned(),
            title: format!("{key} 名稱"),
            project: "F2P".to_owned(),
            status: "進行中".to_owned(),
            today_minutes: 0,
            running: false,
            can_time,
            is_meeting: false,
            start_at: None,
            end_at: None,
            cancelled: false,
        }
    }

    #[test]
    fn 同步狀態文字() {
        let t = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap().and_hms_opt(9, 5, 0).unwrap();
        assert_eq!("同步中…", sync_text(true, true, Some(t)));
        assert_eq!("09:05 同步 · 暫停自動", sync_text(false, true, Some(t)));
        assert_eq!(" 同步 · 暫停自動", sync_text(false, true, None));
        assert_eq!("09:05 已同步", sync_text(false, false, Some(t)));
        assert_eq!("", sync_text(false, false, None));
    }

    #[test]
    fn 徽章最多99() {
        assert_eq!("", badge_text(0));
        assert_eq!("7", badge_text(7));
        assert_eq!("99", badge_text(99));
        assert_eq!("99+", badge_text(100));
    }

    #[test]
    fn 開始任務_釘選排最前_不重複() {
        let pins = [pin("F2P-2", true), pin("F2P-9", false)];
        let all = [pick("F2P-1", "登入頁"), pick("f2p-2", "重複的"), pick("F2P-3", "列印")];
        let (items, _) = start_candidates("", &pins, &all, &[]);
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
        // 不能計時的釘選不列；釘選的同一個任務不重複
        assert_eq!(vec!["F2P-2", "F2P-1", "F2P-3"], keys);
        assert_eq!("📌 F2P · 進行中", items[0].detail);
    }

    #[test]
    fn 開始任務_篩選不分大小寫_最多12筆() {
        let all: Vec<PickItem> = (1..=20).map(|i| pick(&format!("F2P-{i}"), "做事")).collect();
        let (items, local) = start_candidates("f2p", &[], &all, &[]);
        assert_eq!(12, items.len());
        assert_eq!(12, local);
        let (items, _) = start_candidates("列印", &[], &[pick("A-1", "登入"), pick("A-2", "列印頁")], &[]);
        assert_eq!(vec!["A-2"], items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn 開始任務_補遠端結果與直接開始() {
        let remote = [pick("EG-5", "遠端"), pick("A-2", "重複")];
        let (items, local) = start_candidates("EG-5", &[], &[pick("A-2", "EG-5 相關")], &remote);
        let keys: Vec<&str> = items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(vec!["A-2", "EG-5"], keys);
        assert_eq!(2, local);
        // 輸入的編號不在清單裡：最前面加「直接開始」
        let (items, _) = start_candidates("XY-12", &[], &[], &[]);
        assert_eq!("XY-12", items[0].key);
        assert_eq!("直接開始這個任務", items[0].title);
        assert_eq!("不在我的任務清單裡", items[0].detail);
        // 一個字不補遠端
        let (items, _) = start_candidates("E", &[], &[], &remote);
        assert!(items.is_empty());
    }

    #[test]
    fn 最後結束時間_計時中最大() {
        let t =
            |h| Some(chrono::FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 11, 12, h, 0, 0).unwrap());
        use chrono::TimeZone;
        assert_eq!(t(10), max_end(t(9), t(10)));
        assert_eq!(None, max_end(t(9), None));
        assert!(end_key(None) > end_key(t(23)));
    }
}
