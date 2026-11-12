//! 對應 C# `UI/SettingsWindow.xaml.cs`：設定頁，左側分頁、右側卡片，改了立刻生效。
//! Slack 頁在 [`slack`]（C# SettingsWindow.Slack.cs）。最後多一頁「關於」放 Slint 的 AboutSlint（Slint 免版稅授權要求）。

mod slack;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use chrono::{Local, NaiveDateTime};
use hb_api::{DaySchedule, WorkMath};
use hb_backend::config::{ProjectConfig, RepoConfig};
use hb_backend::infra::AppPaths;
use hb_backend::infra::startup_registration;
use hb_claude::{ClaudeMonitor, DetectedProject, HookInstaller, RepoActivity};
use hb_core::ci_set::eq_ic;
use hb_core::{HandlerId, TimerEngine};
use hb_eip::{EipClient, EipCredentialStore, EipCredentials};
use hb_ui_common::view_models::ColorKey;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::dialogs::{self, Level};
use crate::menu::{self, MenuEntry, MenuPlace};
use crate::panel::spawn;
use crate::theme_manager::ThemeManager;
use crate::ui::{ProjectRow, RepoCard, SettingsWindow};

/// 分頁（C# NavPanel 的 Tag 去掉 Page；about 是 Rust 版多的「關於」）。
pub const PAGES: [&str; 9] = ["account", "taskbar", "schedule", "repos", "claude", "eip", "slack", "general", "about"];
const ANCHORS: [&str; 4] = ["Left", "BeforeStart", "AfterIcons", "BeforeTray"];
const POLL_SECONDS: [i32; 4] = [15, 30, 60, 120];
const PANEL_HEIGHTS: [i32; 5] = [360, 440, 520, 600, 0];
const MEETING_LEADS: [i32; 6] = [0, 1, 3, 5, 10, 15];
const POPUP_SECONDS: [i32; 5] = [3, 5, 8, 15, 0];
const THEMES: [&str; 3] = ["System", "Light", "Dark"];

/// C# `new[] { … }.MinBy(v => Math.Abs(v - value))`：最接近的那一個的索引（一樣近取前面的）。
pub fn nearest(values: &[i32], value: i32) -> usize {
    let mut best = 0;
    for (i, v) in values.iter().enumerate() {
        if (v - value).abs() < (values[best] - value).abs() {
            best = i;
        }
    }
    best
}

/// 分頁名稱 → 索引（不分大小寫）；找不到 None。
pub fn page_index(page: &str) -> Option<usize> {
    PAGES.iter().position(|p| p.eq_ignore_ascii_case(page))
}

/// 快捷鍵的顯示（「Win + Shift + H」；空的是「（不使用）」）。
pub fn hotkey_text(hotkey: &str) -> String {
    if hotkey.trim().is_empty() { "（不使用）".to_owned() } else { hotkey.replace('+', " + ") }
}

/// 「今天的班表」。
pub fn schedule_text(schedule: Option<&DaySchedule>) -> String {
    let Some(s) = schedule else { return "登入後顯示".to_owned() };
    if let Some(h) = &s.holiday {
        return format!("放假（{h}）");
    }
    if s.working.is_empty() {
        return if s.on_leave { "請假" } else { "不上班" }.to_owned();
    }
    let segs: Vec<String> =
        s.working.iter().map(|w| format!("{}–{}", w.start.format("%H:%M"), w.end.format("%H:%M"))).collect();
    format!("{}　應出勤 {}", segs.join("、"), WorkMath::hours_minutes(s.expected_minutes as f64))
}

/// C# `Ago`：剛剛、N 分鐘前、N 小時前、N 天前。
pub fn ago(time: NaiveDateTime, now: NaiveDateTime) -> String {
    let span = now - time;
    let minutes = span.num_milliseconds() as f64 / 60000.0;
    if minutes < 1.0 {
        "剛剛".to_owned()
    } else if minutes < 60.0 {
        format!("{} 分鐘前", minutes as i64)
    } else if minutes < 1440.0 {
        format!("{} 小時前", (minutes / 60.0) as i64)
    } else {
        format!("{} 天前", (minutes / 1440.0) as i64)
    }
}

fn task_text(a: &RepoActivity) -> String {
    match &a.resolved_task {
        Some(t) => format!("任務 {t}（{}）", a.task_source.as_deref().unwrap_or("")),
        None => "偵測不到任務編號，不計時；可在面板上按「選擇」指定".to_owned(),
    }
}

/// 主資料夾本身的活動（C# Describe）。
pub fn describe(a: Option<&RepoActivity>, now: NaiveDateTime) -> (String, ColorKey) {
    let Some(a) = a.filter(|a| a.last_activity != NaiveDateTime::MIN) else {
        return ("主資料夾本身還沒有 Claude session 紀錄。".to_owned(), ColorKey::Muted);
    };
    let source = a.source_text();
    let source = if source.is_empty() { String::new() } else { format!("（{source}）") };
    if a.active {
        (format!("● 主資料夾本身：Claude 活動中{source} · {}", task_text(a)), ColorKey::Claude)
    } else {
        (format!("主資料夾本身：最近活動 {}{source} · {}", ago(a.last_activity, now), task_text(a)), ColorKey::Muted)
    }
}

/// 主資料夾裡的一個專案（C# ProjectItem）。
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectItem {
    pub path: String,
    pub name: String,
    pub is_git: bool,
    pub last_session: Option<NaiveDateTime>,
    pub source: String,
    pub enabled: bool,
    pub auto_create: bool,
    pub detail: String,
    pub detail_color: ColorKey,
    /// 圓點：None 是 LineBrush
    pub dot: Option<ColorKey>,
}

impl ProjectItem {
    pub fn new(d: &DetectedProject, config: Option<&ProjectConfig>) -> Self {
        Self {
            path: d.path.clone(),
            name: d.name.clone(),
            is_git: d.is_git,
            last_session: d.last_session,
            source: d.source.clone(),
            enabled: config.is_none_or(|c| c.enabled),
            auto_create: config.is_some_and(|c| c.auto_create),
            detail: String::new(),
            detail_color: ColorKey::Muted,
            dot: None,
        }
    }

    pub fn to_config(&self) -> ProjectConfig {
        ProjectConfig { name: self.name.clone(), enabled: self.enabled, auto_create: self.auto_create }
    }

