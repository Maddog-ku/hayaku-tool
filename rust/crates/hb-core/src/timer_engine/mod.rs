//! 對應 C# `Core/TimerEngine.cs`（以及同一個 partial class 的 Eip、Extras、NewWork、Pins、Reviews、Slack、Testing、Work，各一個子模組）。
//!
//! 工具的大腦，全部在 UI 執行緒上跑：定期同步 Hayaku 計時 → 依班表自動上下班 → 依 Claude 活動自動計時。
//! 所有會打 API 的流程都經過同一個 gate，避免同時開始／暫停同一個任務。
//!
//! Rust 版的對應：
//! - `TimerEngine` 包一個 `Rc<Inner>`，`clone()` 很便宜，UI 到處拿；不是 `Send`，只能在 UI 執行緒用。
//! - C# 的 `DispatcherTimer` 用 `slint::Timer`；`async` 方法是 `async fn`，由呼叫端用 `slint::spawn_local` 跑；
//!   C# 裡 fire-and-forget 的（`_ = XxxAsync()`）經過 [`TimerEngine::spawn`]（預設 `slint::spawn_local`）。
//! - C# 的 `event` 都在 [`EngineEvents`]（`engine.events().changed.subscribe(...)`）。
//! - C# 的 `IReadOnlyList<T>` 屬性回傳 `Rc<Vec<T>>`（整份換掉的快照，跟 C# 一樣）。
//! - `Config`、`State` 跟 App 共用 `Rc<RefCell<…>>`。

mod eip;
mod events;
mod extras;
mod new_work;
mod pins;
mod reviews;
mod slack;
mod testing;
mod work;

pub use eip::EipLeaveRecord;
pub use events::{Event, HandlerId};
pub use extras::{AsyncAction, ToastAction, ToastRequest, WorkspaceArg};
pub use new_work::{NewWorkSuggestion, TreeLoad};
pub use slack::SlackLink;
pub use testing::TestActionRequest;
pub use work::ClaudeDigest;

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime, TimeDelta};
use hb_api::{
    ChangeRequest, DateTimeOffset, DayPhaseKind, DaySchedule, HayakuApi, KeyFinder, MeetingItem, NotificationItem,
    PinItem, ProjectDetail, ProjectInfo, ReminderItem, TaskItem, TaskSummary, TeamMember, TimeEntry, TodoItem,
    UserInfo, WorkMath, WorkSegment,
};
use hb_backend::auth::OAuthClient;
use hb_backend::config::{AppConfig, AppState, CiMap};
use hb_claude::ClaudeMonitor;
use indexmap::IndexMap;
use tokio::sync::Semaphore;

use crate::ci_set::{CiSet, eq_ic};
use crate::error::{CoreError, is_mcp};
use crate::file_reader::FileReader;
use crate::sync_policy::SyncPolicy;
use crate::time::{self, MIN};

/// fire-and-forget 的 future。
pub type LocalFuture = Pin<Box<dyn Future<Output = ()>>>;

/// 把 future 丟到 UI 執行緒跑（預設 `slint::spawn_local`）。
pub type Spawner = Rc<dyn Fn(LocalFuture)>;

const END_ACTION_WINDOW: TimeDelta = TimeDelta::minutes(30);

// 背景同步的頻率：每次輪詢只問「哪些在計時」，其他資料各有自己的間隔（計時列表見 SyncPolicy）
const SCHEDULE_INTERVAL: TimeDelta = TimeDelta::minutes(30);
const TASKS_INTERVAL: TimeDelta = TimeDelta::minutes(30);

const CLAUDE_ACTIVE_INTERVAL: Duration = Duration::from_secs(15);
const CLAUDE_IDLE_INTERVAL: Duration = Duration::from_secs(60);
const HOOK_DEBOUNCE: Duration = Duration::from_secs(1);

/// TimerEngine 的所有事件（C# 的 `event`）。
#[derive(Default)]
pub struct EngineEvents {
    /// 狀態變了，畫面要更新。
    pub changed: Event<()>,
    /// 簡短提示（App 用 `ToastRequest("HayakuBar", message)` 顯示）。
    pub notify: Event<String>,
    /// 跳出提示（右下角）。
    pub toast: Event<ToastRequest>,
    /// 要關掉某個標記的提示（在 Slack 上處理掉了）。
    pub toast_dismiss_requested: Event<String>,
    /// 要打開「工時紀錄」視窗（例如下班時還沒滿應出勤）。
    pub time_entries_requested: Event<NaiveDate>,
    /// 要開工作台的某一頁（頁名, 參數）。
    pub workspace_requested: Event<(String, Option<WorkspaceArg>)>,
    /// 使用者按了「建立任務」：開建立任務的小視窗。
    pub create_task_requested: Event<NewWorkSuggestion>,
    /// 「有新工作」的提示跳出、建立任務要用的資料也讀好了：App 先在背景把小視窗建好。
    pub new_work_ready: Event<NewWorkSuggestion>,
    /// 使用者按了「選既有任務」：開任務選擇視窗。
    pub pick_task_requested: Event<NewWorkSuggestion>,
    /// 使用者在審核提示按了「駁回」：開填說明的小視窗。
    pub reject_requested: Event<ChangeRequest>,
    /// 要開小視窗處理測試結果（開始修填預估工時、退回填說明、加入測試結果）。
    pub test_action_requested: Event<TestActionRequest>,
    /// 套用了新的設定（`apply_config`）。
    pub config_applied: Event<()>,
}

