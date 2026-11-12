//! 對應 C# `Core/TimerEngine.Work.cs`：我的任務分頁、團隊請假、任務狀態、通知封存／稍後提醒、Claude 整理的工作日誌。

use std::rc::Rc;
use std::sync::Arc;

use chrono::NaiveDate;
use hb_api::{
    HayakuApi, KeyFinder, NotificationItem, ProjectDetail, TaskItem, TaskSummary, TeamMember, TimeEntry, WorkLogEntry,
};
use hb_claude::session_digest;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::TimerEngine;
use super::extras::try_load;
use crate::ci_set::eq_ic;
use crate::error::{CoreError, is_transient};
use crate::my_task_tabs::MyTaskTabs;
use crate::time;

/// Claude 整理的工作日誌：Markdown 草稿、建議的計時備註（計時段 id → 備註）、用到的指令與 commit 數。
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeDigest {
    pub markdown: String,
    pub notes: IndexMap<String, String>,
    pub prompt_count: usize,
    pub commit_count: usize,
}

/// my_open_tasks 最多翻幾頁（C# 預設值）。
const MY_TASKS_MAX_PAGES: i32 = 10;

impl TimerEngine {
    /// my_tasks 的分頁：進行中、測試（放在進行中後面，跟網頁一樣）、逾期、該開始還沒開始、未開始。
    pub const TASK_TABS: [(&'static str, &'static str); 5] = [
        ("in_progress", "進行中"),
        ("test", "測試"),
        ("overdue", "逾期"),
        ("should_start", "該開始"),
        ("not_started", "未開始"),
    ];

    pub const ALL_TAB: &'static str = "all";

    /// 畫面上的分頁：「全部」（所有還沒完成的，進行中排前面）＋ my_tasks 的分頁。
    pub const TASK_VIEW_TABS: [(&'static str, &'static str); 6] = [
        (Self::ALL_TAB, "全部"),
        ("in_progress", "進行中"),
        ("test", "測試"),
        ("overdue", "逾期"),
        ("should_start", "該開始"),
        ("not_started", "未開始"),
    ];

    /// 我的任務各分頁（分頁 → 任務）。
    pub fn task_lists(&self) -> Rc<IndexMap<String, Rc<Vec<TaskSummary>>>> {
        self.s().task_lists.clone()
    }

    pub fn team(&self) -> Rc<Vec<TeamMember>> {
        self.s().team.clone()
    }

    /// 直接用 Hayaku API（工作台的查詢與表單用；錯誤由呼叫端處理）。
    pub fn api(&self) -> Arc<HayakuApi> {
        self.0.api.clone()
    }

    pub fn task_list(&self, tab: &str) -> Rc<Vec<TaskSummary>> {
        let s = self.s();
        if tab == Self::ALL_TAB {
            return s.all_tasks.clone();
        }
        s.task_lists.get(tab).cloned().unwrap_or_default()
    }

    pub fn overdue_count(&self) -> usize {
        self.task_list("overdue").len()
    }

    fn find_in_lists(&self, key: &str) -> Option<TaskSummary> {
        self.s().task_lists.values().flat_map(|l| l.iter()).find(|t| eq_ic(&t.key, key)).cloned()
    }

    /// 任務目前的狀態（從我的任務清單找；不在清單裡回 None）。
    pub fn status_of(&self, key: &str) -> Option<String> {
        self.find_in_lists(key)
            .map(|t| t.status)
            .or_else(|| self.s().my_tasks.iter().find(|t| eq_ic(&t.key, key)).map(|t| t.status.clone()))
    }

    /// 直接上層任務的名稱（從我的任務清單的 path 找，不打 Hayaku；最上層或不在清單裡回 None）。
    pub fn parent_title_of(&self, key: &str) -> Option<String> {
        let t = self.find_in_lists(key)?;
        let (_, title) = t.ancestors.as_ref()?.last()?;
        if title.is_empty() { None } else { Some(title.clone()) }
    }

    /// 上層任務名稱，不打 Hayaku：我的任務清單或之前補讀過的。第二個值 false 表示還不知道（要 `load_parent_title`）。
    pub fn known_parent_title(&self, key: &str) -> (Option<String>, bool) {
        if self.find_in_lists(key).is_some() {
            return (self.parent_title_of(key), true);
        }
        match self.s().parent_titles.get(key) {
            Some(title) => (title.clone(), true),
            None => (None, false),
        }
    }

