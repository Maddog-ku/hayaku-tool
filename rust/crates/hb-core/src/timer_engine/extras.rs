//! 對應 C# `Core/TimerEngine.Extras.cs`：會議、待辦、提醒、通知、工時修改，以及到點跳出的提示。

use std::fmt;
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use chrono::{NaiveDate, TimeDelta};
use hb_api::{DaySchedule, KeyFinder, MeetingItem, NotificationItem, ReminderItem, TimeEntry, TodoItem, WorkMath};
use hb_backend::config::MeetingPause;

use super::{LocalFuture, TimerEngine};
use crate::ci_set::CiSet;
use crate::error::{CoreError, is_mcp, is_transient};
use crate::time::{self, MIN};

/// 提示按鈕的動作（C# `Func<Task>`）；錯誤都在裡面處理掉，不會往外丟。
pub type AsyncAction = Rc<dyn Fn() -> LocalFuture>;

/// 右下角跳出提示上的按鈕。
#[derive(Clone)]
pub struct ToastAction {
    pub label: String,
    pub run: AsyncAction,
    pub primary: bool,
}

impl ToastAction {
    pub fn new<F, Fut>(label: impl Into<String>, run: F) -> Self
    where
        F: Fn() -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        Self { label: label.into(), run: Rc::new(move || Box::pin(run()) as LocalFuture), primary: false }
    }

    /// 主要按鈕（C# `Primary: true`）。
    pub fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub fn primary_if(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }
}

impl fmt::Debug for ToastAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToastAction").field("label", &self.label).field("primary", &self.primary).finish()
    }
}

/// 右下角跳出提示。
/// duration：停留多久自動收回（None 用預設）；from_bottom：從螢幕下緣滑上來、時間到滑回去（新通知用）；
/// important：全螢幕、簡報時也照樣跳出（會議快開始）；tag：標記，在 Slack 上處理了就用它關掉這則提示。
#[derive(Debug, Clone, Default)]
pub struct ToastRequest {
    pub title: String,
    pub message: String,
    pub actions: Vec<ToastAction>,
    pub sticky: bool,
    /// 要按掉才關、而且不會時間到自己收（sticky 原本最多停 10 分鐘，同 C#）；建立任務提示設成「不自動收起」時用。
    pub persistent: bool,
    pub duration: Option<Duration>,
    pub from_bottom: bool,
    pub important: bool,
    pub tag: Option<String>,
}

impl ToastRequest {
    pub fn new(title: impl Into<String>, message: impl Into<String>, actions: Vec<ToastAction>) -> Self {
        Self { title: title.into(), message: message.into(), actions, ..Default::default() }
    }

    pub fn sticky(mut self, sticky: bool) -> Self {
        self.sticky = sticky;
        self
    }

    /// 不會時間到自己收起（一定是 sticky）。
    pub fn persistent(mut self, persistent: bool) -> Self {
        self.persistent = persistent;
        if persistent {
            self.sticky = true;
        }
        self
    }

    pub fn duration(mut self, duration: Option<Duration>) -> Self {
        self.duration = duration;
        self
    }

    pub fn from_bottom(mut self) -> Self {
        self.from_bottom = true;
        self
    }

    pub fn important(mut self) -> Self {
        self.important = true;
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = Some(tag.into());
        self
    }
}

/// 工作台某一頁的參數（C# 的 `object? arg`：任務編號、會議 id、專案代碼、"draft"…是文字，問題回報編號是數字）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceArg {
    Text(String),
    Number(i32),
}

impl WorkspaceArg {
    pub fn text(s: impl Into<String>) -> Self {
        WorkspaceArg::Text(s.into())
    }
}

// 通知要即時提示，2 分鐘；會議、提醒的到點提示用已抓的時間在本機判斷，10 分鐘就夠
const NOTIFICATIONS_INTERVAL: TimeDelta = TimeDelta::minutes(2);
const MEETINGS_INTERVAL: TimeDelta = TimeDelta::minutes(10);