    /// C# DescribeProject。
    pub fn describe(&mut self, a: Option<&RepoActivity>, now: NaiveDateTime) {
        let mut parts: Vec<String> = vec![if self.is_git { "Git" } else { "資料夾" }.to_owned()];
        if !self.enabled {
            parts.push("已停用".to_owned());
            self.detail = parts.join(" · ");
            self.detail_color = ColorKey::Muted;
            self.dot = None;
            return;
        }
        if let Some(a) = a.filter(|a| a.active) {
            let source = a.source_text();
            let source = if source.is_empty() { String::new() } else { format!("（{source}）") };
            self.detail = format!("● Claude 活動中{source} · {}", task_text(a));
            self.detail_color = ColorKey::Claude;
            self.dot = Some(ColorKey::Claude);
            return;
        }
        let last = a.filter(|a| a.last_activity != NaiveDateTime::MIN).map(|a| a.last_activity).or(self.last_session);
        let source = a.map(|a| a.source_text()).filter(|s| !s.is_empty()).unwrap_or_else(|| self.source.clone());
        match last {
            Some(time) => {
                if !source.is_empty() {
                    parts.push(source);
                }
                parts.push(ago(time, now));
                if let Some(task) = a.and_then(|a| a.resolved_task.as_ref()) {
                    parts.push(format!("任務 {task}"));
                }
            }
            None => parts.push("還沒有 Claude session".to_owned()),
        }
        self.detail = parts.join(" · ");
        self.detail_color = ColorKey::Muted;
        self.dot = last.map(|_| ColorKey::Run);
    }
}

/// 一個主資料夾（例 D:\repo）與裡面偵測到的專案（C# RepoItem）。
#[derive(Debug, Clone, PartialEq)]
pub struct RepoItem {
    pub path: String,
    pub enabled: bool,
    pub include_subprojects: bool,
    pub auto_create: bool,
    pub projects: Vec<ProjectItem>,
    /// 設定裡有、但這次沒偵測到的專案設定；存檔時保留，資料夾回來時設定還在。
    pub hidden_project_configs: Vec<ProjectConfig>,
    hooks_done: usize,
    hooks_total: usize,
    pub activity_text: String,
    pub activity_color: ColorKey,
    pub activity_visible: bool,
}

impl RepoItem {
    pub fn new(config: &RepoConfig, path: String) -> Self {
        Self {
            path,
            enabled: config.enabled,
            include_subprojects: config.include_subprojects,
            auto_create: config.auto_create,
            projects: Vec::new(),
            hidden_project_configs: Vec::new(),
            hooks_done: 0,
            hooks_total: 1,
            activity_text: String::new(),
            activity_color: ColorKey::Muted,
            activity_visible: true,
        }
    }

    pub fn name(&self) -> String {
        let trimmed = self.path.trim_end_matches(['\\', '/']);
        trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed).to_owned()
    }

    pub fn projects_header(&self) -> String {
        if self.include_subprojects {
            format!("裡面的專案（{}）", self.projects.len())
        } else {
            "裡面的專案（未偵測）".to_owned()
        }
    }

    pub fn set_hooks(&mut self, done: usize, total: usize) {
        self.hooks_done = done;
        self.hooks_total = total.max(1);
    }

    pub fn hooks_installed(&self) -> bool {
        self.hooks_done >= self.hooks_total
    }

    pub fn hooks_text(&self) -> String {
        if self.hooks_total <= 1 {
            if self.hooks_installed() { "✓ Hooks 已安裝" } else { "Hooks 未安裝" }.to_owned()
        } else if self.hooks_done == 0 {
            "Hooks 未安裝".to_owned()
        } else {
            format!("✓ Hooks {}/{}", self.hooks_done, self.hooks_total)
        }
    }

    pub fn hooks_color(&self) -> ColorKey {
        if self.hooks_done > 0 { ColorKey::Run } else { ColorKey::Muted }
    }

    pub fn hooks_action(&self) -> &'static str {
        if self.hooks_installed() { "移除 hooks" } else { "安裝 hooks" }
    }

    /// hooks 要裝在每個會開 Claude 的資料夾：主資料夾本身＋啟用中的專案。
    pub fn hook_targets(&self) -> Vec<String> {
        let mut targets = vec![self.path.clone()];
        if self.include_subprojects {
            targets.extend(self.projects.iter().filter(|p| p.enabled).map(|p| p.path.clone()));
        }
        targets
    }

    /// 存檔用：只存改過的專案設定，再加上這次沒偵測到的。
    pub fn saved_projects(&self) -> Vec<ProjectConfig> {
        self.projects
            .iter()
            .map(ProjectItem::to_config)
            .filter(|p| !p.is_default())
            .chain(self.hidden_project_configs.iter().cloned())
            .collect()
    }

    pub fn to_config(&self) -> RepoConfig {
        RepoConfig {
            path: self.path.clone(),
            enabled: self.enabled,
            include_subprojects: self.include_subprojects,
            auto_create: self.auto_create,
            projects: self.saved_projects(),
        }
    }

    /// 偵測到的專案套上已存的個別設定（C# LoadProjects 不碰 ClaudeMonitor 的部分）。
    pub fn set_projects(&mut self, detected: &[DetectedProject], saved: &[ProjectConfig]) {
        self.projects =
            detected.iter().map(|d| ProjectItem::new(d, saved.iter().find(|c| eq_ic(&c.name, &d.name)))).collect();
        self.hidden_project_configs =
            saved.iter().filter(|c| !detected.iter().any(|d| eq_ic(&d.name, &c.name))).cloned().collect();
    }

    fn card(&self) -> RepoCard {
        RepoCard {
            name: self.name().into(),
            path: self.path.as_str().into(),
            enabled: self.enabled,
            auto_create: self.auto_create,
            include_subprojects: self.include_subprojects,
            hooks_text: self.hooks_text().into(),
            hooks_color: self.hooks_color().code(),
            hooks_action: self.hooks_action().into(),
            activity_text: self.activity_text.as_str().into(),
            activity_color: self.activity_color.code(),
            activity_visible: self.activity_visible,
            projects_header: self.projects_header().into(),
            projects: ModelRc::new(VecModel::from(
                self.projects
                    .iter()
                    .map(|p| ProjectRow {
                        name: p.name.as_str().into(),
                        path: p.path.as_str().into(),
                        detail: p.detail.as_str().into(),
                        detail_color: p.detail_color.code(),
                        dot_color: p.dot.map_or(0, ColorKey::code),
                        enabled: p.enabled,
                        auto_create: p.auto_create,
                    })
                    .collect::<Vec<_>>(),
            )),
        }
    }
}

/// 延到下一輪事件迴圈（開對話框、選單時不要在 Slint 的回呼裡跑另一個訊息迴圈）。
fn later(f: impl FnOnce() + 'static) {
    slint::Timer::single_shot(Duration::ZERO, f);
}

pub(crate) struct SettingsInner {
    pub(crate) engine: TimerEngine,
    pub(crate) ui: SettingsWindow,
    loading: Cell<bool>,
    repos: RefCell<Vec<RepoItem>>,
    pub(crate) me: Weak<SettingsInner>,
    changed_id: Cell<Option<HandlerId>>,
    slack_status_id: Cell<Option<u64>>,
}

/// 設定視窗。App 持有一個；關掉後再開會重建（C# `_settings is not { IsLoaded: true }`）。
#[derive(Clone)]
pub struct Settings(Rc<SettingsInner>);

macro_rules! with_settings {
    ($me:expr, |$p:ident $(, $arg:ident)*| $body:expr) => {{
        let weak = $me.clone();
        move |$($arg),*| {
            if let Some($p) = weak.upgrade() {
                $body
            }
        }
    }};
}
pub(crate) use with_settings;