    /// 用 get_task 補讀上層任務名稱並記起來（同一個任務這次執行只讀一次）；讀失敗不記，下次打開再試。
    pub async fn load_parent_title(&self, key: &str) -> Result<(), CoreError> {
        let d = self.0.api.get_task(key).await?;
        let title = d.path.last().map(|(_, t)| t.clone()).filter(|t| !t.is_empty());
        self.s_mut().parent_titles.insert(key.to_owned(), title);
        Ok(())
    }

    /// 今天到期（預計結束是今天、還沒完成）。
    pub fn due_today_count(&self) -> usize {
        let today = time::today();
        let keys: std::collections::HashSet<String> = self
            .s()
            .task_lists
            .values()
            .flat_map(|l| l.iter())
            .filter(|t| t.planned_end == Some(today))
            .map(|t| t.key.clone())
            .collect();
        keys.len()
    }

    pub fn on_leave_today(&self) -> Vec<TeamMember> {
        self.s().team.iter().filter(|m| m.on_leave).cloned().collect()
    }

    pub(super) async fn refresh_work_core(&self) -> Result<(), CoreError> {
        self.refresh_my_tasks().await?;
        let team = try_load(self.0.api.team_schedule(), self.team(), "團隊狀況").await?;
        self.s_mut().team = team;
        let pins = try_load(self.0.api.list_pins(), self.pins(), "釘選托盤").await?;
        self.s_mut().pins = pins;
        Ok(())
    }

    /// 我的任務：一個請求讀全部未完成的，四個分頁在本機分（原本一個分頁打一次）；
    /// 開始任務的清單、Claude 的任務選單也用同一份。讀不到時保留上次的。
    pub(super) async fn refresh_my_tasks(&self) -> Result<(), CoreError> {
        let open = match self.0.api.my_open_tasks(MY_TASKS_MAX_PAGES).await {
            Ok(open) => open,
            Err(err) if is_transient(&err) => {
                log::error!("讀取我的任務失敗: {err}");
                return Ok(());
            }
            Err(err) => return Err(err.into()),
        };
        let tabs = MyTaskTabs::split(&open, time::today());
        for t in &open {
            self.add_known_project(&t.key);
        }
        let mut s = self.s_mut();
        s.task_lists = Rc::new(tabs.into_iter().map(|(k, v)| (k, Rc::new(v))).collect());
        s.all_tasks = Rc::new(MyTaskTabs::all(&open));
        s.my_tasks = Rc::new(
            open.iter()
                .map(|t| TaskItem {
                    key: t.key.clone(),
                    title: t.title.clone(),
                    project: t.project.clone(),
                    status: t.status.clone(),
                })
                .collect(),
        );
        s.tasks_fetched = time::now();
        Ok(())
    }

    /// 專案的狀態清單、成員（快取）。
    pub async fn get_project(&self, project: &str) -> Result<Rc<ProjectDetail>, CoreError> {
        if let Some(cached) = self.s().project_details.get(project) {
            return Ok(cached.clone());
        }
        let detail = Rc::new(self.0.api.get_project(project).await?);
        self.s_mut().project_details.insert(project.to_owned(), detail.clone());
        Ok(detail)
    }

    /// 停止計時並把任務改成完成類狀態（跟網頁的「停止並完成」一樣）。
    /// cascade：false＝這次不帶動上層、子任務的狀態（任務狀態連動）；None 照專案設定。
    pub async fn complete_task(&self, key: &str, cascade: Option<bool>) {
        self.guarded(
            async {
                let project = self.get_project(KeyFinder::project_of(key)).await?;
                let Some(done) = project.statuses.iter().find(|s| s.category == "DONE").cloned() else {
                    return Err(CoreError::mcp(format!("{} 沒有完成類的狀態", project.name)));
                };
                if self.running_keys().contains(key) {
                    self.s_mut().self_paused.insert(key);
                    self.0.api.pause_timer(Some(key)).await?;
                    {
                        let mut s = self.s_mut();
                        if s.claude_desired.contains(key) {
                            s.suppressed.insert(key);
                        }
                    }
                    self.remove_claude_owned(key);
                }
                let mut fields = Map::new();
                fields.insert("status".into(), json!(done.name));
                if let Some(c) = cascade {
                    fields.insert("cascade".into(), Value::Bool(c));
                }
                let message = self.0.api.update_task(key, fields).await?;
                self.notify(
                    message.filter(|m| !m.is_empty()).unwrap_or_else(|| format!("{key} 已停止並改成「{}」", done.name)),
                );
                self.invalidate_extras();
                self.refresh_core(false).await
            },
            true,
        )
        .await;
    }

