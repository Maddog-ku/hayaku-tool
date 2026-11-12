//! 對應 C# `App.xaml.cs`：程式主體的接線。TimerEngine、面板、系統匣、工作列上的工時、滑鼠停留的摘要、提示、
//! 全域快捷鍵、Claude hook 接收埠、系統事件（喚醒、鎖定）、Slack、深淺色、建立任務小視窗的預先建立、命令列參數。

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Local, NaiveDate};
use hb_api::{ChangeRequest, HayakuApi, MeetingItem, PersonRef, WorkMath};
use hb_backend::auth::OAuthClient;
use hb_backend::config::{AppConfig, AppState};
use hb_backend::infra::{AppPaths, MiniHttpServer, MiniResponse, housekeeping, startup_registration};
use hb_backend::mcp::McpClient;
use hb_claude::ClaudeMonitor;
use hb_core::{NewWorkSuggestion, TestActionRequest, TimerEngine, ToastAction, ToastRequest};
use hb_slack::SlackBridge;
use hb_ui_common::QuickCreate;
use hb_ui_work::{PageArg, Workspace};
use hb_win::{
    GlobalHotkey, SessionSwitchReason, SystemEvents, TaskbarAnchor, TaskbarWidget, TrayActions, TrayIcon,
    WidgetContent, WidgetLayout, WidgetStatus,
};
use slint::ComponentHandle;

use crate::cli::{self, CliOptions};
use crate::hover_card::HoverCard;
use crate::panel::{Panel, spawn};
use crate::settings::Settings;
use crate::theme_manager::ThemeManager;
use crate::toast::ToastManager;

thread_local! {
    /// hook 接收埠（在 tokio 執行緒）轉回 UI 執行緒時找 App 用。
    static APP: RefCell<Weak<AppInner>> = const { RefCell::new(Weak::new()) };
}

pub struct AppInner {
    engine: TimerEngine,
    widget: TaskbarWidget,
    panel: Panel,
    tray: RefCell<Option<TrayIcon>>,
    hook_server: RefCell<Option<MiniHttpServer>>,
    settings: RefCell<Option<Settings>>,
    workspace: RefCell<Option<Workspace>>,
    hotkey: GlobalHotkey,
    hover_card: RefCell<Option<HoverCard>>,
    toasts: ToastManager,
    widget_tick: slint::Timer,
    hover_delay: slint::Timer,
    housekeeping: slint::Timer,
    system_events: RefCell<Option<SystemEvents>>,
    slack: RefCell<Option<SlackBridge>>,
    me: Weak<AppInner>,
}

/// 程式主體；main 持有到事件迴圈結束，結束時呼叫 [`App::exit`]。
pub struct App(Rc<AppInner>);

/// 在回呼裡拿 App（App 沒了就不做）。
macro_rules! with_app {
    ($me:expr, |$a:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = $me.clone();
        move |$($arg),*| {
            if let Some($a) = weak.upgrade() {
                $body
            }
        }
    }};
}

/// 非 tokio 的 sleep（UI 執行緒的 async 用；C# `Task.Delay`）。tokio 的 oneshot 不需要 tokio 執行環境也能 await。
pub fn sleep(d: Duration) -> impl std::future::Future<Output = ()> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    slint::Timer::single_shot(d, move || {
        let _ = tx.send(());
    });
    async move {
        let _ = rx.await;
    }
}

fn parse_layout(value: &str) -> WidgetLayout {
    WidgetLayout::parse(value).unwrap_or(WidgetLayout::TwoLine)
}

fn parse_anchor(value: &str) -> TaskbarAnchor {
    TaskbarAnchor::parse(value).unwrap_or(TaskbarAnchor::BeforeTray)
}

/// 工作列上的工時內容與提示文字（C# UpdateWidget）。
pub fn widget_content(engine: &TimerEngine) -> (WidgetContent, String) {
    if engine.initial_syncing() {
        let content = WidgetContent { total: "同步中".into(), status: WidgetStatus::Idle, ..Default::default() };
        return (content, "HayakuBar · 同步中，請稍候".into());
    }
    if !engine.logged_in() {
        let content = WidgetContent {
            total: if engine.logging_in() { "登入中" } else { "未登入" }.into(),
            status: if engine.error().is_none() { WidgetStatus::Idle } else { WidgetStatus::Error },
            ..Default::default()
        };
        return (content, "HayakuBar · 未登入".into());
    }
    let now = Local::now().naive_local();
    let day = now.date().and_hms_opt(0, 0, 0).unwrap_or(now);
    let worked = WorkMath::union_minutes(engine.entries().iter(), day, now);
    let expected = engine.schedule().map_or(0, |s| s.expected_minutes) as f64;
    let running = engine.running_keys().len() as i32;
    let content = WidgetContent {
        total: WorkMath::hours_text(worked),
        expected: if expected > 0.0 { format!("/ {}", WorkMath::hours_text(expected)) } else { String::new() },
        progress: if expected > 0.0 { worked / expected } else { 0.0 },
        running,
        claude: engine.claude().repos().any(|r| r.active),
        status: if engine.error().is_some() {
            WidgetStatus::Error
        } else if running > 0 {
            WidgetStatus::Running
        } else {
            WidgetStatus::Idle
        },
        unread: engine.unread_count() as i32,
    };
    let tooltip = format!(
        "HayakuBar · 今天 {} / {} · 計時中 {running}",
        WorkMath::hours_text(worked),
        WorkMath::hours_text(expected)
    );
    (content, tooltip)
}