impl Settings {
    pub fn new(engine: TimerEngine) -> Option<Settings> {
        let ui = match SettingsWindow::new() {
            Ok(ui) => ui,
            Err(e) => {
                log::error!("建立設定視窗失敗: {e}");
                return None;
            }
        };
        crate::track_theme!(ui);
        let inner = Rc::new_cyclic(|me| SettingsInner {
            engine: engine.clone(),
            ui,
            loading: Cell::new(true),
            repos: RefCell::new(Vec::new()),
            me: me.clone(),
            changed_id: Cell::new(None),
            slack_status_id: Cell::new(None),
        });
        inner.load();
        inner.wire();
        inner.refresh_live();
        inner.loading.set(false);

        let me = inner.me.clone();
        let id = engine.events().changed.subscribe(with_settings!(me, |p, _a| p.refresh_live()));
        inner.changed_id.set(Some(id));
        let me = inner.me.clone();
        ThemeManager::on_changed(with_settings!(me, |p| p.refresh_live()));
        Some(Settings(inner))
    }

    pub fn show(&self) {
        if let Err(e) = self.0.ui.show() {
            log::error!("顯示設定視窗失敗: {e}");
            return;
        }
        crate::theme_manager::set_title_bar(self.0.ui.window(), ThemeManager::is_dark());
    }

    /// 切到指定分頁：account、taskbar、schedule、repos、claude、eip、slack、general、about。
    pub fn show_page(&self, page: &str) {
        if let Some(i) = page_index(page) {
            self.0.ui.set_page(i as i32);
            self.0.nav_checked(i);
        }
    }

    pub fn activate(&self) {
        let window = self.0.ui.window();
        if window.is_minimized() {
            window.set_minimized(false);
        }
        hb_ui_common::window_util::activate(window);
    }

    pub fn window(&self) -> &slint::Window {
        self.0.ui.window()
    }

    /// 關掉（使用者按 ×）時呼叫一次 f（C# Closed）。
    pub fn on_closed(&self, f: impl Fn() + 'static) {
        let me = self.0.me.clone();
        self.0.ui.window().on_close_requested(move || {
            if let Some(p) = me.upgrade() {
                p.closed();
            }
            f();
            slint::CloseRequestResponse::HideWindow
        });
    }
}

impl SettingsInner {
    fn apply(&self) {
        if !self.loading.get() {
            self.engine.apply_config();
        }
    }

    fn closed(&self) {
        if let Some(id) = self.changed_id.take() {
            self.engine.events().changed.unsubscribe(id);
        }
        if let (Some(id), Some(slack)) = (self.slack_status_id.take(), self.engine.slack()) {
            slack.unsubscribe_status_changed(id);
        }
    }

    fn load(&self) {
        let ui = &self.ui;
        {
            let c = self.engine.config();
            ui.set_auto_schedule(c.auto_schedule);
            ui.set_claude_enabled(c.claude_enabled);
            ui.set_claude_only_work(c.claude_only_during_work);
            ui.set_poll_index(nearest(&POLL_SECONDS, c.poll_seconds) as i32);
            ui.set_theme_index(match c.theme_mode.as_str() {
                "Light" => 1,
                "Dark" => 2,
                _ => 0,
            });
            ui.set_panel_height_index(if c.panel_height <= 0 {
                4
            } else {
                nearest(&PANEL_HEIGHTS[..4], c.panel_height) as i32
            });
            ui.set_hotkey_text(hotkey_text(&c.hotkey).into());
            ui.set_meeting_alerts(c.meeting_alerts);
            ui.set_meeting_all_projects(c.meeting_alerts_all_projects);
            ui.set_meeting_pauses_others(c.meeting_pauses_others);
            ui.set_meeting_lead_index(nearest(&MEETING_LEADS, c.meeting_lead_minutes) as i32);
            ui.set_reminder_alerts(c.reminder_alerts);
            ui.set_notification_alerts(c.notification_alerts);
            ui.set_popup_seconds_index(if c.notification_popup_seconds <= 0 {
                4
            } else {
                nearest(&POPUP_SECONDS[..4], c.notification_popup_seconds) as i32
            });
            ui.set_sync_only_work(c.sync_only_during_work);
            ui.set_prompt_unfilled(c.prompt_unfilled);
            ui.set_quiet_when_busy(c.quiet_when_busy);
        }
        ui.set_startup(startup_registration::is_enabled());
        self.load_taskbar();
        self.load_eip();
        self.load_slack();
        let repos: Vec<RepoConfig> = self.engine.config().repos.clone();
        for r in &repos {
            self.add_repo_item(r);
        }
        ui.set_data_dir(AppPaths::dir().display().to_string().into());
        ui.set_version_text(format!("版本 {}", env!("CARGO_PKG_VERSION")).into());
        ui.set_page(0);
        self.render_repos();
    }