/// 會變的狀態（C# 的欄位與 private set 的屬性）。借用時間都很短，不會跨 await。
struct S {
    user: Option<UserInfo>,
    schedule: Option<DaySchedule>,
    entries: Rc<Vec<TimeEntry>>,
    my_tasks: Rc<Vec<TaskItem>>,
    logged_in: bool,
    logging_in: bool,
    error: Option<String>,

    suppressed: CiSet,
    self_paused: CiSet,
    prev_running: CiSet,
    claude_desired: CiSet,
    schedule_fetched: NaiveDateTime,
    tasks_fetched: NaiveDateTime,
    entries_fetched: NaiveDateTime,
    entries_day: NaiveDate,
    failures: i32,
    retry_at: NaiveDateTime,
    initial_sync_done: bool,
    syncing: bool,
    last_synced: Option<NaiveDateTime>,
    auto_sync_paused: bool,
    last_polled: NaiveDateTime,
    sync_failed: bool,

    // Eip
    eip_years: HashMap<i32, Rc<Vec<EipLeaveRecord>>>,
    eip_retry_at: NaiveDateTime,
    eip_syncing: bool,

    // Extras
    notifications_fetched: NaiveDateTime,
    meetings_fetched: NaiveDateTime,
    panel_data_fetched: NaiveDateTime,
    open_refreshed: NaiveDateTime,
    alerted: HashSet<String>,
    started_at: DateTimeOffset,
    meetings: Rc<Vec<MeetingItem>>,
    todos: Rc<Vec<TodoItem>>,
    reminders: Rc<Vec<ReminderItem>>,
    notifications: Rc<Vec<NotificationItem>>,
    discussions: Rc<Vec<NotificationItem>>,
    discussions_fetched: NaiveDateTime,
    discussions_query: String,

    // Pins
    reviews_fetched: NaiveDateTime,
    project_load_failed: CiSet,
    projects: Rc<Vec<ProjectInfo>>,
    pins: Rc<Vec<PinItem>>,
    pending_reviews: Rc<Vec<ChangeRequest>>,

    // Work
    project_details: CiMap<Rc<ProjectDetail>>,
    task_lists: Rc<IndexMap<String, Rc<Vec<TaskSummary>>>>,
    all_tasks: Rc<Vec<TaskSummary>>,
    team: Rc<Vec<TeamMember>>,
    parent_titles: CiMap<Option<String>>,
}

impl S {
    fn new() -> Self {
        Self {
            user: None,
            schedule: None,
            entries: Rc::default(),
            my_tasks: Rc::default(),
            logged_in: false,
            logging_in: false,
            error: None,
            suppressed: CiSet::new(),
            self_paused: CiSet::new(),
            prev_running: CiSet::new(),
            claude_desired: CiSet::new(),
            schedule_fetched: MIN,
            tasks_fetched: MIN,
            entries_fetched: MIN,
            entries_day: NaiveDate::MIN,
            failures: 0,
            retry_at: MIN,
            initial_sync_done: false,
            syncing: false,
            last_synced: None,
            auto_sync_paused: false,
            last_polled: MIN,
            sync_failed: false,
            eip_years: HashMap::new(),
            eip_retry_at: MIN,
            eip_syncing: false,
            notifications_fetched: MIN,
            meetings_fetched: MIN,
            panel_data_fetched: MIN,
            open_refreshed: MIN,
            alerted: HashSet::new(),
            started_at: time::now_offset(),
            meetings: Rc::default(),
            todos: Rc::default(),
            reminders: Rc::default(),
            notifications: Rc::default(),
            discussions: Rc::default(),
            discussions_fetched: MIN,
            discussions_query: String::new(),
            reviews_fetched: MIN,
            project_load_failed: CiSet::new(),
            projects: Rc::default(),
            pins: Rc::default(),
            pending_reviews: Rc::default(),
            project_details: CiMap::new(),
            task_lists: Rc::default(),
            all_tasks: Rc::default(),
            team: Rc::default(),
            parent_titles: CiMap::new(),
        }
    }
}

pub(crate) struct Inner {
    config: Rc<RefCell<AppConfig>>,
    state: Rc<RefCell<AppState>>,
    auth: OAuthClient,
    api: Arc<HayakuApi>,
    claude: Rc<RefCell<ClaudeMonitor>>,
    /// 認得的專案代碼（Claude 的任務編號只認這些）；ClaudeMonitor 的 is_known_key 也用，所以是 Arc<Mutex>。
    known_projects: Arc<Mutex<CiSet>>,
    gate: Semaphore,
    poll: slint::Timer,
    poll_interval: Cell<Duration>,
    claude_timer: slint::Timer,
    claude_interval: Cell<Duration>,
    hook_debounce: slint::Timer,
    s: RefCell<S>,
    events: EngineEvents,
    ui_visible: RefCell<Rc<dyn Fn() -> bool>>,
    spawner: RefCell<Spawner>,

    // Eip
    eip_state: RefCell<hb_eip::EipSyncState>,
    eip: RefCell<Option<Rc<hb_eip::EipClient>>>,
    eip_gate: Semaphore,

    // NewWork
    tree_cache: RefCell<CiMap<(NaiveDateTime, TreeLoad)>>,

    // Slack、Testing
    slack: RefCell<Option<Rc<dyn SlackLink>>>,
    files: RefCell<Option<Rc<FileReader>>>,
}

/// 工具的大腦（C# `TimerEngine`）。`clone()` 很便宜（共用同一份狀態）。
#[derive(Clone)]
pub struct TimerEngine(Rc<Inner>);

/// 一個班表時間點（上班開始 S0、結束 E0…）。
#[derive(Debug, Clone)]
struct Boundary {
    at: NaiveDateTime,
    is_start: bool,
    segment: usize,
    id: String,
}