impl App {
    /// C# OnStartup（--mcp-call 與單一執行個體在 main 處理）。
    pub fn start(options: CliOptions) -> Option<App> {
        let config = Rc::new(RefCell::new(AppConfig::load()));
        let state = Rc::new(RefCell::new(AppState::load()));
        ThemeManager::initialize(&config.borrow().theme_mode);
        let base_url = config.borrow().base_url.clone();
        let http = hb_backend::http::app_client();
        let auth = OAuthClient::new(http.clone(), &base_url);
        let api = Arc::new(HayakuApi::new(Arc::new(McpClient::new(http, auth.clone(), &base_url))));
        let engine = TimerEngine::new(config.clone(), state, auth, api, ClaudeMonitor::new());

        let panel = Panel::new(engine.clone())?;
        let (anchor, offset, layout) = {
            let c = config.borrow();
            (parse_anchor(&c.taskbar_anchor), c.taskbar_offset, parse_layout(&c.taskbar_layout))
        };
        let inner = Rc::new_cyclic(|me| AppInner {
            engine: engine.clone(),
            widget: TaskbarWidget::new(anchor, offset, layout),
            panel,
            tray: RefCell::new(None),
            hook_server: RefCell::new(None),
            settings: RefCell::new(None),
            workspace: RefCell::new(None),
            hotkey: GlobalHotkey::new(),
            hover_card: RefCell::new(None),
            toasts: ToastManager::new(),
            widget_tick: slint::Timer::default(),
            hover_delay: slint::Timer::default(),
            housekeeping: slint::Timer::default(),
            system_events: RefCell::new(None),
            slack: RefCell::new(None),
            me: me.clone(),
        });
        APP.with(|a| *a.borrow_mut() = Rc::downgrade(&inner));
        inner.wire(&options);
        Some(App(inner))
    }

    /// C# OnExit。
    pub fn exit(self) {
        let a = &self.0;
        a.system_events.borrow_mut().take();
        a.widget_tick.stop();
        a.housekeeping.stop();
        a.hotkey.register(None);
        if let Some(mut server) = a.hook_server.borrow_mut().take() {
            server.dispose();
        }
        if let Some(slack) = a.slack.borrow_mut().take() {
            slack.dispose();
        }
        a.tray.borrow_mut().take();
        hb_backend::mcp::mcp_stats::flush();
    }
}

impl AppInner {
    fn wire(&self, options: &CliOptions) {
        let engine = &self.engine;
        let me = self.me.clone();
        engine.set_ui_visible(move || {
            me.upgrade()
                .is_some_and(|a| a.panel.is_open() || a.workspace.borrow().as_ref().is_some_and(Workspace::is_visible))
        });
        self.panel.on_settings_requested(with_app!(self.me, |a| a.open_settings(None)));
        self.panel.on_time_entries_requested(with_app!(self.me, |a| a.open_time_entries(Local::now().date_naive())));
        let me = self.me.clone();
        self.panel.on_workspace_requested(move |page, arg| {
            if let Some(a) = me.upgrade() {
                a.open_workspace(page, arg);
            }
        });
        let events = engine.events();
        events.time_entries_requested.subscribe(with_app!(self.me, |a, day| a.open_time_entries(*day)));
        events
            .workspace_requested
            .subscribe(with_app!(self.me, |a, req| a.open_workspace(&req.0, req.1.clone().map(PageArg::from))));
        events.create_task_requested.subscribe(with_app!(self.me, |a, s| a.open_quick_create(s)));
        events.new_work_ready.subscribe(with_app!(self.me, |a, s| a.prewarm_quick_create(s.clone())));
        events.pick_task_requested.subscribe(with_app!(self.me, |a, s| a.pick_task_for_session(s.clone())));
        events
            .reject_requested
            .subscribe(with_app!(self.me, |a, r| hb_ui_common::reject_review::open(&a.engine, r, false)));
        events.test_action_requested.subscribe(with_app!(self.me, |a, r| a.open_test_action(r)));

        self.create_tray();
        let toasts = self.toasts.clone();
        events.notify.subscribe(move |m| toasts.show(ToastRequest::new("HayakuBar", m.clone(), vec![])));
        let toasts = self.toasts.clone();
        events.toast.subscribe(move |r| toasts.show(r.clone()));
        let toasts = self.toasts.clone();
        events.toast_dismiss_requested.subscribe(move |tag| toasts.dismiss(tag));
        let slack = SlackBridge::new(engine.clone());
        engine.set_slack(Some(Rc::new(slack.clone())));
        slack.restart();
        *self.slack.borrow_mut() = Some(slack);
        let me = self.me.clone();
        self.toasts
            .set_anchor(move || me.upgrade().map_or((crate::popup::primary_work_area(), false), |a| a.widget_anchor()));
        let config = engine.config_rc();
        self.toasts.set_is_quiet(move || config.borrow().quiet_when_busy && hb_win::user_busy());
        let config = engine.config_rc();
        self.toasts.set_default_duration(move || {
            let seconds = config.borrow().notification_popup_seconds;
            (seconds > 0).then(|| Duration::from_secs(seconds as u64))
        });

        let widget = &self.widget;
        widget.on_clicked(with_app!(self.me, |a| a.toggle_panel()));
        widget.on_menu_requested(with_app!(self.me, |a, p| {
            if let Some(tray) = a.tray.borrow().as_ref() {
                tray.show_menu(p);
            }
        }));
        widget.on_hover_changed(with_app!(self.me, |a, hovering| a.on_widget_hover(hovering)));
        widget.on_offset_committed(with_app!(self.me, |a, offset| {
            a.engine.config_mut().taskbar_offset = offset;
            a.engine.config().save();
        }));
        widget.start();

        events.changed.subscribe(with_app!(self.me, |a, _x| a.update_widget()));
        events.config_applied.subscribe(with_app!(self.me, |a, _x| {
            let (anchor, layout, offset) = {
                let c = a.engine.config();
                (parse_anchor(&c.taskbar_anchor), parse_layout(&c.taskbar_layout), c.taskbar_offset)
            };
            a.widget.set_anchor(anchor);
            a.widget.set_layout(layout);
            a.widget.set_offset(offset);
            a.widget.relayout();
            a.register_hotkey(true);
        }));
        self.widget_tick.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(1),
            with_app!(self.me, |a| a.update_widget()),
        );
        self.update_widget();