    fn wire(&self) {
        let ui = &self.ui;
        ui.on_page_selected(with_settings!(self.me, |p, i| p.nav_checked(i as usize)));
        // 帳號
        ui.on_account_clicked(with_settings!(self.me, |p| {
            let e = p.engine.clone();
            spawn(async move {
                if e.logged_in() {
                    e.logout().await;
                } else {
                    e.login().await;
                }
            });
        }));
        ui.on_open_web(with_settings!(self.me, |p| hb_win::open(&p.engine.config().base_url.clone())));
        ui.on_web_setting(with_settings!(self.me, |p, path| hb_win::open(&p.engine.web_url(&path))));
        // 工作列
        ui.on_anchor_changed(with_settings!(self.me, |p, i| p.anchor_changed(i as usize)));
        ui.on_layout_changed(with_settings!(self.me, |p, i| {
            if p.loading.get() {
                return;
            }
            p.engine.config_mut().taskbar_layout = if i == 0 { "OneLine" } else { "TwoLine" }.to_owned();
            p.apply();
        }));
        ui.on_offset_commit(with_settings!(self.me, |p| p.offset_commit()));
        ui.on_offset_reset(with_settings!(self.me, |p| {
            p.ui.set_offset_text("0".into());
            p.engine.config_mut().taskbar_offset = 0.0;
            p.apply();
        }));
        // 開關
        ui.on_toggled(with_settings!(self.me, |p| p.toggle_clicked()));
        ui.on_popup_seconds_changed(
            with_settings!(self.me, |p, i| p.set_int(i, &POPUP_SECONDS, |c, v| c.notification_popup_seconds = v))
        );
        ui.on_meeting_lead_changed(
            with_settings!(self.me, |p, i| p.set_int(i, &MEETING_LEADS, |c, v| c.meeting_lead_minutes = v))
        );
        ui.on_poll_changed(with_settings!(self.me, |p, i| p.set_int(i, &POLL_SECONDS, |c, v| c.poll_seconds = v)));
        ui.on_panel_height_changed(
            with_settings!(self.me, |p, i| p.set_int(i, &PANEL_HEIGHTS, |c, v| c.panel_height = v))
        );
        ui.on_theme_changed(with_settings!(self.me, |p, i| {
            if p.loading.get() {
                return;
            }
            let Some(mode) = THEMES.get(i as usize) else { return };
            p.engine.config_mut().theme_mode = (*mode).to_owned();
            ThemeManager::set_mode(mode);
            p.apply();
        }));
        let me = self.me.clone();
        ui.on_hotkey_key(move |e| me.upgrade().is_some_and(|p| p.hotkey_key(&e)));
        ui.on_hotkey_default(with_settings!(self.me, |p| p.set_hotkey("Win+Shift+H")));
        ui.on_startup_toggled(with_settings!(self.me, |p, v| p.startup_clicked(v)));
        ui.on_open_data_dir(with_settings!(self.me, |_p| hb_win::open(&AppPaths::dir().display().to_string())));
        // EIP
        ui.on_eip_login(with_settings!(self.me, |p| p.eip_test()));
        ui.on_eip_logout(with_settings!(self.me, |p| p.eip_logout()));
        ui.on_eip_sync_toggled(with_settings!(self.me, |p, v| {
            p.engine.config_mut().eip_leave_sync = v;
            p.apply();
        }));
        ui.on_eip_sync_now(with_settings!(self.me, |p| p.eip_sync_now()));
        // 專案
        ui.on_add_repo(with_settings!(self.me, |p| p.add_repo_clicked()));
        ui.on_discover(with_settings!(self.me, |p, x, y| p.discover_clicked(x, y)));
        ui.on_repo_toggle(with_settings!(self.me, |p, i, field, v| p.repo_toggle(i as usize, &field, v)));
        ui.on_project_toggle(
            with_settings!(self.me, |p, i, j, field, v| p.project_toggle(i as usize, j as usize, &field, v))
        );
        ui.on_hooks(with_settings!(self.me, |p, i| p.hooks_clicked(i as usize)));
        ui.on_remove_repo(with_settings!(self.me, |p, i| p.remove_repo(i as usize)));
        ui.on_redetect(with_settings!(self.me, |p, i| p.redetect(i as usize)));
        self.wire_slack();

        // 右鍵選單也能改位置，回到這個視窗時同步（C# Activated → LoadTaskbar）
        let me = self.me.clone();
        ui.window().on_winit_window_event(move |_w, event| {
            if let WindowEvent::Focused(true) = event {
                let me = me.clone();
                later(move || {
                    if let Some(p) = me.upgrade() {
                        p.load_taskbar();
                    }
                });
            }
            EventResult::Propagate
        });
    }

    fn nav_checked(&self, page: usize) {
        if PAGES.get(page) == Some(&"general") {
            self.ui.set_mcp_stats(hb_backend::mcp::mcp_stats::current().into());
        }
    }

    // ───────────── 會隨時間變的內容 ─────────────

    fn refresh_live(&self) {
        let engine = &self.engine;
        let ui = &self.ui;
        match engine.user().filter(|_| engine.logged_in()) {
            Some(user) => {
                ui.set_account_name(user.display_name.as_str().into());
                let version = user.hayaku_version.as_ref().map(|v| format!(" · Hayaku {v}")).unwrap_or_default();
                ui.set_account_detail(format!("{} · 已連線{version}", user.username).into());
                ui.set_avatar_text(
                    user.display_name.chars().next().map_or_else(|| "?".to_owned(), |c| c.to_string()).into(),
                );
                ui.set_nav_user(user.display_name.as_str().into());
            }
            None => {
                ui.set_account_name(if engine.logging_in() { "等待瀏覽器登入…" } else { "尚未登入" }.into());
                ui.set_account_detail("登入後才能同步計時".into());
                ui.set_avatar_text("?".into());
                ui.set_nav_user("未登入".into());
            }
        }
        ui.set_account_button(if engine.logged_in() { "登出" } else { "登入" }.into());
        ui.set_account_primary(!engine.logged_in());
        ui.set_account_enabled(!engine.logging_in());
        ui.set_schedule_text(schedule_text(engine.schedule().as_ref()).into());

        let now = Local::now().naive_local();
        {
            let claude = engine.claude();
            let unit = |path: &str| claude.repos().find(|a| eq_ic(&a.path, path)).cloned();
            let mut repos = self.repos.borrow_mut();
            for item in repos.iter_mut() {
                let root = unit(&item.path);
                (item.activity_text, item.activity_color) = describe(root.as_ref(), now);
                // 有專案、主資料夾本身又沒有 session 時，不顯示主資料夾那一行
                item.activity_visible = !(item.include_subprojects
                    && !item.projects.is_empty()
                    && root.as_ref().is_none_or(|r| r.last_activity == NaiveDateTime::MIN));
                for project in item.projects.iter_mut() {
                    let a = unit(&project.path);
                    project.describe(a.as_ref(), now);
                }
            }
        }
        self.render_repos();
        self.refresh_eip_sync();
    }

    fn render_repos(&self) {
        let cards: Vec<RepoCard> = self.repos.borrow().iter().map(RepoItem::card).collect();
        self.ui.set_repos(ModelRc::new(VecModel::from(cards)));
    }

    // ───────────── EIP 請假 ─────────────

    fn load_eip(&self) {
        let cred = EipCredentialStore::load();
        self.show_eip_account(if cred.is_empty() { None } else { Some(cred.account.clone()) });
        self.ui.set_eip_sync(self.engine.config().eip_leave_sync);
    }

    /// 已登入（存了帳密）只顯示帳號、已登入與登出；沒有帳密才顯示輸入欄位。
    fn show_eip_account(&self, account: Option<String>) {
        let logged_in = account.is_some();
        self.ui.set_eip_logged_in(logged_in);
        self.ui.set_eip_account_text(
            account.map(|a| format!("{a} · 帳密用 Windows 加密存在這台電腦")).unwrap_or_default().into(),
        );
        self.ui.set_eip_sync_enabled(logged_in && !self.engine.eip_syncing());
        if !logged_in {
            self.ui.set_eip_password(SharedString::new());
        }
    }

    /// 上次同步的時間與結果。
    fn refresh_eip_sync(&self) {
        let syncing = self.engine.eip_syncing();
        self.ui.set_eip_sync_enabled(!syncing && self.ui.get_eip_logged_in());
        let (text, error) = {
            let s = self.engine.eip_sync();
            (eip_sync_text(&s, syncing), s.last_result.starts_with("失敗"))
        };
        self.ui.set_eip_sync_status(text.into());
        self.ui.set_eip_sync_color(if error { ColorKey::Error } else { ColorKey::Muted }.code());
    }

    fn eip_sync_now(&self) {
        self.ui.set_eip_sync_enabled(false);
        self.ui.set_eip_sync_status("同步中…".into());
        let me = self.me.clone();
        let engine = self.engine.clone();
        spawn(async move {
            let result = engine.sync_eip_leaves(true, false).await;
            if let Some(p) = me.upgrade() {
                p.refresh_eip_sync();
                p.ui.set_eip_sync_status(format!("剛剛同步：{result}").into());
            }
        });
    }