fn poll_duration(config: &AppConfig) -> Duration {
    Duration::from_secs(config.poll_seconds.max(10) as u64)
}

fn to_delta(d: Duration) -> TimeDelta {
    TimeDelta::from_std(d).unwrap_or(TimeDelta::MAX)
}

impl TimerEngine {
    pub fn new(
        config: Rc<RefCell<AppConfig>>,
        state: Rc<RefCell<AppState>>,
        auth: OAuthClient,
        api: Arc<HayakuApi>,
        claude: ClaudeMonitor,
    ) -> Self {
        let known_projects = Arc::new(Mutex::new(CiSet::new()));
        let mut claude = claude;
        let known = known_projects.clone();
        claude.is_known_key = Arc::new(move |key: &str| {
            known.lock().map(|k| k.is_empty() || k.contains(KeyFinder::project_of(key))).unwrap_or(true)
        });
        let poll_interval = poll_duration(&config.borrow());
        let default_spawner: Spawner = Rc::new(|fut: LocalFuture| {
            if let Err(err) = slint::spawn_local(fut) {
                log::error!("無法在 UI 執行緒執行背景工作: {err}");
            }
        });
        TimerEngine(Rc::new(Inner {
            config,
            state,
            auth,
            api,
            claude: Rc::new(RefCell::new(claude)),
            known_projects,
            gate: Semaphore::new(1),
            poll: slint::Timer::default(),
            poll_interval: Cell::new(poll_interval),
            claude_timer: slint::Timer::default(),
            claude_interval: Cell::new(CLAUDE_ACTIVE_INTERVAL),
            hook_debounce: slint::Timer::default(),
            s: RefCell::new(S::new()),
            events: EngineEvents::default(),
            ui_visible: RefCell::new(Rc::new(|| true)),
            spawner: RefCell::new(default_spawner),
            eip_state: RefCell::new(hb_eip::EipSyncState::load()),
            eip: RefCell::new(None),
            eip_gate: Semaphore::new(1),
            tree_cache: RefCell::new(CiMap::new()),
            slack: RefCell::new(None),
            files: RefCell::new(None),
        }))
    }

    fn weak(&self) -> Weak<Inner> {
        Rc::downgrade(&self.0)
    }

    fn from_weak(weak: &Weak<Inner>) -> Option<TimerEngine> {
        weak.upgrade().map(TimerEngine)
    }