    pub async fn set_task_status(&self, key: &str, status: &str) {
        self.guarded(
            async {
                let mut fields = Map::new();
                fields.insert("status".into(), json!(status));
                let message = self.0.api.update_task(key, fields).await?;
                self.notify(message.filter(|m| !m.is_empty()).unwrap_or_else(|| format!("{key} 改成「{status}」")));
                self.invalidate_extras();
                self.refresh_extras_core(false).await
            },
            true,
        )
        .await;
    }

    pub async fn archive_notification(&self, n: &NotificationItem) {
        self.guarded(
            async {
                self.0.api.archive_notifications(&n.all_ids()).await?;
                self.drop_notification(&n.id);
                Ok(())
            },
            true,
        )
        .await;
    }

    /// preset：1h、next_workday、next_week。
    pub async fn snooze_notification(&self, n: &NotificationItem, preset: &str) {
        self.guarded(
            async {
                self.0.api.snooze_notifications(&n.all_ids(), preset).await?;
                self.drop_notification(&n.id);
                Ok(())
            },
            true,
        )
        .await;
    }

    fn drop_notification(&self, id: &str) {
        let mut s = self.s_mut();
        s.notifications = Rc::new(s.notifications.iter().filter(|x| x.id != id).cloned().collect());
        s.discussions = Rc::new(s.discussions.iter().filter(|x| x.id != id).cloned().collect());
    }

    /// 工作台做了修改後呼叫：馬上同步一次，一併更新會議、待辦、提醒、通知、任務（改過資料，不受打開時的節流限制）。
    /// 有別的同步正在跑時等它跑完再同步，清單才會跟著變（不會被略過）。
    pub async fn after_external_change(&self) {
        self.invalidate_extras();
        self.s_mut().tasks_fetched = time::MIN;
        if self.logged_in() {
            self.refresh(true).await;
        }
    }

    // ───────────── Claude 整理的工作日誌 ─────────────

    /// 把某天的 Claude Code 對話與 git commit 整理成工作日誌草稿，並替還沒寫備註的計時段產生備註。
    /// 讀檔在背景執行環境跑，git 由 hb-claude 丟到背景執行。
    pub async fn build_digest(
        &self,
        day: NaiveDate,
        log_entries: &[WorkLogEntry],
        entries: &[TimeEntry],
    ) -> ClaudeDigest {
        let (units, is_known) = {
            let claude = self.0.claude.borrow();
            (claude.repos().cloned().collect::<Vec<_>>(), claude.is_known_key.clone())
        };
        let session_tasks = self.0.state.borrow().session_tasks.clone();
        let now = time::now();
        let prompts = {
            let (units, is_known) = (units.clone(), is_known.clone());
            hb_backend::rt::bg(async move { session_digest::collect(day, &units, &session_tasks, &*is_known) }).await
        };
        let folders: Vec<String> = prompts
            .iter()
            .map(|p| p.folder.clone())
            .chain(units.iter().filter(|u| u.enabled).map(|u| u.path.clone()))
            .collect();
        let commits = session_digest::commits(day, folders, &prompts, &*is_known).await;
        let title_of = |key: &str| self.title_of(key);
        ClaudeDigest {
            markdown: session_digest::markdown(day, &prompts, &commits, log_entries, &title_of),
            notes: session_digest::notes_for(entries, &prompts, &commits, now),
            prompt_count: prompts.len(),
            commit_count: commits.len(),
        }
    }

    /// 一次寫入多段計時的備註，最後同步一次。
    pub async fn set_entry_notes(&self, notes: &IndexMap<String, String>) {
        self.guarded(
            async {
                for (id, note) in notes {
                    self.0.api.update_time_entry(id, None, None, Some(note)).await?;
                }
                self.refresh_core(false).await
            },
            true,
        )
        .await;
    }

    /// 計時中或我的任務裡找得到的任務名稱。
    pub fn title_of(&self, key: &str) -> Option<String> {
        if let Some(e) = self.s().entries.iter().rev().find(|e| eq_ic(&e.task, key)) {
            return Some(e.title.clone());
        }
        if let Some(t) = self.find_in_lists(key) {
            return Some(t.title);
        }
        self.s().my_tasks.iter().find(|t| eq_ic(&t.key, key)).map(|t| t.title.clone())
    }
}