    fn show_eip_status(&self, text: Option<&str>, color: ColorKey) {
        self.ui.set_eip_status(text.unwrap_or("").into());
        self.ui.set_eip_status_color(color.code());
    }

    fn eip_test(&self) {
        let cred = EipCredentials::new(self.ui.get_eip_account().trim(), self.ui.get_eip_password().as_str());
        if cred.is_empty() {
            self.show_eip_status(Some("請輸入帳號和密碼"), ColorKey::Error);
            return;
        }
        self.ui.set_eip_test_enabled(false);
        self.show_eip_status(Some("登入中…"), ColorKey::Muted);
        /// 登入那一步的結果
        enum Outcome {
            /// 帳密不對（C# result.Ok == false）
            LoginFailed(String),
            /// 登入成功、讀完假單
            Read(String),
        }
        let me = self.me.clone();
        spawn(async move {
            let client = EipClient::new();
            let result: Result<Outcome, String> = async {
                let login = client.login(&cred).await.map_err(|e| e.to_string())?;
                if !login.ok {
                    return Ok(Outcome::LoginFailed(format!("登入失敗：{}", login.message)));
                }
                EipCredentialStore::save(&cred).map_err(|e| e.to_string())?;
                if let Some(p) = me.upgrade() {
                    p.engine.reset_eip();
                    p.show_eip_account(Some(cred.account.clone()));
                    p.show_eip_status(Some("讀取假單中…"), ColorKey::Muted);
                }
                let year = chrono::Datelike::year(&Local::now().date_naive());
                let mut leaves =
                    client.list_leaves(year, EipClient::STATE_PENDING, &cred).await.map_err(|e| e.to_string())?;
                leaves.extend(
                    client.list_leaves(year, EipClient::STATE_APPROVED, &cred).await.map_err(|e| e.to_string())?,
                );
                // OrderByDescending(l => l.Start)：穩定排序
                leaves.sort_by_key(|l| std::cmp::Reverse(l.start));
                Ok(Outcome::Read(leaves_summary(&leaves)))
            }
            .await;
            let Some(p) = me.upgrade() else { return };
            match result {
                Ok(Outcome::LoginFailed(text)) => p.show_eip_status(Some(&text), ColorKey::Error),
                Ok(Outcome::Read(text)) => p.show_eip_status(Some(&text), ColorKey::Muted),
                Err(e) => {
                    log::error!("EIP 登入失敗: {e}");
                    p.show_eip_status(Some(&format!("連不上 EIP：{e}")), ColorKey::Error);
                }
            }
            p.ui.set_eip_test_enabled(true);
        });
    }

    fn eip_logout(&self) {
        let me = self.me.clone();
        later(move || {
            let Some(p) = me.upgrade() else { return };
            if !dialogs::confirm(
                Some(p.ui.window()),
                "EIP 請假",
                "登出 EIP？會刪掉存在這台電腦的帳密，之後不再同步請假。",
            ) {
                return;
            }
            EipCredentialStore::clear();
            p.engine.reset_eip();
            p.ui.set_eip_account(SharedString::new());
            p.show_eip_account(None);
            p.show_eip_status(None, ColorKey::Muted);
        });
    }

    // ───────────── 工作列 ─────────────

    fn load_taskbar(&self) {
        let was = self.loading.replace(true);
        let (anchor, layout, offset) = {
            let c = self.engine.config();
            (c.taskbar_anchor.clone(), c.taskbar_layout.clone(), c.taskbar_offset)
        };
        self.ui.set_anchor_index(ANCHORS.iter().position(|a| *a == anchor).unwrap_or(3) as i32);
        self.ui.set_layout_index(if layout.eq_ignore_ascii_case("OneLine") { 0 } else { 1 });
        // 輸入框正在打字時不蓋掉（C# !OffsetBox.IsKeyboardFocusWithin）
        if !self.offset_editing() {
            self.ui.set_offset_text(format!("{}", offset.round_ties_even() as i64).into());
        }
        self.loading.set(was);
    }

    fn offset_editing(&self) -> bool {
        let text = self.ui.get_offset_text();
        let saved = self.engine.config().taskbar_offset.round_ties_even() as i64;
        text.as_str().trim().parse::<f64>().map_or(true, |v| v.round() as i64 != saved)
    }

    fn anchor_changed(&self, index: usize) {
        let Some(anchor) = ANCHORS.get(index) else { return };
        if self.loading.get() || *anchor == self.engine.config().taskbar_anchor {
            return;
        }
        {
            let mut c = self.engine.config_mut();
            c.taskbar_anchor = (*anchor).to_owned();
            c.taskbar_offset = 0.0; // 換基準位置時微調歸零
        }
        self.ui.set_offset_text("0".into());
        self.apply();
    }

    fn offset_commit(&self) {
        let text = self.ui.get_offset_text();
        let Ok(offset) = text.trim().parse::<f64>() else {
            self.ui.set_offset_text(format!("{}", self.engine.config().taskbar_offset.round_ties_even() as i64).into());
            return;
        };
        let offset = offset.clamp(-3000.0, 3000.0);
        self.ui.set_offset_text(format!("{}", offset.round_ties_even() as i64).into());
        if (offset - self.engine.config().taskbar_offset).abs() < 0.5 {
            return;
        }
        self.engine.config_mut().taskbar_offset = offset;
        self.apply();
    }

    // ───────────── 開關 ─────────────

    fn toggle_clicked(&self) {
        let ui = &self.ui;
        {
            let mut c = self.engine.config_mut();
            c.auto_schedule = ui.get_auto_schedule();
            c.claude_enabled = ui.get_claude_enabled();
            c.claude_only_during_work = ui.get_claude_only_work();
            c.meeting_alerts = ui.get_meeting_alerts();
            c.meeting_alerts_all_projects = ui.get_meeting_all_projects();
            c.meeting_pauses_others = ui.get_meeting_pauses_others();
            c.reminder_alerts = ui.get_reminder_alerts();
            c.notification_alerts = ui.get_notification_alerts();
            c.prompt_unfilled = ui.get_prompt_unfilled();
            c.quiet_when_busy = ui.get_quiet_when_busy();
            c.sync_only_during_work = ui.get_sync_only_work();
        }
        self.apply();
    }

    fn set_int(&self, index: i32, values: &[i32], set: impl FnOnce(&mut hb_backend::config::AppConfig, i32)) {
        if self.loading.get() {
            return;
        }
        let Some(v) = values.get(index as usize) else { return };
        set(&mut self.engine.config_mut(), *v);
        self.apply();
    }