        self.hotkey.on_pressed(with_app!(self.me, |a| a.toggle_panel()));
        self.register_hotkey(false);

        self.start_hook_server(engine.config().hook_port);
        let system = SystemEvents::new();
        system.on_resume(with_app!(self.me, |a| {
            let e = a.engine.clone();
            spawn(async move { e.auto_refresh(true).await });
        }));
        system.on_session_switch(with_app!(self.me, |a, reason| a.on_session_switch(reason)));
        // C# ThemeManager 訂閱 UserPreferenceChanged：跟隨 Windows 時換深淺色
        system.on_apps_dark_changed(|_dark| ThemeManager::system_changed());
        system.on_client_area_animation_changed(ThemeManager::animation_changed);
        *self.system_events.borrow_mut() = Some(system);

        self.handle_cli(options);

        let me = self.me.clone();
        let engine = engine.clone();
        spawn(async move {
            engine.start().await;
            // 閒置時先把面板的畫面建好（C# Dispatcher ApplicationIdle）
            slint::Timer::single_shot(Duration::from_millis(500), with_app!(me, |a| a.panel.prewarm()));
            let Some(a) = me.upgrade() else { return };
            // 啟動 3 分鐘後第一次，之後每 30 分鐘
            let me2 = me.clone();
            a.housekeeping.start(slint::TimerMode::SingleShot, Duration::from_secs(180), move || {
                let Some(a) = me2.upgrade() else { return };
                a.housekeep("定期");
                let me3 = me2.clone();
                a.housekeeping.start(slint::TimerMode::Repeated, Duration::from_secs(30 * 60), move || {
                    if let Some(a) = me3.upgrade() {
                        a.housekeep("定期");
                    }
                });
            });
            log::info!("HayakuBar 啟動");
        });
    }

    fn create_tray(&self) {
        let config = self.engine.config_rc();
        let me = self.me.clone();
        let actions = TrayActions {
            toggle_panel: Box::new(with_app!(me, |a| a.toggle_panel())),
            refresh: Box::new(with_app!(me, |a| {
                let e = a.engine.clone();
                spawn(async move { e.refresh(true).await });
            })),
            settings: Box::new(with_app!(me, |a| a.open_settings(None))),
            login_or_logout: Box::new(with_app!(me, |a| {
                let e = a.engine.clone();
                spawn(async move {
                    if e.logged_in() {
                        e.logout().await;
                    } else {
                        e.login().await;
                    }
                });
            })),
            is_logged_in: Box::new({
                let me = me.clone();
                move || me.upgrade().is_some_and(|a| a.engine.logged_in())
            }),
            get_anchor: Box::new({
                let config = config.clone();
                move || parse_anchor(&config.borrow().taskbar_anchor)
            }),
            set_anchor: Box::new(with_app!(me, |a, anchor| a.set_anchor(anchor))),
            reset_offset: Box::new(with_app!(me, |a| a.set_offset(0.0))),
            get_layout: Box::new({
                let config = config.clone();
                move || parse_layout(&config.borrow().taskbar_layout)
            }),
            set_layout: Box::new(with_app!(me, |a, layout| a.set_layout(layout))),
            time_entries: Box::new(with_app!(me, |a| a.open_time_entries(Local::now().date_naive()))),
            workspace: Box::new(with_app!(me, |a| a.open_workspace("tasks", None))),
            open_web: Box::new({
                let config = config.clone();
                move || hb_win::open(&config.borrow().base_url.clone())
            }),
            open_log: Box::new(|| hb_win::open(&AppPaths::dir().display().to_string())),
            exit: Box::new(|| {
                let _ = slint::quit_event_loop();
            }),
            is_startup_enabled: Box::new(startup_registration::is_enabled),
            set_startup: Box::new(|enabled| {
                if let Err(e) = startup_registration::set(enabled) {
                    log::error!("設定開機啟動失敗: {e}");
                }
            }),
        };
        *self.tray.borrow_mut() = Some(TrayIcon::new(actions));
    }

    // ───────────── 工作列與面板 ─────────────

    fn update_widget(&self) {
        let (content, tooltip) = widget_content(&self.engine);
        self.widget.update(content);
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.set_tooltip(&tooltip);
        }
    }

    fn toggle_panel(&self) {
        if self.panel.is_open() {
            self.panel.hide_panel();
            return;
        }
        // 點工作列時面板先因失去焦點收起，同一下點擊不要又把它打開
        if self.panel.hidden_at().is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
            return;
        }
        self.hover_delay.stop();
        if let Some(h) = self.hover_card.borrow().as_ref() {
            h.hide();
        }
        let (anchor, align_left) = self.widget_anchor();
        self.panel.show_at(anchor, align_left);
    }

    /// 工作列上工時的位置（實體像素；C# 是 DIP）；面板、摘要、提示都從它正上方出現。
    fn widget_anchor(&self) -> (hb_win::Rect, bool) {
        if self.widget.is_embedded() {
            let r = self.widget.screen_rect();
            let tb = self.widget.taskbar_rect();
            let anchor = hb_win::Rect { left: r.left, top: tb.top, right: r.right, bottom: tb.bottom };
            return (anchor, (r.left + r.right) / 2 < (tb.left + tb.right) / 2);
        }
        let work = crate::popup::primary_work_area();
        let x = work.right - (12.0 * crate::popup::scale_at(work.right - 1, work.bottom - 1)).round() as i32;
        (hb_win::Rect { left: x, top: work.bottom, right: x, bottom: work.bottom }, false)
    }

    fn on_widget_hover(&self, hovering: bool) {
        if hovering {
            self.hover_delay.start(
                slint::TimerMode::SingleShot,
                Duration::from_millis(450),
                with_app!(self.me, |a| a.show_hover_card()),
            );
            return;
        }
        self.hover_delay.stop();
        if let Some(h) = self.hover_card.borrow().as_ref() {
            h.hide();
        }
    }

    fn show_hover_card(&self) {
        if !self.engine.logged_in() || self.panel.is_open() {
            return;
        }
        self.show_hover_card_now();
    }

    fn show_hover_card_now(&self) {
        if self.hover_card.borrow().is_none() {
            *self.hover_card.borrow_mut() = HoverCard::new();
        }
        let (anchor, align_left) = self.widget_anchor();
        if let Some(h) = self.hover_card.borrow().as_ref() {
            h.show_for(&self.engine, anchor, align_left);
        }
    }

    fn register_hotkey(&self, show_error: bool) {
        let text = self.engine.config().hotkey.clone();
        if !self.hotkey.register(Some(&text)) && show_error {
            self.toasts.show(ToastRequest::new(
                "快捷鍵無法使用",
                format!("「{text}」可能被其他程式占用，請在設定換一組。"),
                vec![],
            ));
        }
    }

    /// 工時紀錄在工作台裡：開工作台並切到那一天。
    fn open_time_entries(&self, day: NaiveDate) {
        self.open_workspace("entries", Some(PageArg::Date(day)));
    }

    /// 開工作台到指定頁（tasks、hours、entries、worklog、reminders、meetings、chat、team、leaves、stats…）。
    fn open_workspace(&self, page: &str, arg: Option<PageArg>) {
        if page == "feedback"
            && let Some(PageArg::Number(number)) = arg
        {
            // 問題回報不在工作台裡，開獨立視窗（同一則只開一個）
            hb_ui_common::feedback::open(&self.engine, number);
            return;
        }
        if self.workspace.borrow().is_none() {
            let ws = match Workspace::new(self.engine.clone()) {
                Ok(ws) => ws,
                Err(e) => {
                    log::error!("建立工作台失敗: {e}");
                    return;
                }
            };
            let me = self.me.clone();
            ws.window().window().on_close_requested(move || {
                // C# Closed → _workspace = null（延到下一輪再丟，不在自己的回呼裡丟掉自己）
                let me = me.clone();
                slint::Timer::single_shot(Duration::ZERO, move || {
                    if let Some(a) = me.upgrade() {
                        a.workspace.borrow_mut().take();
                    }
                });
                slint::CloseRequestResponse::HideWindow
            });
            // 工作台跟著深淺色、「顯示動畫」切換（C# DynamicResource）
            let weak = ws.window().as_weak();
            ThemeManager::track(move |dark, reduce_motion| match weak.upgrade() {
                Some(w) => {
                    let theme = w.global::<hb_ui_work::Theme>();
                    theme.set_dark(dark);
                    theme.set_reduce_motion(reduce_motion);
                    crate::theme_manager::set_title_bar(w.window(), dark);
                    true
                }
                None => false,
            });
            *self.workspace.borrow_mut() = Some(ws);
        }
        let Some(ws) = self.workspace.borrow().clone() else { return };
        let window = ws.window().window();
        if window.is_minimized() {
            window.set_minimized(false);
        }
        ws.show_page(page, arg);
        if let Err(e) = ws.present() {
            log::error!("顯示工作台失敗: {e}");
            return;
        }
        crate::theme_manager::set_title_bar(window, ThemeManager::is_dark());
        hb_ui_common::window_util::activate(window);
    }

    /// 偵測到沒有任務的新工作，使用者按了「建立任務」：開建立任務的小視窗（同一個 session 只開一個）。
    fn open_quick_create(&self, suggestion: &NewWorkSuggestion) {
        let existing = QuickCreate::find(&suggestion.session_id);
        if let Some(q) = &existing
            && q.presented()
        {
            q.activate();
            return;
        }
        // 提示跳出時多半已經預先建好（prewarm_quick_create），直接顯示
        let Some(q) = existing.or_else(|| self.new_quick_create(suggestion)) else { return };
        let (anchor, align_left) = self.widget_anchor();
        q.present(anchor, align_left);
    }

    fn new_quick_create(&self, suggestion: &NewWorkSuggestion) -> Option<QuickCreate> {
        let q = QuickCreate::new(&self.engine, suggestion)?;
        let toasts = self.toasts.clone();
        let unit = suggestion.unit_name.clone();
        q.on_closed(move |r| {
            if let Some(key) = &r.created_key {
                let message = if r.assigned {
                    format!("{}\n已指定給 {unit} 的 Claude session，會自動計時", r.created_title)
                } else {
                    r.created_title.clone()
                };
                toasts.show(ToastRequest::new(format!("已建立 {key}"), message, vec![]));
            }
        });
        Some(q)
    }

    /// 「有新工作」的提示跳出、資料也讀好了：先把建立任務視窗建好、畫好，按「建立任務」時移到定位馬上出現。
    /// 提示最多留 10 分鐘，沒按的視窗 12 分鐘後收掉。
    fn prewarm_quick_create(&self, suggestion: NewWorkSuggestion) {
        let me = self.me.clone();
        // 等提示滑出來的動畫跑完：建畫面會占住畫面執行緒，跟動畫擠在一起會頓
        slint::Timer::single_shot(Duration::from_millis(800), move || {
            let Some(a) = me.upgrade() else { return };
            if QuickCreate::find(&suggestion.session_id).is_some() {
                return;
            }
            let Some(q) = a.new_quick_create(&suggestion) else {
                log::error!("預先建立任務視窗失敗");
                return;
            };
            q.prewarm();
            let session = suggestion.session_id.clone();
            slint::Timer::single_shot(Duration::from_secs(12 * 60), move || {
                if let Some(q) = QuickCreate::find(&session)
                    && !q.presented()
                {
                    q.close();
                }
            });
        });
    }

    /// 從提示、面板按了測試結果的「開始修」「退回」：開小視窗（同一筆只開一個）；add 是在測試任務加入測試結果。
    fn open_test_action(&self, request: &TestActionRequest) {
        if request.action == "add" {
            hb_ui_common::add_test_result::open(&self.engine, &request.key, &request.title, None, &[], None);
            return;
        }
        let engine = self.engine.clone();
        hb_ui_common::test_action::open(
            &self.engine,
            request,
            Some(Rc::new(move |message: &str| engine.show_notice(message))),
        );
    }

    /// 使用者按了「選既有任務」：選一個任務指定給那個 Claude session。
    fn pick_task_for_session(&self, suggestion: NewWorkSuggestion) {
        let engine = self.engine.clone();
        spawn(async move {
            let prompt = format!("「{}」這個 Claude session 在做哪個任務？", suggestion.unit_name);
            let picked = hb_ui_common::task_picker::pick(&engine.my_tasks(), &prompt, false).await;
            if let Some(key) = picked.as_ref().and_then(|k| k.first()) {
                // 資料夾對應的專案會自動記下
                engine.assign_session_task(&suggestion.session_id, key, true).await;
            }
        });
    }

    fn set_anchor(&self, anchor: TaskbarAnchor) {
        {
            let mut c = self.engine.config_mut();
            c.taskbar_anchor = anchor.as_str().to_owned();
            c.taskbar_offset = 0.0;
        }
        self.engine.config().save();
        self.widget.set_anchor(anchor);
        self.widget.set_offset(0.0);
        self.widget.relayout();
    }

    fn set_offset(&self, offset: f64) {
        self.engine.config_mut().taskbar_offset = offset;
        self.engine.config().save();
        self.widget.set_offset(offset);
        self.widget.relayout();
    }

    fn set_layout(&self, layout: WidgetLayout) {
        self.engine.config_mut().taskbar_layout = layout.as_str().to_owned();
        self.engine.config().save();
        self.widget.set_layout(layout);
        self.widget.relayout();
    }

    // ───────────── 其他 ─────────────

    fn start_hook_server(&self, port: i32) {
        let mut server = MiniHttpServer::new(port as u16, |req| async move {
            if req.path == "/health" {
                return Ok(MiniResponse::new(200, "text/plain", "ok"));
            }
            if req.method != "POST" || req.path != "/hook" {
                return Ok(MiniResponse::new(404, "text/plain", "not found"));
            }
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&req.body) else {
                return Ok(MiniResponse::new(400, "text/plain", "bad json"));
            };
            // 轉回 UI 執行緒處理，處理完才回應（C# await Dispatcher.InvokeAsync）
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            let posted = slint::invoke_from_event_loop(move || {
                APP.with(|a| {
                    if let Some(app) = a.borrow().upgrade() {
                        app.engine.on_hook(&payload);
                    }
                });
                let _ = tx.send(());
            });
            if posted.is_ok() {
                let _ = rx.await;
            }
            Ok(MiniResponse::new(200, "application/json", "{}"))
        });
        match server.start() {
            Ok(()) => *self.hook_server.borrow_mut() = Some(server),
            Err(e) => log::error!("Claude hook 接收埠 {port} 無法啟動: {e}"),
        }
    }

    fn open_settings(&self, page: Option<&str>) {
        if self.settings.borrow().is_none() {
            let Some(settings) = Settings::new(self.engine.clone()) else { return };
            let me = self.me.clone();
            settings.on_closed(move || {
                let me = me.clone();
                slint::Timer::single_shot(Duration::ZERO, move || {
                    if let Some(a) = me.upgrade() {
                        a.settings.borrow_mut().take();
                    }
                });
            });
            settings.show();
            *self.settings.borrow_mut() = Some(settings);
        }
        let Some(settings) = self.settings.borrow().clone() else { return };
        if let Some(page) = page {
            settings.show_page(page);
        }
        settings.activate();
    }

    fn on_session_switch(&self, reason: SessionSwitchReason) {
        // 鎖定中（離開座位）一般通知也傳到 Slack
        if let Some(slack) = self.engine.slack() {
            slack.set_locked(reason == SessionSwitchReason::SessionLock);
        }
        match reason {
            SessionSwitchReason::SessionLock => {
                slint::Timer::single_shot(Duration::from_millis(200), with_app!(self.me, |a| a.housekeep("鎖定電腦")));
            }
            SessionSwitchReason::SessionUnlock | SessionSwitchReason::SessionLogon => {
                let e = self.engine.clone();
                spawn(async move { e.auto_refresh(true).await });
            }
        }
    }

    /// 清暫存：舊截圖與紀錄、各處快取，再整理記憶體。
    /// 面板、工作台、設定開著（正在用）時只清檔案與快取，不整理記憶體，避免操作時卡一下。
    fn housekeep(&self, reason: &str) {
        let before = housekeeping::memory_text();
        let deleted = housekeeping::clean_files(&AppPaths::dir(), Local::now().naive_local());
        self.engine.trim_caches();
        self.panel.trim_caches();
        // 提示、滑鼠停留的摘要、預先建好還沒顯示的建立任務視窗不算；面板、工作台、設定、建立任務…開著就算在用
        let open = open_window_titles();
        if open.is_empty() {
            housekeeping::trim_memory();
        }
        let after = if open.is_empty() {
            housekeeping::memory_text()
        } else {
            format!("{} 開著，略過整理記憶體", open.join("、"))
        };
        log::info!("清暫存（{reason}）：刪除 {deleted} 個舊檔；{before} → {after}");
    }

    // ───────────── 命令列參數 ─────────────

    fn handle_cli(&self, options: &CliOptions) {
        // --settings 或 --settings:claude 之類：啟動後直接開設定頁
        if let Some(page) = &options.settings {
            self.open_settings(page.as_deref());
        }
        // --toast-test：顯示一則範例提示（檢查外觀用）
        if options.toast_test {
            self.toasts.show(
                ToastRequest::new(
                    "會議 14:00・3 分鐘後開始",
                    "HAYAKU專案管理系統介紹會議-3",
                    vec![ToastAction::new("加入並計時", || async {}).primary(), ToastAction::new("開啟", || async {})],
                )
                .sticky(true),
            );
        }
        // --meeting-test：模擬兩場快開始的會議（我是參加者／不是參加者），檢查會議提示用；按鈕不會真的加入
        if options.meeting_test {
            let now = Local::now().fixed_offset();
            let m = |key: &str, id: &str, title: &str, project: &str, start: i64, end: i64, status: &str| MeetingItem {
                key: key.into(),
                id: id.into(),
                title: title.into(),
                project: project.into(),
                start_at: now + chrono::TimeDelta::minutes(start),
                end_at: now + chrono::TimeDelta::minutes(end),
                location: None,
                my_status: Some(status.into()),
            };
            self.toasts.show(self.engine.meeting_toast(
                &m("F2P-0", "demo-1", "HayakuBar 會議提醒測試", "F2P", 3, 30, "還沒加入"),
                now,
                true,
            ));
            self.toasts.show(self.engine.meeting_toast(
                &m("EG-0", "demo-2", "長榮工時估計會議（測試）", "EG", 5, 60, "不是參加者"),
                now,
                true,
            ));
        }
        // --entries：啟動後直接開工時紀錄
        if options.entries {
            slint::Timer::single_shot(
                Duration::from_millis(300),
                with_app!(self.me, |a| a.open_time_entries(Local::now().date_naive())),
            );
        }
        // --workspace 或 --workspace:hours 之類：啟動後直接開工作台
        if let Some(page) = options.workspace.clone() {
            slint::Timer::single_shot(
                Duration::from_millis(300),
                with_app!(self.me, |a| a.open_workspace(&page, None)),
            );
        }
        // --shot:tasks,hours,panel-today…：開啟各頁後把畫面存成 PNG（檢查外觀用，只拍這個程式自己的視窗）
        if let Some(pages) = options.shot.clone() {
            let me = self.me.clone();
            spawn(async move { shoot(me, pages).await });
        }
        // --panel：啟動後直接打開面板
        if let Some(tab) = options.panel {
            self.panel.set_stay_open(true);
            if let Some(tab) = tab {
                self.panel.select_tab(tab);
            }
            slint::Timer::single_shot(Duration::from_millis(300), with_app!(self.me, |a| a.toggle_panel()));
        }
    }
}