/// 只在面板、工作台看得到的資料（待辦、我的任務分頁、團隊）：不在背景抓，打開時才抓，太久沒抓才重抓。
const PANEL_DATA_INTERVAL: TimeDelta = TimeDelta::minutes(2);

/// 打開面板、工作台時，距離上次完整同步不到這麼久就不重抓（常常開開關關時，一次完整同步約 8～10 個請求）。要最新的按重新整理。
const OPEN_THROTTLE: TimeDelta = TimeDelta::seconds(60);

/// 讀清單失敗（工具錯誤、連不上）時保留上次的；需要登入等其他錯誤往外傳（C# `Try<T>`）。
pub(super) async fn try_load<T>(
    load: impl Future<Output = Result<Vec<T>, hb_api::McpError>>,
    fallback: Rc<Vec<T>>,
    what: &str,
) -> Result<Rc<Vec<T>>, CoreError> {
    match load.await {
        Ok(list) => Ok(Rc::new(list)),
        Err(err) if is_transient(&err) => {
            log::error!("讀取{what}失敗: {err}");
            Ok(fallback)
        }
        Err(err) => Err(err.into()),
    }
}

impl TimerEngine {
    pub fn meetings(&self) -> Rc<Vec<MeetingItem>> {
        self.s().meetings.clone()
    }

    pub fn todos(&self) -> Rc<Vec<TodoItem>> {
        self.s().todos.clone()
    }

    pub fn reminders(&self) -> Rc<Vec<ReminderItem>> {
        self.s().reminders.clone()
    }

    pub fn notifications(&self) -> Rc<Vec<NotificationItem>> {
        self.s().notifications.clone()
    }

    pub fn unread_count(&self) -> usize {
        self.s().notifications.iter().filter(|n| n.unread).count()
    }

    /// 我的討論（通知的討論分頁：空間評論、聊天討論串、@ 我，一段討論一組）；只有打開那個分頁才讀，不輪詢。
    pub fn discussions(&self) -> Rc<Vec<NotificationItem>> {
        self.s().discussions.clone()
    }

    /// 讀我的討論；source：space（空間評論）、chat（聊天），None 是全部。一分鐘內、條件一樣就不重讀（force 例外）。
    /// 失敗回傳錯誤給呼叫端顯示。
    pub async fn load_discussions(
        &self,
        unresolved_only: bool,
        source: Option<&str>,
        force: bool,
    ) -> Result<(), CoreError> {
        let query = format!("{}|{}", if unresolved_only { "True" } else { "False" }, source.unwrap_or(""));
        {
            let s = self.s();
            if !force && query == s.discussions_query && time::now() - s.discussions_fetched < TimeDelta::minutes(1) {
                return Ok(());
            }
        }
        let list = self.0.api.list_discussions(unresolved_only, source).await?;
        {
            let mut s = self.s_mut();
            s.discussions = Rc::new(list);
            s.discussions_query = query;
            s.discussions_fetched = time::now();
        }
        self.raise();
        Ok(())
    }

    /// 要開工作台的某一頁（C# `WorkspaceRequested?.Invoke(page, arg)`）。
    pub(super) fn request_workspace(&self, page: &str, arg: Option<WorkspaceArg>) {
        self.0.events.workspace_requested.invoke(&(page.to_owned(), arg));
    }

    // ───────────── 網址 ─────────────

    pub fn web_url(&self, path: &str) -> String {
        format!("{}{}", self.0.config.borrow().base_url.trim_end_matches('/'), path)
    }

    pub fn task_url(&self, key: &str) -> String {
        self.web_url(&format!(
            "/projects/{}?task={}",
            urlencoding::encode(KeyFinder::project_of(key)),
            urlencoding::encode(key)
        ))
    }

    pub fn meeting_url(&self, id: &str) -> String {
        self.web_url(&format!("/meetings/{}", urlencoding::encode(id)))
    }