    /// 錄製快捷鍵：按下組合鍵就套用；Backspace／Delete 取消快捷鍵；Tab、Esc 不處理。
    fn hotkey_key(&self, e: &slint::private_unstable_api::re_exports::KeyEvent) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState};
        let text = e.text.as_str();
        if text == "\u{8}" || text == "\u{7f}" {
            self.set_hotkey("");
            return true;
        }
        if text == "\t" || text == "\u{1b}" {
            return false;
        }
        let down = |vk: i32| unsafe { GetKeyState(vk) } < 0;
        let win = down(0x5B) || down(0x5C);
        let mut modifiers = hb_win::ModifierKeys::NONE;
        if down(0x11) {
            modifiers = modifiers | hb_win::ModifierKeys::CONTROL;
        }
        if down(0x12) {
            modifiers = modifiers | hb_win::ModifierKeys::ALT;
        }
        if down(0x10) {
            modifiers = modifiers | hb_win::ModifierKeys::SHIFT;
        }
        // 按下的那個鍵（修飾鍵以外）
        let vk = (0x08..=0xFEu32)
            .filter(|vk| !matches!(vk, 0x10 | 0x11 | 0x12 | 0xA0..=0xA5 | 0x5B | 0x5C))
            .find(|vk| unsafe { GetAsyncKeyState(*vk as i32) } as u16 & 0x8000 != 0);
        if let Some(text) = vk.and_then(|vk| hb_win::global_hotkey::format(modifiers, win, vk)) {
            self.set_hotkey(&text);
        }
        true
    }

    fn set_hotkey(&self, text: &str) {
        self.engine.config_mut().hotkey = text.to_owned();
        self.ui.set_hotkey_text(hotkey_text(text).into());
        self.apply(); // App 收到 ConfigApplied 會重新註冊；被占用時會跳提示
    }

    fn startup_clicked(&self, enabled: bool) {
        if let Err(e) = startup_registration::set(enabled) {
            log::error!("設定開機啟動失敗: {e}");
            self.ui.set_startup(startup_registration::is_enabled());
            let me = self.me.clone();
            let message = format!("設定失敗：{e}");
            later(move || {
                if let Some(p) = me.upgrade() {
                    dialogs::show(Some(p.ui.window()), "HayakuBar", &message, Level::Error);
                }
            });
        }
    }

    // ───────────── 主資料夾與專案 ─────────────

    fn add_repo_item(&self, config: &RepoConfig) {
        let path = ClaudeMonitor::normalize(&config.path).unwrap_or_else(|_| config.path.clone());
        let mut item = RepoItem::new(config, path);
        self.load_projects(&mut item, &config.projects.clone(), false);
        self.repos.borrow_mut().push(item);
    }

    /// 偵測主資料夾裡的專案，套上已存的個別設定。
    fn load_projects(&self, item: &mut RepoItem, saved: &[ProjectConfig], refresh: bool) {
        let detected = self.engine.claude_mut().detect_projects(&item.path, refresh);
        item.set_projects(&detected, saved);
        self.update_hooks(item);
    }

    fn update_hooks(&self, item: &mut RepoItem) {
        let port = self.engine.config().hook_port;
        let targets = item.hook_targets();
        let done = targets.iter().filter(|t| HookInstaller::is_installed(t, port).unwrap_or(false)).count();
        item.set_hooks(done, targets.len());
    }

    fn save_repos(&self) {
        let configs: Vec<RepoConfig> = self.repos.borrow().iter().map(RepoItem::to_config).collect();
        self.engine.config_mut().repos = configs;
        self.apply();
        self.refresh_live();
    }

    fn repo_toggle(&self, index: usize, field: &str, value: bool) {
        {
            let mut repos = self.repos.borrow_mut();
            let Some(item) = repos.get_mut(index) else { return };
            match field {
                "enabled" => item.enabled = value,
                "auto-create" => item.auto_create = value,
                "include" => item.include_subprojects = value,
                _ => return,
            }
        }
        // C# OnRepoEdited：存檔、更新 hooks 狀態
        self.save_repos();
        let mut repos = self.repos.borrow_mut();
        if let Some(item) = repos.get_mut(index) {
            self.update_hooks(item);
        }
        drop(repos);
        self.render_repos();
    }

    fn project_toggle(&self, index: usize, project: usize, field: &str, value: bool) {
        {
            let mut repos = self.repos.borrow_mut();
            let Some(p) = repos.get_mut(index).and_then(|r| r.projects.get_mut(project)) else { return };
            match field {
                "enabled" => p.enabled = value,
                "auto-create" => p.auto_create = value,
                _ => return,
            }
        }
        self.save_repos();
    }

    fn redetect(&self, index: usize) {
        let item = self.repos.borrow().get(index).cloned();
        let Some(mut item) = item else { return };
        let saved = item.saved_projects();
        self.load_projects(&mut item, &saved, true);
        if let Some(slot) = self.repos.borrow_mut().get_mut(index) {
            *slot = item;
        }
        self.save_repos();
    }

    fn add_repo_clicked(&self) {
        let me = self.me.clone();
        later(move || {
            let Some(p) = me.upgrade() else { return };
            let mut dialog = rfd::FileDialog::new().set_title("選擇放專案的主資料夾（例 D:\\repo），或單一專案資料夾");
            let handle = p.ui.window().window_handle();
            {
                use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
                if handle.window_handle().is_ok() && handle.display_handle().is_ok() {
                    dialog = dialog.set_parent(&handle);
                }
            }
            if let Some(folder) = dialog.pick_folder() {
                p.add_folder(&folder.display().to_string());
            }
        });
    }

    fn add_folder(&self, folder: &str) {
        let path = ClaudeMonitor::normalize(folder).unwrap_or_else(|_| folder.to_owned());
        if self.repos.borrow().iter().any(|r| eq_ic(&r.path, &path)) {
            return;
        }
        // 已經是某個主資料夾裡的專案：不用另外加
        let parent =
            self.repos.borrow().iter().find(|r| r.projects.iter().any(|p| eq_ic(&p.path, &path))).map(RepoItem::name);
        if let Some(parent) = parent {
            let name =
                std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            dialogs::show(
                Some(self.ui.window()),
                "HayakuBar",
                &format!("「{name}」已經是「{parent}」裡偵測到的專案，不用另外加。"),
                Level::Info,
            );
            return;
        }
        self.add_repo_item(&RepoConfig { path, ..Default::default() });
        self.save_repos();
    }

    fn discover_clicked(&self, x: f32, y: f32) {
        let now = Local::now().naive_local();
        let covered: Vec<String> = self
            .repos
            .borrow()
            .iter()
            .flat_map(|r| std::iter::once(r.path.clone()).chain(r.projects.iter().map(|p| p.path.clone())))
            .collect();
        let projects: Vec<_> =
            ClaudeMonitor::discover(15).into_iter().filter(|p| !covered.iter().any(|c| eq_ic(c, &p.path))).collect();
        let mut entries: Vec<MenuEntry> = projects
            .iter()
            .map(|p| MenuEntry::item(format!("{}　{} · {}", p.path, p.source, ago(p.last_activity, now))))
            .collect();
        if projects.is_empty() {
            entries.push(MenuEntry::item_if("最近用過的資料夾都已經在清單裡了", false));
        }
        let anchor = menu::to_screen(self.ui.window(), x, y, 1.0, 1.0);
        let me = self.me.clone();
        menu::show_later(self.ui.as_weak(), entries, MenuPlace::Below(anchor), move |i| {
            if let (Some(p), Some(project)) = (me.upgrade(), projects.get(i)) {
                p.add_folder(&project.path);
            }
        });
    }

    fn remove_repo(&self, index: usize) {
        let Some(name) = self.repos.borrow().get(index).map(RepoItem::name) else { return };
        let me = self.me.clone();
        later(move || {
            let Some(p) = me.upgrade() else { return };
            if !dialogs::confirm(
                Some(p.ui.window()),
                "移除資料夾",
                &format!("從清單移除「{name}」？（不會刪除任何檔案）"),
            ) {
                return;
            }
            if index < p.repos.borrow().len() {
                p.repos.borrow_mut().remove(index);
            }
            p.save_repos();
        });
    }

    fn hooks_clicked(&self, index: usize) {
        let me = self.me.clone();
        later(move || {
            let Some(p) = me.upgrade() else { return };
            let Some(item) = p.repos.borrow().get(index).cloned() else { return };
            let port = p.engine.config().hook_port;
            let targets = item.hook_targets();
            let uninstall = item.hooks_installed();
            if !uninstall {
                let files: Vec<String> =
                    targets.iter().map(|t| HookInstaller::settings_path(t).display().to_string()).collect();
                let text = format!(
                    "會在下面 {} 個檔案加入 UserPromptSubmit／Stop／SessionEnd 三個 hook（原檔會備份成 .hayakubar.bak）：\n\n{}\n\n確定要安裝嗎？",
                    targets.len(),
                    files.join("\n")
                );
                if !dialogs::confirm(Some(p.ui.window()), "安裝 Claude hooks", &text) {
                    return;
                }
            }
            let mut failed: Vec<String> = Vec::new();
            for target in &targets {
                let result = if uninstall {
                    HookInstaller::uninstall(target, port)
                } else {
                    HookInstaller::install(target, port)
                };
                if let Err(e) = result {
                    log::error!("變更 hooks 失敗：{target}: {e}");
                    failed.push(format!("{target}：{e}"));
                }
            }
            if let Some(item) = p.repos.borrow_mut().get_mut(index) {
                p.update_hooks(item);
            }
            p.render_repos();
            if !failed.is_empty() {
                dialogs::show(
                    Some(p.ui.window()),
                    "HayakuBar",
                    &format!("部分資料夾失敗：\n{}", failed.join("\n")),
                    Level::Warning,
                );
            }
        });
    }
}