/// 開著（看得到、會搶焦點）的這個程式的視窗標題；提示、滑鼠停留的摘要（NOACTIVATE）不算。
fn open_window_titles() -> Vec<String> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GWL_EXSTYLE, GetWindowLongPtrW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
        WS_EX_NOACTIVATE,
    };
    use windows::core::BOOL;
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let list = &mut *(lparam.0 as *mut Vec<String>);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid != std::process::id() || !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_NOACTIVATE.0 as isize != 0 {
                return BOOL(1);
            }
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(hwnd, &mut buf);
            if n > 0 {
                list.push(String::from_utf16_lossy(&buf[..n as usize]));
            }
            BOOL(1)
        }
    }
    let mut list: Vec<String> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut list as *mut Vec<String> as isize));
    }
    list
}

/// C# `ShootAsync`：開啟各頁後把畫面存成 PNG（AppPaths.Dir\shots\頁.png）。
async fn shoot(me: Weak<AppInner>, pages: String) {
    let dir = AppPaths::dir().join("shots");
    let _ = std::fs::create_dir_all(&dir);
    loop {
        match me.upgrade() {
            Some(a) if !a.engine.initial_syncing() => break,
            Some(_) => sleep(Duration::from_millis(500)).await,
            None => return,
        }
    }
    sleep(Duration::from_millis(1500)).await;
    for page in cli::shot_pages(&pages) {
        let Some(a) = me.upgrade() else { return };
        let target = shoot_page(&a, &page).await;
        drop(a);
        sleep(Duration::from_millis(4000)).await;
        // 提示可能已經自己收掉了（停留秒數比較短）：拍那時還開著的最後一則
        let target = match (page.as_str(), me.upgrade()) {
            ("toast", Some(a)) => a
                .toasts
                .open_toasts()
                .last()
                .and_then(|t| crate::popup::hwnd(t.window()))
                .map(crate::shot::Target::Window),
            _ => target,
        };
        let Some(target) = target else { continue };
        let path = dir.join(format!("{}.png", page.replace(':', "_")));
        if let Err(e) = crate::shot::capture(&target, &path) {
            log::error!("截圖 {page} 失敗: {e}");
        }
    }
    log::info!("截圖完成：{}", dir.display());
}