    fn s(&self) -> Ref<'_, S> {
        self.0.s.borrow()
    }

    fn s_mut(&self) -> RefMut<'_, S> {
        self.0.s.borrow_mut()
    }

    /// 換掉 fire-and-forget 用的執行方式（測試用；預設 `slint::spawn_local`）。
    pub fn set_spawner(&self, spawner: Spawner) {
        *self.0.spawner.borrow_mut() = spawner;
    }

    /// C# 的 `_ = XxxAsync()`：丟到 UI 執行緒跑、不等結果。
    pub fn spawn(&self, fut: impl Future<Output = ()> + 'static) {
        let spawner = self.0.spawner.borrow().clone();
        spawner(Box::pin(fut));
    }

    // ───────────── 屬性 ─────────────

    pub fn events(&self) -> &EngineEvents {
        &self.0.events
    }

    pub fn user(&self) -> Option<UserInfo> {
        self.s().user.clone()
    }

    pub fn schedule(&self) -> Option<DaySchedule> {
        self.s().schedule.clone()
    }

    pub fn entries(&self) -> Rc<Vec<TimeEntry>> {
        self.s().entries.clone()
    }

    pub fn my_tasks(&self) -> Rc<Vec<TaskItem>> {
        self.s().my_tasks.clone()
    }

    pub fn logged_in(&self) -> bool {
        self.s().logged_in
    }

    pub fn logging_in(&self) -> bool {
        self.s().logging_in
    }

    pub fn error(&self) -> Option<String> {
        self.s().error.clone()
    }

    pub fn config(&self) -> Ref<'_, AppConfig> {
        self.0.config.borrow()
    }

    /// 改設定（改完呼叫 [`apply_config`](Self::apply_config) 存檔並套用）。
    pub fn config_mut(&self) -> RefMut<'_, AppConfig> {
        self.0.config.borrow_mut()
    }

    pub fn config_rc(&self) -> Rc<RefCell<AppConfig>> {
        self.0.config.clone()
    }

    pub fn state(&self) -> Ref<'_, AppState> {
        self.0.state.borrow()
    }

    pub fn state_mut(&self) -> RefMut<'_, AppState> {
        self.0.state.borrow_mut()
    }

    pub fn state_rc(&self) -> Rc<RefCell<AppState>> {
        self.0.state.clone()
    }

    /// Claude 監看（C# `engine.Claude`）。
    pub fn claude(&self) -> Ref<'_, ClaudeMonitor> {
        self.0.claude.borrow()
    }

    pub fn claude_mut(&self) -> RefMut<'_, ClaudeMonitor> {
        self.0.claude.borrow_mut()
    }

    pub fn auth(&self) -> &OAuthClient {
        &self.0.auth
    }

    /// 從畫面上跳一則簡短提示（例如刪除任務後）。
    pub fn show_notice(&self, text: &str) {
        self.notify(text.to_owned());
    }

    fn notify(&self, text: String) {
        self.0.events.notify.invoke(&text);
    }

    fn toast(&self, request: ToastRequest) {
        self.0.events.toast.invoke(&request);
    }

    pub fn running_keys(&self) -> CiSet {
        self.s().entries.iter().filter(|e| e.running).map(|e| e.task.clone()).collect()
    }

    pub fn is_claude_owned(&self, key: &str) -> bool {
        self.0.state.borrow().claude_owned.iter().any(|k| eq_ic(k, key))
    }

    fn add_known_project(&self, key: &str) {
        if let Ok(mut known) = self.0.known_projects.lock() {
            known.insert(KeyFinder::project_of(key));
        }
    }

    fn is_known_project(&self, project: &str) -> bool {
        self.0.known_projects.lock().is_ok_and(|k| k.contains(project))
    }

    /// 啟動定時器，有登入資料時做第一次同步。
    pub async fn start(&self) {
        let weak = self.weak();
        self.0.poll.start(slint::TimerMode::Repeated, self.0.poll_interval.get(), move || {
            if let Some(e) = Self::from_weak(&weak) {
                let e2 = e.clone();
                e.spawn(async move { e2.auto_refresh(false).await });
            }
        });
        self.start_claude_timer();
        if !self.0.auth.has_session() {
            self.s_mut().initial_sync_done = true;
            self.raise();
            return;
        }
        if self.use_cached_off_day(time::today()) {
            // 今天不用上班（請假、放假）：不問 Hayaku，用上次記下的帳號顯示
            let cached = self.cached_user();
            {
                let mut s = self.s_mut();
                if s.user.is_none() {
                    s.user = cached;
                }
                s.logged_in = true;
                s.initial_sync_done = true;
            }
            self.raise();
            return;
        }
        self.raise(); // 讓畫面先顯示「同步中」
        self.refresh(true).await;
        self.s_mut().initial_sync_done = true;
        self.raise();
    }

    fn start_claude_timer(&self) {
        let weak = self.weak();
        self.0.claude_timer.start(slint::TimerMode::Repeated, self.0.claude_interval.get(), move || {
            if let Some(e) = Self::from_weak(&weak) {
                e.check_alerts();
                let e2 = e.clone();
                e.spawn(async move {
                    e2.claude_tick(false).await;
                    e2.after_work_tick().await;
                });
            }
        });
    }

    /// 有登入資料、第一次同步還沒完成：畫面顯示「同步中，請稍候」。
    pub fn initial_syncing(&self) -> bool {
        !self.s().initial_sync_done && self.0.auth.has_session()
    }

    /// 正在跟 Hayaku 同步。
    pub fn syncing(&self) -> bool {
        self.s().syncing
    }

    /// 上次同步成功的時間。
    pub fn last_synced(&self) -> Option<NaiveDateTime> {
        self.s().last_synced
    }

    /// 設定改了：存檔、套用同步頻率，通知 App（ConfigApplied）。
    pub fn apply_config(&self) {
        let interval = poll_duration(&self.0.config.borrow());
        if interval != self.0.poll_interval.get() {
            self.0.poll_interval.set(interval);
            if self.0.poll.running() {
                self.0.poll.set_interval(interval);
            }
        }
        self.0.config.borrow().save();
        if !self.0.config.borrow().sync_only_during_work {
            self.s_mut().auto_sync_paused = false;
        }
        self.0.events.config_applied.invoke(&());
        let e = self.clone();
        self.spawn(async move { e.claude_tick(false).await });
    }

    // ───────────── 登入 ─────────────

    pub async fn login(&self) {
        {
            let mut s = self.s_mut();
            if s.logging_in {
                return;
            }
            s.logging_in = true;
            s.error = None;
        }
        self.raise();
        match self.0.auth.login().await {
            Ok(()) => {
                let mut s = self.s_mut();
                s.user = None;
                s.schedule_fetched = MIN;
                s.tasks_fetched = MIN;
                s.logged_in = true;
            }
            Err(err) => {
                log::error!("登入失敗: {err}");
                self.s_mut().error = Some(format!("登入失敗：{}", err.message()));
            }
        }
        self.s_mut().logging_in = false;
        self.raise();
        if self.logged_in() {
            self.refresh(true).await;
            let name = self.s().user.as_ref().map(|u| u.display_name.clone()).unwrap_or_default();
            self.notify(format!("已登入 {name}"));
        }
    }

    pub async fn logout(&self) {
        self.0.auth.logout().await;
        {
            let mut s = self.s_mut();
            s.logged_in = false;
            s.user = None;
            s.schedule = None;
            s.entries = Rc::default();
            s.my_tasks = Rc::default();
        }
        self.raise();
    }

    // ───────────── 對外動作 ─────────────

    /// 同步一輪；user_action：使用者要求（等前一輪做完），否則有別的在跑就略過。
    pub async fn refresh(&self, user_action: bool) {
        self.round(false, user_action).await;
    }

    /// 同步一輪：同步 → 依班表自動上下班 → Claude 自動計時。background：背景輪詢，只抓有變化或到期的資料。
    async fn round(&self, background: bool, wait: bool) {
        self.guarded(
            async {
                self.refresh_core(background).await?;
                self.check_meeting_left().await?;
                self.automation().await?;
                self.claude_core().await
            },
            wait,
        )
        .await;
    }

    /// 面板或工作台開著（App 設定）：開著時照設定的頻率同步，沒開又沒 Claude 在動時放慢。
    pub fn set_ui_visible(&self, f: impl Fn() -> bool + 'static) {
        *self.0.ui_visible.borrow_mut() = Rc::new(f);
    }

    fn ui_visible(&self) -> bool {
        let f = self.0.ui_visible.borrow().clone();
        f()
    }

    fn claude_active(&self) -> bool {
        self.0.claude.borrow().repos().any(|r| r.active)
    }

    /// 定時、喚醒、解鎖時的自動同步（鎖定畫面時照常同步）。不打 Hayaku 的情況：
    /// 設定「只在上班時間自動同步」時的下班、休息、放假；連續失敗時退避等待中；
    /// 面板、工作台都沒開、Claude 也沒在動時，兩次定時才同步一次。
    /// force：喚醒、解鎖：不管放慢，照常同步。
    pub async fn auto_refresh(&self, force: bool) {
        if !self.0.auth.has_session() || self.logging_in() {
            return;
        }
        let now = time::now();
        let paused = !self.should_auto_sync(now);
        if paused != self.s().auto_sync_paused {
            self.s_mut().auto_sync_paused = paused;
            log::info!("{}", if paused { "不在上班時間，暫停自動同步" } else { "恢復自動同步" });
            self.raise();
        }
        if paused || now < self.s().retry_at {
            return;
        }
        if !force {
            let (visible, active, last_polled) = (self.ui_visible(), self.claude_active(), self.s().last_polled);
            if !SyncPolicy::should_poll(visible, active, now, last_polled, to_delta(self.0.poll_interval.get())) {
                return;
            }
        }
        self.s_mut().last_polled = now;

        let before = self.s().last_synced;
        self.round(true, false).await;
        let mut s = self.s_mut();
        if s.last_synced != before {
            if s.failures > 0 {
                log::info!("同步恢復正常");
            }
            s.failures = 0;
            s.retry_at = MIN;
        } else if s.sync_failed {
            s.failures += 1;
            let wait = SyncPolicy::backoff_delay(s.failures);
            s.retry_at = time::now() + wait;
            log::info!("同步失敗 {} 次，{} 分鐘後再自動同步", s.failures, wait.num_minutes());
        }
    }

    /// 目前不在上班時間，自動同步暫停中（打開面板、按重新整理仍會同步）。
    pub fn auto_sync_paused(&self) -> bool {
        self.s().auto_sync_paused
    }

    fn should_auto_sync(&self, now: NaiveDateTime) -> bool {
        if !self.0.config.borrow().sync_only_during_work {
            return true;
        }
        self.use_cached_off_day(now.date()); // 跨日後：快取知道今天不用上班就不用問 Hayaku
        let s = self.s();
        let Some(schedule) = s.schedule.as_ref().filter(|sc| sc.date == now.date()) else {
            return true; // 還不知道今天的班表（跨日後要先讀一次）
        };
        if s.entries.iter().any(|e| e.running) {
            return true; // 有計時在跑：加班中，照常同步
        }
        let schedule = schedule.clone();
        drop(s);
        if self.boundary_due(now) {
            return true;
        }
        !self.0.state.borrow().clocked_out && WorkMath::phase(Some(&schedule), now, false).kind == DayPhaseKind::Working
    }

    /// 上班、休息、下班的自動動作到時間了、還沒做。
    fn boundary_due(&self, now: NaiveDateTime) -> bool {
        let state = self.0.state.borrow();
        if !self.0.config.borrow().auto_schedule || state.clocked_out {
            return false;
        }
        let s = self.s();
        let Some(schedule) = s.schedule.as_ref().filter(|sc| sc.date == now.date()) else {
            return false;
        };
        boundaries(&schedule.working, now.date())
            .iter()
            .any(|b| b.at <= now && !state.handled_boundaries.contains(&b.id))
    }

    pub async fn start_task(&self, key: &str) {
        self.guarded(
            async {
                let key = key.trim().to_uppercase();
                self.0.api.start_timer(&key).await?;
                self.s_mut().suppressed.remove(&key);
                self.remove_claude_owned(&key); // 使用者手動開始的，不再標成 Claude 的
                self.add_known_project(&key);
                self.refresh_core(false).await
            },
            true,
        )
        .await;
    }

    pub async fn pause_task(&self, key: &str) {
        self.guarded(
            async {
                self.s_mut().self_paused.insert(key);
                self.0.api.pause_timer(Some(key)).await?;
                {
                    let mut s = self.s_mut();
                    if s.claude_desired.contains(key) {
                        s.suppressed.insert(key); // Claude 還在動也不要馬上又開始，等它停下來一陣子再說
                    }
                }
                self.remove_claude_owned(key);
                self.refresh_core(false).await?;
                self.check_meeting_left().await // 在面板上停掉會議的計時：恢復加入會議時暫停的
            },
            true,
        )
        .await;
    }

    /// 可恢復的任務：上次記下的那組；沒有的話用今天被午休自動暫停（BREAK）的。
    pub fn resume_candidates(&self) -> Vec<String> {
        let resume = self.0.state.borrow().resume_set.clone();
        if !resume.is_empty() {
            return resume;
        }
        let set: CiSet = self
            .s()
            .entries
            .iter()
            .filter(|e| e.stop_reason.as_deref() == Some("BREAK"))
            .map(|e| e.task.clone())
            .collect();
        set.to_vec()
    }

    pub async fn clock_in(&self, tasks: Option<Vec<String>>) {
        self.guarded(
            async {
                self.0.state.borrow_mut().clocked_out = false;
                self.mark_past_boundaries_handled(time::now());
                {
                    let mut state = self.0.state.borrow_mut();
                    if let Some(tasks) = tasks.as_ref().filter(|t| !t.is_empty()) {
                        let mut list: Vec<String> = Vec::new();
                        for t in tasks.iter().map(|t| t.to_uppercase()) {
                            if !list.contains(&t) {
                                list.push(t);
                            }
                        }
                        state.resume_set = list;
                    }
                    state.save();
                }
                self.refresh_core(false).await?;
                self.resume_core("上班").await
            },
            true,
        )
        .await;
    }

    pub async fn clock_out(&self) {
        self.guarded(
            async {
                self.refresh_core(false).await?;
                let running = self.running_keys().sorted();
                {
                    let mut state = self.0.state.borrow_mut();
                    if !running.is_empty() {
                        state.resume_set = running.clone();
                    }
                    state.clocked_out = true;
                    state.save();
                }
                if !running.is_empty() {
                    self.pause_all_core().await?;
                }
                let now = time::now();
                let worked =
                    WorkMath::union_minutes(self.entries().iter(), now.date().and_time(chrono::NaiveTime::MIN), now);
                let expected = self.s().schedule.as_ref().map_or(0, |s| s.expected_minutes);
                self.after_clock_out(running, worked, expected);
                Ok(())
            },
            true,
        )
        .await;
    }

    /// 為 repo 目前的 Claude session 指定任務（只對這個 session 有效，沒有預設任務）。
    pub async fn pick_task_for_repo(&self, unit_path: &str, key: &str) {
        let key = key.to_uppercase();
        let sid = self.0.claude.borrow().repos().find(|r| eq_ic(&r.path, unit_path)).and_then(|r| r.session_id.clone());
        if let Some(sid) = sid {
            self.assign_session_task(&sid, &key, false).await;
        }
    }

    /// Claude Code hooks 的事件（本機 HTTP 收到的 JSON）：有變化就在 1 秒後掃一次。
    pub fn on_hook(&self, payload: &serde_json::Value) {
        if !self.0.claude.borrow_mut().on_hook(payload, time::now()) {
            return;
        }
        let weak = self.weak();
        self.0.hook_debounce.start(slint::TimerMode::SingleShot, HOOK_DEBOUNCE, move || {
            if let Some(e) = Self::from_weak(&weak) {
                let e2 = e.clone();
                e.spawn(async move { e2.claude_tick(false).await });
            }
        });
    }

    // ───────────── 內部流程 ─────────────

    async fn claude_tick(&self, wait: bool) {
        if self.logged_in() {
            self.guarded(self.claude_core(), wait).await;
        }
    }

    /// 同一時間只跑一個會打 API 的流程；wait=false 時有別的在跑就略過。錯誤寫進 `error`（畫面顯示）。
    async fn guarded(&self, work: impl Future<Output = Result<(), CoreError>>, wait: bool) {
        let permit = if wait {
            match self.0.gate.acquire().await {
                Ok(p) => p,
                Err(_) => return,
            }
        } else {
            match self.0.gate.try_acquire() {
                Ok(p) => p,
                Err(_) => return,
            }
        };
        if let Err(err) = work.await {
            if err.is_auth_required() {
                let mut s = self.s_mut();
                s.logged_in = false;
                s.error = Some(err.message().to_owned());
            } else {
                log::error!("執行失敗: {err}");
                let message = if err.is_transient() {
                    err.message().to_owned()
                } else {
                    format!("發生錯誤：{}", err.message())
                };
                self.s_mut().error = Some(message);
            }
        }
        drop(permit);
        self.raise();
    }

    /// background：背景輪詢（今天的計時列表只在計時有變化或每 5 分鐘才重抓）；否則是動作後或使用者要求的同步，一定重抓。
    async fn refresh_core(&self, background: bool) -> Result<(), CoreError> {
        self.s_mut().syncing = true;
        self.raise();
        let result = Box::pin(self.sync(background)).await;
        let mut s = self.s_mut();
        match &result {
            Ok(()) => {
                s.last_synced = Some(time::now());
                s.sync_failed = false;
            }
            Err(_) => s.sync_failed = true,
        }
        s.syncing = false;
        result
    }

    async fn sync(&self, background: bool) -> Result<(), CoreError> {
        let now = time::now();
        let today = now.date();
        {
            let mut state = self.0.state.borrow_mut();
            if state.roll_day(&time::ymd(today)) {
                self.s_mut().schedule_fetched = MIN;
                state.save();
            }
        }

        if self.s().user.is_none() {
            let user = self.0.api.who_am_i().await?;
            {
                let mut state = self.0.state.borrow_mut();
                if state.user_name.as_deref() != Some(user.username.as_str())
                    || state.user_display_name.as_deref() != Some(user.display_name.as_str())
                {
                    state.user_name = Some(user.username.clone());
                    state.user_display_name = Some(user.display_name.clone());
                    state.save();
                }
            }
            self.s_mut().user = Some(user);
        }
        let need_schedule = {
            let s = self.s();
            s.schedule.as_ref().is_none_or(|sc| sc.date != today) || now - s.schedule_fetched > SCHEDULE_INTERVAL
        };
        if need_schedule {
            let schedule = self.0.api.get_schedule(today).await?;
            let mut s = self.s_mut();
            s.schedule = Some(schedule);
            s.schedule_fetched = now;
        }
        if now - self.s().tasks_fetched > TASKS_INTERVAL {
            self.refresh_tasks().await?;
        }

        // 跨日還在跑的計時不一定出現在今天的列表，所以每次都問；正在計時的跟上次一樣就不用重抓整天的列表
        let running = self.0.api.running_timers().await?;
        let need_entries = {
            let s = self.s();
            SyncPolicy::need_entries(
                background,
                running.iter().map(|e| e.id.as_str()),
                s.entries.iter().filter(|e| e.running).map(|e| e.id.as_str()),
                today,
                s.entries_day,
                now,
                s.entries_fetched,
            )
        };
        if need_entries {
            let mut entries: IndexMap<String, TimeEntry> =
                self.0.api.list_time_entries(today, today).await?.into_iter().map(|e| (e.id.clone(), e)).collect();
            for e in running {
                entries.insert(e.id.clone(), e);
            }
            let mut list: Vec<TimeEntry> = entries.into_values().collect();
            list.sort_by_key(|e| e.started_at.map(|t| t.timestamp_micros()).unwrap_or(i64::MIN));
            let mut s = self.s_mut();
            s.entries = Rc::new(list);
            s.entries_fetched = now;
            s.entries_day = today;
        }
        for e in self.entries().iter() {
            self.add_known_project(&e.task);
        }

        {
            let mut s = self.s_mut();
            s.logged_in = true;
            s.error = None;
        }
        self.track_external_stops();
        self.refresh_extras_core(background).await
    }

    async fn refresh_tasks(&self) -> Result<(), CoreError> {
        match self.0.api.list_projects(true).await {
            Ok(projects) => {
                for p in &projects {
                    self.add_known_project(&p.key);
                }
                self.s_mut().projects = Rc::new(projects);
                self.refresh_my_tasks().await
            }
            Err(err) if is_mcp(&err) => {
                log::error!("讀取任務清單失敗: {err}");
                Ok(())
            }
            Err(err) => Err(err.into()),
        }
    }

    /// 不是這個工具停掉的計時（網頁上手動、伺服器自動暫停）：不再視為 Claude 的，Claude 也先別重開。
    fn track_external_stops(&self) {
        let running = self.running_keys();
        let (prev, self_paused) = {
            let s = self.s();
            (s.prev_running.to_vec(), s.self_paused.clone())
        };
        for key in prev {
            if running.contains(&key) || self_paused.contains(&key) {
                continue;
            }
            self.remove_claude_owned(&key);
            let mut s = self.s_mut();
            if s.claude_desired.contains(&key) {
                s.suppressed.insert(key.as_str());
            }
        }
        let mut s = self.s_mut();
        s.self_paused.clear();
        s.prev_running = running;
    }

    async fn automation(&self) -> Result<(), CoreError> {
        let schedule = {
            let s = self.s();
            match &s.schedule {
                Some(sc) if self.0.config.borrow().auto_schedule && !self.0.state.borrow().clocked_out => sc.clone(),
                _ => return Ok(()),
            }
        };
        let segs = schedule.working.clone();
        if segs.is_empty() {
            return Ok(());
        }

        let now = time::now();
        let day = now.date();
        let bounds = boundaries(&segs, day);
        let due: Vec<Boundary> = {
            let state = self.0.state.borrow();
            bounds.iter().filter(|b| b.at <= now && !state.handled_boundaries.contains(&b.id)).cloned().collect()
        };
        // 上班前那次 EIP 同步還沒做完：先等（同步完登記了請假會重抓班表）
        if !due.is_empty() && !self.morning_eip_pending(now) {
            // 睡眠或剛開機時可能一次錯過好幾個時間點，只執行最後一個
            {
                let mut state = self.0.state.borrow_mut();
                for b in &due {
                    state.handled_boundaries.push(b.id.clone());
                }
                state.save();
            }

            let last = &due[due.len() - 1];
            if last.is_start && now < day.and_time(segs[last.segment].end) {
                self.resume_core(if last.segment == 0 { "上班時間到" } else { "休息結束" }).await?;
            } else if !last.is_start && now - last.at < END_ACTION_WINDOW {
                let running = self.running_keys();
                if !running.is_empty() {
                    self.pause_all_core().await?;
                    self.notify(if last.segment == segs.len() - 1 {
                        format!("下班時間到，已停止 {} 個計時", running.len())
                    } else {
                        format!(
                            "休息時間，已暫停 {} 個計時，{} 會自動恢復",
                            running.len(),
                            segs[last.segment + 1].start.format("%H:%M")
                        )
                    });
                }
            }
        }

        // 上班時段內持續記下「正在做哪些」（含 Claude 自動開的），休息／下班後就恢復這組
        let in_segment =
            segs.iter().any(|s| day.and_time(s.start) <= now && now < day.and_time(s.end) - TimeDelta::minutes(1));
        if in_segment {
            let snapshot = self.running_keys().sorted();
            let mut state = self.0.state.borrow_mut();
            if snapshot != state.resume_set {
                state.resume_set = snapshot;
                state.save();
            }
        }
        Ok(())
    }

    async fn resume_core(&self, reason: &str) -> Result<(), CoreError> {
        let running = self.running_keys();
        let to_start: Vec<String> = self.resume_candidates().into_iter().filter(|k| !running.contains(k)).collect();
        if to_start.is_empty() {
            if running.is_empty() {
                self.notify(format!("{reason}，還沒有要計時的任務，點工作列小條選一個"));
            }
            return Ok(());
        }
        let mut started = Vec::new();
        for key in to_start {
            match self.0.api.start_timer(&key).await {
                Ok(()) => started.push(key),
                Err(err) if is_mcp(&err) => log::error!("開始 {key} 失敗: {err}"),
                Err(err) => return Err(err.into()),
            }
        }
        if !started.is_empty() {
            self.notify(format!("{reason}，已開始計時：{}", started.join("、")));
        }
        self.refresh_core(false).await
    }

    async fn pause_all_core(&self) -> Result<(), CoreError> {
        let pause = self.0.state.borrow().meeting_pause.clone();
        if let Some(pause) = pause {
            // 會議中到了休息、下班：之後恢復的是會議前在做的，不是會議
            let meeting_tasks: CiSet = pause.meeting_tasks.iter().cloned().collect();
            let mut set: CiSet = pause.tasks.iter().cloned().collect();
            set.extend(self.running_keys().iter().filter(|k| !meeting_tasks.contains(k)).map(str::to_owned));
            let mut state = self.0.state.borrow_mut();
            state.resume_set = set.sorted();
            state.meeting_pause = None;
            state.save();
        }
        let running = self.running_keys();
        self.s_mut().self_paused.extend(running.iter().map(str::to_owned));
        self.0.api.pause_timer(None).await?;
        self.refresh_core(false).await
    }

    async fn claude_core(&self) -> Result<(), CoreError> {
        let now = time::now();
        {
            let config = self.0.config.borrow();
            let state = self.0.state.borrow();
            let mut claude = self.0.claude.borrow_mut();
            claude.sync(&config.repos);
            claude.scan();
            claude.resolve(now, config.claude_idle_minutes, &state.session_tasks);
        }
        self.learn_folder_projects();
        self.check_new_work(now);

        let mut desired = CiSet::new();
        let (enabled, only_work) = {
            let c = self.0.config.borrow();
            (c.claude_enabled, c.claude_only_during_work)
        };
        if enabled && self.logged_in() && (!only_work || self.is_working(now)) {
            for repo in self.0.claude.borrow().repos() {
                if repo.active
                    && let Some(task) = &repo.resolved_task
                {
                    desired.insert(task.as_str());
                }
            }
        }
        {
            let mut s = self.s_mut();
            s.claude_desired = desired.clone();
            s.suppressed.retain(|k| desired.contains(k));
        }

        let running = self.running_keys();
        let mut changed = false;
        for key in desired.iter() {
            if running.contains(key)
                || self.s().suppressed.contains(key)
                || self.0.state.borrow().meeting_pause.is_some()
            {
                continue; // 會議中先不自動開始，離開會議再說
            }
            match self.0.api.start_timer(key).await {
                Ok(()) => {
                    if !self.is_claude_owned(key) {
                        self.0.state.borrow_mut().claude_owned.push(key.to_owned());
                    }
                    log::info!("Claude 活動，開始計時 {key}");
                    changed = true;
                }
                Err(err) if is_mcp(&err) => {
                    log::error!("Claude 開始計時 {key} 失敗: {err}");
                    let mut s = self.s_mut();
                    s.suppressed.insert(key);
                    s.error = Some(format!("Claude 無法開始 {key}：{err}"));
                }
                Err(err) => return Err(err.into()),
            }
        }
        // 上班中或 Claude 正在動：15 秒掃一次；其他時候 60 秒（hooks 有事件時仍會馬上反應）
        let interval =
            if self.is_working(now) || self.claude_active() { CLAUDE_ACTIVE_INTERVAL } else { CLAUDE_IDLE_INTERVAL };
        if self.0.claude_interval.get() != interval {
            self.0.claude_interval.set(interval);
            if self.0.claude_timer.running() {
                self.0.claude_timer.set_interval(interval);
            }
        }

        // Claude 開始的計時不會因為閒置而暫停，會一直算到休息、下班或手動暫停
        if changed {
            self.0.state.borrow().save();
            self.refresh_core(false).await?;
        }
        Ok(())
    }

    /// 面板用的上班狀態：有計時在跑，或在班表的上班／休息時段內且今天沒按過下班。
    /// 上班狀態只顯示「下班」按鈕，否則只顯示「上班」。
    pub fn on_duty(&self) -> bool {
        let s = self.s();
        s.entries.iter().any(|e| e.running)
            || matches!(
                WorkMath::phase(s.schedule.as_ref(), time::now(), self.0.state.borrow().clocked_out).kind,
                DayPhaseKind::Working | DayPhaseKind::Break
            )
    }

    /// 「上班中」：在班表時段內（且今天沒手動下班），或有計時在跑（加班）。
    fn is_working(&self, now: NaiveDateTime) -> bool {
        let s = self.s();
        if s.entries.iter().any(|e| e.running) {
            return true;
        }
        !self.0.state.borrow().clocked_out
            && WorkMath::phase(s.schedule.as_ref(), now, false).kind == DayPhaseKind::Working
    }

    fn mark_past_boundaries_handled(&self, now: NaiveDateTime) {
        let Some(schedule) = self.schedule() else {
            return;
        };
        let mut state = self.0.state.borrow_mut();
        for b in boundaries(&schedule.working, now.date()) {
            if b.at <= now && !state.handled_boundaries.contains(&b.id) {
                state.handled_boundaries.push(b.id);
            }
        }
    }

    fn remove_claude_owned(&self, key: &str) {
        let mut state = self.0.state.borrow_mut();
        let before = state.claude_owned.len();
        state.claude_owned.retain(|k| !eq_ic(k, key));
        if state.claude_owned.len() != before {
            state.save();
        }
    }

    /// 定期清暫存：專案狀態清單（順便讓狀態更新）、Claude session 的快取、讀過的檔案內容（圖片很佔記憶體）。
    pub fn trim_caches(&self) {
        self.s_mut().project_details.clear();
        self.0.claude.borrow_mut().trim_caches();
        if let Some(files) = self.0.files.borrow().as_ref() {
            files.clear();
        }
    }

    fn raise(&self) {
        self.0.events.changed.invoke(&());
    }
}

fn boundaries(segs: &[WorkSegment], day: NaiveDate) -> Vec<Boundary> {
    let mut list = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        list.push(Boundary { at: day.and_time(seg.start), is_start: true, segment: i, id: format!("S{i}") });
        list.push(Boundary { at: day.and_time(seg.end), is_start: false, segment: i, id: format!("E{i}") });
    }
    list.sort_by_key(|b| b.at);
    list
}

#[cfg(test)]
mod tests;