/// EIP 同步狀態（C# RefreshEipSync 的文字）。
pub fn eip_sync_text(s: &hb_eip::EipSyncState, syncing: bool) -> String {
    let at = |t: NaiveDateTime| {
        format!(
            "{}/{}（{}）{}",
            t.format("%-m"),
            t.format("%-d"),
            hb_ui_common::inputs::weekday_short(t.date()),
            t.format("%H:%M")
        )
    };
    let mut lines: Vec<String> = Vec::new();
    if let Some(m) = s.morning_at {
        lines.push(format!("上班前 {}：{}", at(m), s.morning_result));
    }
    if let Some(a) = s.after_work_at {
        lines.push(format!("下班 {}：{}", at(a), s.after_work_result));
    }
    if let Some(last) = s.last_sync_at
        && last > s.morning_at.unwrap_or(NaiveDateTime::MIN)
        && last > s.after_work_at.unwrap_or(NaiveDateTime::MIN)
    {
        lines.push(format!("{} {}：{}", if lines.is_empty() { "上次同步" } else { "手動" }, at(last), s.last_result));
    }
    if syncing {
        "同步中…".to_owned()
    } else if !lines.is_empty() {
        lines.join("\n")
    } else if !s.last_result.is_empty() {
        s.last_result.clone()
    } else {
        "還沒同步過".to_owned()
    }
}