/// 打開要拍的頁，回傳要拍哪裡。
async fn shoot_page(a: &Rc<AppInner>, page: &str) -> Option<crate::shot::Target> {
    use crate::shot::Target;
    let engine = a.engine.clone();
    match page {
        "review" => {
            // 範例：有新的申請等我審核（可以直接核准、駁回），以及按「駁回」開的小視窗；按鈕都不會真的送出
            let demo = ChangeRequest {
                id: "demo".into(),
                project: "F2P".into(),
                r#type: "CREATE".into(),
                status: "PENDING".into(),
                task_key: Some("F2P-128".into()),
                task_title: "面板新增全部列印及選擇列印按鈕".into(),
                requester: Some(PersonRef::new("demo", "丁鈺昕")),
                reviewer: None,
                created_at: Local::now().fixed_offset(),
                changes: vec!["預計結束：2026-11-05".into(), "預計工時：4 小時".into()],
                reason: Some("客戶要求下週上線".into()),
                review_note: None,
                logged_minutes: 95,
                descendant_count: 0,
            };
            a.toasts.show(engine.review_toast(&demo, true));
            hb_ui_common::reject_review::open(&engine, &demo, true);
            sleep(Duration::from_millis(1500)).await;
            Some(Target::Foreground)
        }
        "toast" => {
            // 範例：新通知的彈窗（從下面滑上來）
            a.toasts.show(
                ToastRequest::new(
                    "申請已審核",
                    "丁鈺昕：F2P-101 面板新增全部列印及選擇列印按鈕",
                    vec![ToastAction::new("標為已讀", || async {}).primary(), ToastAction::new("查看", || async {})],
                )
                .duration(Some(Duration::from_secs(30)))
                .from_bottom(),
            );
            a.toasts.show(ToastRequest::new("HayakuBar", "休息時間，已暫停 7 個計時，13:00 會自動恢復", vec![]));
            sleep(Duration::from_millis(1500)).await;
            let toast = a.toasts.open_toasts().last().cloned()?;
            if let Some(r) = crate::popup::outer_rect(toast.window()) {
                log::info!(
                    "截圖：通知彈窗位置 {},{} 寬 {}；工時位置 {:?}",
                    r.left,
                    r.top,
                    r.width(),
                    a.widget_anchor()
                );
            }
            Some(Target::Window(crate::popup::hwnd(toast.window())?))
        }
        "quickcreate" => {
            // 範例：偵測到新工作後的「建立任務」小視窗（用第一個監看資料夾）
            let unit = engine.claude().repos().max_by_key(|r| r.last_activity).cloned();
            let suggestion = NewWorkSuggestion {
                unit_path: unit.as_ref().map_or_else(|| AppPaths::dir().display().to_string(), |u| u.path.clone()),
                unit_name: unit.as_ref().map_or_else(|| "HayakuBar".to_owned(), |u| u.name().to_owned()),
                session_id: format!("shot-{:x}", Local::now().timestamp_micros()),
                title: Some(
                    unit.as_ref()
                        .and_then(|u| u.first_prompt.clone())
                        .unwrap_or_else(|| "列印頁面加上全部列印按鈕".to_owned()),
                ),
                project: None,
                parent: None,
                session_file: None,
            };
            engine.prefetch_new_work(&suggestion).await; // 跟實際一樣：提示跳出時就先讀好、預先建好視窗
            sleep(Duration::from_millis(1500)).await;
            a.open_quick_create(&suggestion);
            Some(Target::Foreground)
        }
        "testfix" | "testadd" => {
            // 範例：測試結果的「開始修」小視窗、測試任務的「加入測試結果」小視窗（不按按鈕就不會送出）
            if page == "testfix" {
                let request = TestActionRequest {
                    action: "fix".into(),
                    key: "F2P-140".into(),
                    title: "登入頁跑版（範例）".into(),
                };
                hb_ui_common::test_action::open(&engine, &request, None);
            } else {
                hb_ui_common::add_test_result::open(
                    &engine,
                    "F2P-139",
                    "第二階段測試（範例）",
                    Some("F2P-25"),
                    &[],
                    None,
                );
            }
            sleep(Duration::from_millis(1500)).await;
            Some(Target::Foreground)
        }
        // Rust 版多的：滑鼠停留的摘要（沒登入也顯示，檢查外觀用）、工作列上的工時
        "hover" => {
            a.show_hover_card_now();
            sleep(Duration::from_millis(800)).await;
            let h = a.hover_card.borrow().as_ref().and_then(|h| crate::popup::hwnd(h.window()))?;
            Some(Target::Window(h))
        }
        "widget" => {
            let r = a.widget.screen_rect();
            let tb = a.widget.taskbar_rect();
            Some(Target::Screen(hb_win::Rect { left: r.left - 8, top: tb.top, right: r.right + 8, bottom: tb.bottom }))
        }
        _ if page.len() > 9 && page[..9].eq_ignore_ascii_case("settings-") => {
            a.open_settings(Some(&page[9..]));
            sleep(Duration::from_millis(500)).await;
            let settings = a.settings.borrow().clone()?;
            Some(Target::Window(crate::popup::hwnd(settings.window())?))
        }
        _ if page.len() >= 5 && page[..5].eq_ignore_ascii_case("panel") => {
            a.panel.set_stay_open(true);
            a.panel.select_tab(&cli::shot_panel_tab(page));
            if !a.panel.is_open() {
                a.toggle_panel();
            }
            sleep(Duration::from_millis(500)).await;
            Some(Target::Window(crate::popup::hwnd(a.panel.window())?))
        }
        _ => {
            a.panel.hide_panel();
            // tasks=F2P-97、meetings=new：等號後面是要帶給那一頁的參數
            let (p, arg) = cli::split_page_arg(page);
            a.open_workspace(p, arg.map(|s| PageArg::Text(s.to_owned())));
            sleep(Duration::from_millis(500)).await;
            let ws = a.workspace.borrow().clone()?;
            Some(Target::Window(crate::popup::hwnd(ws.window().window())?))
        }
    }
}