    pub fn notification_url(&self, n: &NotificationItem) -> String {
        if let Some(meeting) = &n.meeting_id {
            self.meeting_url(meeting)
        } else if let Some(task) = &n.task_key {
            self.task_url(task)
        } else {
            self.web_url("/notifications")
        }
    }

    /// 點通知：能在工作台處理的就在工作台打開（申請 → 待我審核、會議 → 會議、任務相關 → 任務詳細、
    /// 論壇、空間、頻道、補工時 → 對應頁），其他的才開網頁；順便標為已讀。
    pub async fn open_notification(&self, n: &NotificationItem) {
        match Self::notification_target(n) {
            (Some(page), arg) => self.request_workspace(page, arg),
            (None, _) => open(&self.notification_url(n)),
        }
        if n.unread {
            self.mark_notifications_read(Some(n.all_ids())).await;
        }
    }

    /// 通知對應到工作台的哪一頁（與參數）；沒有對應的頁回 (None, None)，開網頁。
    pub fn notification_target(n: &NotificationItem) -> (Option<&'static str>, Option<WorkspaceArg>) {
        match n.r#type.as_str() {
            "CHANGE_REQUESTED" => return (Some("reviews"), None),
            "HOURS_UNFILLED" | "HOURS_UNFILLED_WEEKLY" => return (Some("hours"), None),
            "FORUM_POST" => return (Some("forum"), n.project_key.clone().map(WorkspaceArg::Text)),
            "SPACE_PAGE_COMMENT" => return (Some("spaces"), None),
            _ => {}
        }
        if let Some(number) = n.feedback {
            return (Some("feedback"), Some(WorkspaceArg::Number(number)));
        }
        if let Some(meeting) = &n.meeting_id {
            return (Some("meetings"), Some(WorkspaceArg::Text(meeting.clone())));
        }
        if let Some(key) = &n.task_key {
            return (Some("tasks"), Some(WorkspaceArg::Text(key.clone())));
        }
        match n.r#type.as_str() {
            "CHANNEL_MESSAGE" | "THREAD_REPLY" | "CHAT_REACTION" => (Some("chat"), None),
            _ => (None, None),
        }
    }

    // ───────────── 同步 ─────────────

    /// 按重新整理（user_action）或打開面板、工作台時呼叫：同步計時，並馬上更新會議、待辦、提醒、通知、任務。
    /// 打開時距離上次不到 60 秒就略過，連續開關不會一直打 Hayaku。
    pub async fn refresh_all(&self, user_action: bool) {
        let now = time::now();
        if !self.logged_in() || (!user_action && (now - self.s().open_refreshed < OPEN_THROTTLE || self.off_today(now)))
        {
            return; // 不用上班的日子打開面板也不同步，按重新整理才會
        }
        self.s_mut().open_refreshed = now;
        self.invalidate_extras(); // 就算這次被略過，下一次同步也會一起更新
        self.refresh(user_action).await;
    }

    /// 下次同步時重抓會議、提醒、通知、待辦、任務分頁、團隊。
    pub(super) fn invalidate_extras(&self) {
        let mut s = self.s_mut();
        s.notifications_fetched = MIN;
        s.meetings_fetched = MIN;
        s.panel_data_fetched = MIN;
    }

    /// background：背景輪詢時不抓只有面板、工作台才看得到的資料。
    pub(super) async fn refresh_extras_core(&self, background: bool) -> Result<(), CoreError> {
        let now = time::now();
        let today = now.date();
        if now - self.s().meetings_fetched >= MEETINGS_INTERVAL {
            self.s_mut().meetings_fetched = now;
            let mine = !self.0.config.borrow().meeting_alerts_all_projects;
            let meetings = try_load(self.0.api.list_meetings(today, today, mine), self.meetings(), "會議").await?;
            self.s_mut().meetings = meetings;
            let reminders = try_load(self.0.api.list_reminders(), self.reminders(), "提醒").await?;
            self.s_mut().reminders = reminders;
        }
        if now - self.s().notifications_fetched >= NOTIFICATIONS_INTERVAL {
            self.s_mut().notifications_fetched = now;
            let notifications = try_load(self.0.api.list_notifications(), self.notifications(), "通知").await?;
            self.s_mut().notifications = notifications;
        }
        // 管理員、編輯者才抓待我審核的筆數：每 10 分鐘，或收到「有新的變更申請」通知時
        let reviews_fetched = self.s().reviews_fetched;
        let new_request = self
            .notifications()
            .iter()
            .any(|n| n.r#type == "CHANGE_REQUESTED" && n.unread && time::local(&n.created_at) > reviews_fetched);
        if self.can_review() && (new_request || now - reviews_fetched >= super::pins::REVIEWS_INTERVAL) {
            self.refresh_reviews_core().await?;
        }
        if !background && now - self.s().panel_data_fetched >= PANEL_DATA_INTERVAL {
            self.s_mut().panel_data_fetched = now;
            let todos = try_load(self.0.api.list_todos(), self.todos(), "待辦").await?;
            self.s_mut().todos = todos;
            self.refresh_work_core().await?;
        }
        self.check_alerts();
        Ok(())
    }

    // ───────────── 動作 ─────────────

    pub async fn join_meeting(&self, meeting: &MeetingItem) {
        self.guarded(
            async {
                let before = self.running_keys();
                self.0.api.join_meeting(&meeting.key).await?;
                self.invalidate_extras();
                self.refresh_core(false).await?;
                let paused = if self.0.config.borrow().meeting_pauses_others {
                    self.pause_for_meeting(&meeting.key, &before).await?
                } else {
                    Vec::new()
                };
                self.notify(format!(
                    "已加入會議並開始計時：{}{}",
                    meeting.title,
                    if paused.is_empty() { String::new() } else { format!("，暫停 {}", paused.join("、")) }
                ));
                Ok(())
            },
            true,
        )
        .await;
    }

    pub async fn leave_meeting(&self, meeting: &MeetingItem) {
        self.guarded(
            async {
                self.0.api.leave_meeting(&meeting.key).await?;
                self.invalidate_extras();
                self.refresh_core(false).await?;
                let paused_for_this =
                    self.0.state.borrow().meeting_pause.as_ref().is_some_and(|p| p.meeting == meeting.key);
                if paused_for_this {
                    self.resume_after_meeting("已離開會議").await?;
                } else {
                    self.notify("已離開會議，停止計時".to_owned());
                }
                Ok(())
            },
            true,
        )
        .await;
    }

    /// 加入會議前就在跑的計時，會議以外的都暫停，記下來等離開會議後恢復。
    async fn pause_for_meeting(&self, meeting: &str, before: &CiSet) -> Result<Vec<String>, CoreError> {
        let (meeting_tasks, others) = Self::split_for_meeting(meeting, before, &self.running_keys());
        let mut paused = Vec::new();
        for key in others {
            self.s_mut().self_paused.insert(key.as_str()); // 自己暫停的：保留 Claude 標記，恢復後照舊
            match self.0.api.pause_timer(Some(&key)).await {
                Ok(()) => paused.push(key),
                Err(err) if is_mcp(&err) => log::error!("加入會議時暫停 {key} 失敗: {err}"),
                Err(err) => return Err(err.into()),
            }
        }
        if paused.is_empty() {
            return Ok(paused);
        }
        {
            let mut state = self.0.state.borrow_mut();
            let mut pause = state.meeting_pause.clone().unwrap_or_default(); // 會議中又加入另一場：之前暫停的一起留著
            pause.meeting = meeting.to_owned();
            pause.meeting_tasks = meeting_tasks;
            let mut tasks: CiSet = pause.tasks.iter().cloned().collect();
            tasks.extend(paused.iter().cloned());
            pause.tasks = tasks.to_vec();
            state.meeting_pause = Some(pause);
            state.save();
        }
        log::info!("加入會議 {meeting}，暫停 {}", paused.join("、"));
        self.refresh_core(false).await?;
        Ok(paused)
    }

    /// 加入會議後在跑的計時分成「會議的」與「要暫停的」：會議本身是任務（例 REVIVAL-9），
    /// 找不到的話把加入後才出現的當成會議的；加入前就在跑的其他計時要暫停。
    pub fn split_for_meeting(meeting: &str, before: &CiSet, after: &CiSet) -> (Vec<String>, Vec<String>) {
        let meeting_tasks: Vec<String> = match after.get(meeting) {
            Some(k) => vec![k.to_owned()],
            None => {
                let mut list: Vec<String> = after.iter().filter(|k| !before.contains(k)).map(str::to_owned).collect();
                list.sort();
                list
            }
        };
        let meeting_set: CiSet = meeting_tasks.iter().cloned().collect();
        let mut others: Vec<String> =
            after.iter().filter(|k| before.contains(k) && !meeting_set.contains(k)).map(str::to_owned).collect();
        others.sort();
        (meeting_tasks, others)
    }

    /// 不是按「離開會議」結束的（網頁上離開、手動停掉會議計時）：同步時發現會議的計時都停了，一樣恢復。
    pub(super) async fn check_meeting_left(&self) -> Result<(), CoreError> {
        let Some(pause) = self.0.state.borrow().meeting_pause.clone().filter(|p| !p.meeting_tasks.is_empty()) else {
            return Ok(());
        };
        let running = self.running_keys();
        if pause.meeting_tasks.iter().any(|k| running.contains(k)) {
            return Ok(());
        }
        self.resume_after_meeting("會議的計時停了").await
    }

    async fn resume_after_meeting(&self, reason: &str) -> Result<(), CoreError> {
        let pause: MeetingPause = {
            let mut state = self.0.state.borrow_mut();
            let Some(pause) = state.meeting_pause.take() else {
                return Ok(());
            };
            state.save();
            pause
        };
        let running = self.running_keys();
        let mut started = Vec::new();
        for key in pause.tasks.iter().filter(|k| !running.contains(k)) {
            match self.0.api.start_timer(key).await {
                Ok(()) => started.push(key.clone()),
                Err(err) if is_mcp(&err) => log::error!("離開會議後恢復 {key} 失敗: {err}"),
                Err(err) => return Err(err.into()),
            }
        }
        if started.is_empty() {
            if reason == "已離開會議" {
                self.notify("已離開會議，停止計時".to_owned());
            }
            return Ok(());
        }
        self.notify(format!("{reason}，恢復計時：{}", started.join("、")));
        self.refresh_core(false).await
    }

    pub async fn complete_todo(&self, todo: &TodoItem) {
        self.guarded(
            async {
                self.0.api.complete_todo(&todo.id).await?;
                let mut s = self.s_mut();
                s.todos = Rc::new(s.todos.iter().filter(|t| t.id != todo.id).cloned().collect());
                Ok(())
            },
            true,
        )
        .await;
    }

    pub async fn snooze_reminder(&self, reminder: &ReminderItem, minutes: i32) {
        self.guarded(
            async {
                self.0.api.snooze_reminder(&reminder.id, minutes).await?;
                self.invalidate_extras();
                self.refresh_extras_core(false).await
            },
            true,
        )
        .await;
    }

    pub async fn quick_capture(&self, text: &str, todo: bool) {
        self.guarded(
            async {
                let key = KeyFinder::parse(Some(text)).filter(|k| self.is_known_project(KeyFinder::project_of(k)));
                self.0.api.quick_capture(text, key.as_deref(), todo).await?;
                self.notify(if todo { "已存成待辦" } else { "已存到隨手記" }.to_owned());
                if todo {
                    self.invalidate_extras();
                    self.refresh_extras_core(false).await?;
                }
                Ok(())
            },
            true,
        )
        .await;
    }

    /// 標為已讀；ids 為 None 是全部。
    pub async fn mark_notifications_read(&self, ids: Option<Vec<String>>) {
        self.guarded(
            async {
                self.0.api.mark_notifications_read(ids.as_deref()).await?;
                let hit = |id: &String| ids.as_ref().is_none_or(|ids| ids.contains(id));
                let mut s = self.s_mut();
                s.notifications = Rc::new(
                    s.notifications
                        .iter()
                        .map(|n| if hit(&n.id) { NotificationItem { unread: false, ..n.clone() } } else { n.clone() })
                        .collect(),
                );
                s.discussions = Rc::new(
                    s.discussions
                        .iter()
                        .map(|n| {
                            if ids.is_none() || n.all_ids().iter().any(hit) {
                                NotificationItem { unread: false, ..n.clone() }
                            } else {
                                n.clone()
                            }
                        })
                        .collect(),
                );
                Ok(())
            },
            true,
        )
        .await;
    }

    // ───────────── 工時紀錄 ─────────────

    pub async fn load_day(&self, day: NaiveDate) -> Result<(Vec<TimeEntry>, DaySchedule), CoreError> {
        let mut entries = self.0.api.list_day_entries(day).await?;
        let cached = self.schedule().filter(|s| s.date == day);
        let schedule = match cached {
            Some(s) => s,
            None => self.0.api.get_schedule(day).await?,
        };
        entries.sort_by_key(|e| e.started_at.map(|t| t.timestamp_micros()).unwrap_or(i64::MAX));
        Ok((entries, schedule))
    }

    pub async fn add_time_entry(
        &self,
        task: &str,
        start: hb_api::DateTimeOffset,
        end: hb_api::DateTimeOffset,
        note: Option<&str>,
    ) {
        self.guarded(
            async {
                self.0.api.add_time_entry(&task.to_uppercase(), start, end, note).await?;
                self.refresh_core(false).await
            },
            true,
        )
        .await;
    }

    pub async fn update_time_entry(
        &self,
        id: &str,
        start: Option<hb_api::DateTimeOffset>,
        end: Option<hb_api::DateTimeOffset>,
        note: Option<&str>,
    ) {
        self.guarded(
            async {
                self.0.api.update_time_entry(id, start, end, note).await?;
                self.refresh_core(false).await
            },
            true,
        )
        .await;
    }

    pub async fn delete_time_entry(&self, id: &str) {
        self.guarded(
            async {
                self.0.api.delete_time_entry(id).await?;
                self.refresh_core(false).await
            },
            true,
        )
        .await;
    }

    // ───────────── 下班復原 ─────────────

    pub async fn undo_clock_out(&self, tasks: &[String]) {
        self.guarded(
            async {
                {
                    let mut state = self.0.state.borrow_mut();
                    state.clocked_out = false;
                    state.save();
                }
                for key in tasks {
                    match self.0.api.start_timer(key).await {
                        Ok(()) => {}
                        Err(err) if is_mcp(&err) => log::error!("復原 {key} 失敗: {err}"),
                        Err(err) => return Err(err.into()),
                    }
                }
                self.notify(format!("已復原，重新開始 {} 個計時", tasks.len()));
                self.refresh_core(false).await
            },
            true,
        )
        .await;
    }

    pub(super) fn after_clock_out(&self, stopped: Vec<String>, worked: f64, expected: i32) {
        let mut actions = Vec::new();
        if !stopped.is_empty() {
            let e = self.clone();
            let stopped = stopped.clone();
            actions.push(
                ToastAction::new("復原", move || {
                    let (e, stopped) = (e.clone(), stopped.clone());
                    async move { e.undo_clock_out(&stopped).await }
                })
                .primary(),
            );
        }
        let (prompt_unfilled, claude_enabled, has_repos) = {
            let c = self.0.config.borrow();
            (c.prompt_unfilled, c.claude_enabled, !c.repos.is_empty())
        };
        let unfilled = prompt_unfilled && expected > 0 && !WorkMath::is_filled(worked, expected as f64);
        if unfilled {
            let e = self.clone();
            actions.push(ToastAction::new("補登工時", move || {
                e.0.events.time_entries_requested.invoke(&time::today());
                async {}
            }));
        }
        if claude_enabled && has_repos {
            let e = self.clone();
            actions.push(ToastAction::new("整理日誌", move || {
                e.request_workspace("worklog", Some(WorkspaceArg::text("draft")));
                async {}
            }));
        }
        let mut message = format!("停止 {} 個計時。今天 {}", stopped.len(), WorkMath::hours_minutes(worked));
        if expected > 0 {
            message.push_str(&format!("／應出勤 {}", WorkMath::hours_minutes(expected as f64)));
        }
        if unfilled {
            message.push_str(&format!("，還差 {}", WorkMath::hours_minutes(expected as f64 - worked)));
        }
        self.toast(ToastRequest::new("已下班", message, actions).sticky(unfilled));
    }

    // ───────────── 到點提示 ─────────────

    /// 會議快開始、提醒到了、有新通知時跳出提示；Claude 計時器每次 tick 檢查一次。
    pub fn check_alerts(&self) {
        if !self.logged_in() {
            return;
        }
        let now = time::now_offset();
        self.check_review_alerts();

        let (meeting_alerts, lead, reminder_alerts, notification_alerts, seconds) = {
            let c = self.0.config.borrow();
            (
                c.meeting_alerts,
                c.meeting_lead_minutes,
                c.reminder_alerts,
                c.notification_alerts,
                c.notification_popup_seconds,
            )
        };
        let mut toasts: Vec<ToastRequest> = Vec::new();

        if meeting_alerts {
            for m in self.meetings().iter() {
                if m.in_meeting()
                    || now < m.start_at - TimeDelta::minutes(lead as i64)
                    || now > m.start_at + TimeDelta::minutes(10)
                {
                    continue;
                }
                if !self.s_mut().alerted.insert(format!("m:{}:{}", m.id, m.start_at.to_rfc3339())) {
                    continue;
                }
                toasts.push(self.meeting_toast(m, now, false));
            }
        }

        if reminder_alerts {
            for r in self.reminders().iter() {
                let Some(at) = r.at else { continue };
                if at > now
                    || at < now - TimeDelta::minutes(30)
                    || !self.s_mut().alerted.insert(format!("r:{}:{}", r.id, at.to_rfc3339()))
                {
                    continue;
                }
                let e = self.clone();
                let reminder = r.clone();
                let mut actions = vec![
                    ToastAction::new("10 分鐘後再提醒", move || {
                        let (e, r) = (e.clone(), reminder.clone());
                        async move { e.snooze_reminder(&r, 10).await }
                    })
                    .primary(),
                ];
                if let Some(task) = &r.task {
                    let url = self.task_url(task);
                    actions.push(ToastAction::new(format!("開啟 {task}"), move || {
                        open(&url);
                        async {}
                    }));
                }
                toasts.push(ToastRequest::new("提醒", r.text.clone(), actions).sticky(true));
            }
        }

        if notification_alerts {
            let can_review = self.can_review();
            let started_at = self.s().started_at;
            let mut fresh: Vec<NotificationItem> = Vec::new();
            for n in self.notifications().iter() {
                // 「有新的變更申請」由審核提示（可以直接核准、駁回）取代
                if n.unread
                    && n.created_at > started_at
                    && !(n.r#type == "CHANGE_REQUESTED" && can_review)
                    && self.s_mut().alerted.insert(format!("n:{}", n.id))
                {
                    fresh.push(n.clone());
                }
            }
            // 從下面滑上來，停留幾秒（設定可調；滑鼠移上去會停住）後自動滑回去
            let duration = if seconds > 0 { Some(Duration::from_secs(seconds as u64)) } else { None };
            if fresh.len() > 3 {
                let ids: Vec<String> = fresh.iter().map(|n| n.id.clone()).collect();
                let labels: Vec<&str> = fresh.iter().take(3).map(|n| n.label.as_str()).collect();
                let (e1, e2) = (self.clone(), self.clone());
                toasts.push(
                    ToastRequest::new(
                        format!("有 {} 則新通知", fresh.len()),
                        format!("{}…", labels.join("、")),
                        vec![
                            ToastAction::new("全部標為已讀", move || {
                                let (e, ids) = (e1.clone(), ids.clone());
                                async move { e.mark_notifications_read(Some(ids)).await }
                            })
                            .primary(),
                            ToastAction::new("查看", move || {
                                open(&e2.web_url("/notifications"));
                                async {}
                            }),
                        ],
                    )
                    .sticky(seconds == 0)
                    .duration(duration)
                    .from_bottom(),
                );
            } else {
                let is_test = |n: &NotificationItem| n.is_test_result() && n.task_key.is_some();
                for n in fresh.iter().filter(|n| is_test(n)) {
                    toasts.push(self.test_toast(n, duration, seconds == 0));
                }
                for n in fresh.iter().filter(|n| !is_test(n)) {
                    let (e1, e2) = (self.clone(), self.clone());
                    let (id, item) = (n.id.clone(), n.clone());
                    let actor = n.actor.as_ref().map(|a| format!("{a}：")).unwrap_or_default();
                    toasts.push(
                        ToastRequest::new(
                            n.label.clone(),
                            format!("{actor}{}", n.text),
                            vec![
                                ToastAction::new("標為已讀", move || {
                                    let (e, id) = (e1.clone(), id.clone());
                                    async move { e.mark_notifications_read(Some(vec![id])).await }
                                })
                                .primary(),
                                ToastAction::new("查看", move || {
                                    let (e, n) = (e2.clone(), item.clone());
                                    async move { e.open_notification(&n).await }
                                }),
                            ],
                        )
                        .sticky(seconds == 0)
                        .duration(duration)
                        .from_bottom()
                        .tag(format!("n:{}", n.id)),
                    );
                }
            }
        }
        for t in toasts {
            self.toast(t);
        }
    }

    /// 會議快開始的提示；demo＝本機模擬（--meeting-test），按鈕不會真的加入或開網頁。
    pub fn meeting_toast(&self, m: &MeetingItem, now: hb_api::DateTimeOffset, demo: bool) -> ToastRequest {
        let when = if m.start_at > now {
            let minutes = hb_api::work_math::total_minutes(m.start_at - now);
            format!("{} 分鐘後開始", minutes.ceil() as i64)
        } else {
            "已經開始了".to_owned()
        };
        let demo_action = |what: &'static str, e: TimerEngine| {
            move || {
                e.notify(format!("（測試）這是模擬的會議，沒有真的{what}"));
                async {}
            }
        };
        let join = if demo {
            ToastAction::new("加入並計時", demo_action("加入", self.clone()))
        } else {
            let (e, meeting) = (self.clone(), m.clone());
            ToastAction::new("加入並計時", move || {
                let (e, m) = (e.clone(), meeting.clone());
                async move { e.join_meeting(&m).await }
            })
        };
        let open_action = if demo {
            ToastAction::new("開啟", demo_action("開啟", self.clone()))
        } else {
            let url = self.meeting_url(&m.id);
            ToastAction::new("開啟", move || {
                open(&url);
                async {}
            })
        };
        let local_start = m.start_at.with_timezone(&chrono::Local);
        ToastRequest::new(
            format!("會議 {}・{when}", local_start.format("%H:%M")),
            if m.not_attendee() {
                format!("{}\n{}・你不是參加者，加入時會自動加為參加者", m.title, m.project)
            } else {
                m.title.clone()
            },
            vec![join.primary(), open_action],
        )
        .sticky(true)
        .important()
    }
}

/// 用預設瀏覽器開網址（C# `Web.Open`）。
pub(super) fn open(url: &str) {
    hb_backend::infra::web::open(url);
}