/// 「今年讀到 5 筆假單：11/12（週四） 09:00–18:00 特休（核准）、…」（最近 3 筆）。
pub fn leaves_summary(leaves: &[hb_eip::EipLeave]) -> String {
    let recent: Vec<String> = leaves
        .iter()
        .take(3)
        .map(|l| {
            let day = format!(
                "{}/{}（{}）",
                l.start.format("%-m"),
                l.start.format("%-d"),
                hb_ui_common::inputs::weekday_short(l.start.date())
            );
            let end = if l.end.date() == l.start.date() {
                l.end.format("%H:%M").to_string()
            } else {
                l.end.format("%-m/%-d %H:%M").to_string()
            };
            format!("{day} {}–{end} {}（{}）", l.start.format("%H:%M"), l.r#type, l.status)
        })
        .collect();
    let mut text = format!("今年讀到 {} 筆假單", leaves.len());
    if !leaves.is_empty() {
        text += &format!("：{}", recent.join("、"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};
    use hb_api::WorkSegment;

    fn dt(m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    #[test]
    fn 最接近的選項() {
        assert_eq!(1, nearest(&POLL_SECONDS, 30));
        assert_eq!(0, nearest(&POLL_SECONDS, 1));
        assert_eq!(3, nearest(&POLL_SECONDS, 999));
        // 一樣近取前面的（C# MinBy）
        assert_eq!(0, nearest(&[10, 20], 15));
        assert_eq!(2, nearest(&MEETING_LEADS, 4));
    }

    #[test]
    fn 分頁名稱() {
        assert_eq!(Some(4), page_index("claude"));
        assert_eq!(Some(7), page_index("General"));
        assert_eq!(Some(8), page_index("about"));
        assert_eq!(None, page_index("x"));
    }

    #[test]
    fn 快捷鍵顯示() {
        assert_eq!("（不使用）", hotkey_text(""));
        assert_eq!("Win + Shift + H", hotkey_text("Win+Shift+H"));
    }

    #[test]
    fn 今天的班表() {
        assert_eq!("登入後顯示", schedule_text(None));
        let mut s = DaySchedule {
            date: NaiveDate::from_ymd_opt(2026, 11, 12).unwrap(),
            working: vec![],
            expected_minutes: 0,
            on_leave: false,
            holiday: Some("國慶日".into()),
        };
        assert_eq!("放假（國慶日）", schedule_text(Some(&s)));
        s.holiday = None;
        assert_eq!("不上班", schedule_text(Some(&s)));
        s.on_leave = true;
        assert_eq!("請假", schedule_text(Some(&s)));
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        s.working = vec![WorkSegment { start: t(9, 0), end: t(12, 0) }, WorkSegment { start: t(13, 0), end: t(18, 0) }];
        s.expected_minutes = 480;
        assert_eq!("09:00–12:00、13:00–18:00　應出勤 8 小時", schedule_text(Some(&s)));
    }

    #[test]
    fn 多久以前() {
        let now = dt(11, 12, 10, 0);
        assert_eq!("剛剛", ago(now - chrono::TimeDelta::seconds(59), now));
        assert_eq!("1 分鐘前", ago(dt(11, 12, 9, 59), now));
        assert_eq!("5 分鐘前", ago(dt(11, 12, 9, 55), now));
        assert_eq!("2 小時前", ago(dt(11, 12, 7, 30), now));
        assert_eq!("3 天前", ago(dt(11, 9, 9, 0), now));
    }

    fn activity(active: bool, last: NaiveDateTime, task: Option<&str>) -> RepoActivity {
        let mut a = RepoActivity::new("D:\\repo");
        a.active = active;
        a.last_activity = last;
        a.resolved_task = task.map(str::to_owned);
        a.task_source = task.map(|_| "分支".to_owned());
        a
    }

    #[test]
    fn 主資料夾本身的活動() {
        let now = dt(11, 12, 10, 0);
        assert_eq!(("主資料夾本身還沒有 Claude session 紀錄。".to_owned(), ColorKey::Muted), describe(None, now));
        assert_eq!(ColorKey::Muted, describe(Some(&activity(false, NaiveDateTime::MIN, None)), now).1);
        let (text, color) = describe(Some(&activity(true, dt(11, 12, 9, 59), Some("F2P-1"))), now);
        assert_eq!(ColorKey::Claude, color);
        assert!(text.starts_with("● 主資料夾本身：Claude 活動中"), "{text}");
        assert!(text.ends_with(" · 任務 F2P-1（分支）"), "{text}");
        let (text, _) = describe(Some(&activity(false, dt(11, 12, 9, 0), None)), now);
        assert!(text.starts_with("主資料夾本身：最近活動 1 小時前"), "{text}");
        assert!(text.ends_with(" · 偵測不到任務編號，不計時；可在面板上按「選擇」指定"), "{text}");
    }

    fn detected(name: &str, git: bool, last: Option<NaiveDateTime>) -> DetectedProject {
        DetectedProject {
            path: format!("D:\\repo\\{name}"),
            name: name.into(),
            is_git: git,
            last_session: last,
            source: "CLI".into(),
        }
    }

    #[test]
    fn 專案說明() {
        let now = dt(11, 12, 10, 0);
        let mut p = ProjectItem::new(&detected("app", true, None), None);
        p.describe(None, now);
        assert_eq!(("Git · 還沒有 Claude session".to_owned(), None), (p.detail.clone(), p.dot));
        let mut p = ProjectItem::new(&detected("app", false, Some(dt(11, 12, 8, 0))), None);
        p.describe(None, now);
        assert_eq!(("資料夾 · CLI · 2 小時前".to_owned(), Some(ColorKey::Run)), (p.detail.clone(), p.dot));
        p.describe(Some(&activity(false, dt(11, 12, 9, 30), Some("F2P-3"))), now);
        assert_eq!("資料夾 · CLI · 30 分鐘前 · 任務 F2P-3", p.detail);
        p.describe(Some(&activity(true, dt(11, 12, 9, 59), Some("F2P-3"))), now);
        assert_eq!((ColorKey::Claude, Some(ColorKey::Claude)), (p.detail_color, p.dot));
        p.enabled = false;
        p.describe(None, now);
        assert_eq!(("資料夾 · 已停用".to_owned(), None), (p.detail.clone(), p.dot));
    }

    #[test]
    fn 主資料夾_hooks_與存檔() {
        let config = RepoConfig {
            path: "D:\\repo".into(),
            projects: vec![
                ProjectConfig { name: "APP".into(), enabled: false, auto_create: false },
                ProjectConfig { name: "gone".into(), enabled: true, auto_create: true },
            ],
            ..Default::default()
        };
        let mut item = RepoItem::new(&config, "D:\\repo".into());
        item.set_projects(&[detected("app", true, None), detected("web", true, None)], &config.projects);
        assert_eq!("repo", item.name());
        assert_eq!("裡面的專案（2）", item.projects_header());
        // 停用的專案不裝 hooks
        assert_eq!(vec!["D:\\repo".to_owned(), "D:\\repo\\web".to_owned()], item.hook_targets());
        item.set_hooks(0, 2);
        assert_eq!(
            ("Hooks 未安裝".to_owned(), "安裝 hooks", ColorKey::Muted),
            (item.hooks_text(), item.hooks_action(), item.hooks_color())
        );
        item.set_hooks(1, 2);
        assert_eq!(
            ("✓ Hooks 1/2".to_owned(), "安裝 hooks", ColorKey::Run),
            (item.hooks_text(), item.hooks_action(), item.hooks_color())
        );
        item.set_hooks(1, 1);
        assert_eq!(("✓ Hooks 已安裝".to_owned(), "移除 hooks"), (item.hooks_text(), item.hooks_action()));
        item.set_hooks(0, 0);
        assert_eq!("Hooks 未安裝", item.hooks_text());
        // 只存改過的（app 停用），加上沒偵測到的 gone
        let saved = item.to_config();
        assert_eq!(vec!["app", "gone"], saved.projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>());
        item.include_subprojects = false;
        assert_eq!("裡面的專案（未偵測）", item.projects_header());
        assert_eq!(vec!["D:\\repo".to_owned()], item.hook_targets());
    }

    #[test]
    fn eip_同步狀態文字() {
        let mut s = hb_eip::EipSyncState::default();
        assert_eq!("還沒同步過", eip_sync_text(&s, false));
        assert_eq!("同步中…", eip_sync_text(&s, true));
        s.last_result = "失敗：連不上".into();
        assert_eq!("失敗：連不上", eip_sync_text(&s, false));
        s.morning_at = Some(dt(11, 12, 8, 45));
        s.morning_result = "沒有請假".into();
        s.last_sync_at = Some(dt(11, 12, 15, 0));
        s.last_result = "登記 1 筆".into();
        assert_eq!("上班前 11/12（週四）08:45：沒有請假\n手動 11/12（週四）15:00：登記 1 筆", eip_sync_text(&s, false));
        s.morning_at = None;
        assert_eq!("上次同步 11/12（週四）15:00：登記 1 筆", eip_sync_text(&s, false));
    }

    #[test]
    fn eip_假單摘要() {
        assert_eq!("今年讀到 0 筆假單", leaves_summary(&[]));
        let leave = |start: NaiveDateTime, end: NaiveDateTime| hb_eip::EipLeave {
            no: "1".into(),
            person_no: "p".into(),
            kind: "請假".into(),
            r#type: "特休".into(),
            reason: String::new(),
            start,
            end,
            status: "核准".into(),
            id: None,
            subtotal: String::new(),
        };
        let text =
            leaves_summary(&[leave(dt(11, 12, 9, 0), dt(11, 12, 18, 0)), leave(dt(11, 13, 9, 0), dt(11, 14, 18, 0))]);
        assert_eq!(
            "今年讀到 2 筆假單：11/12（週四） 09:00–18:00 特休（核准）、11/13（週五） 09:00–11/14 18:00 特休（核准）",
            text
        );
    }
}
