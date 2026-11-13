//! 對應 C# `Core/TimerEngine.Testing.cs`：測試流程。開始修、送復驗、通過、退回；通知（被退回、進待復驗）跳提示可以直接處理。
//! 都是按了才送請求；我要修、要驗的數量從「我的任務」那一份在本機算，不另外抓。

use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use hb_api::{KeyFinder, NotificationItem, TaskKind, TaskKinds, TaskSummary, TestSubStatus};
use serde_json::{Map, Value, json};

use super::{TimerEngine, ToastAction, ToastRequest};
use crate::ci_set::eq_ic;
use crate::error::CoreError;
use crate::file_reader::FileReader;

/// 測試結果要人處理的動作：fix（開始修，可填預估工時）、reopen（退回，填說明）、add（在測試任務加入測試結果）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestActionRequest {
    pub action: String,
    pub key: String,
    pub title: String,
}

fn status_fields(status: &str) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("status".into(), json!(status));
    fields
}

fn non_empty(message: Option<String>) -> Option<String> {
    message.filter(|m| !m.is_empty())
}

impl TimerEngine {
    pub const STATUS_FIXING: &'static str = "修復中";
    pub const STATUS_RETEST: &'static str = "待復驗";
    pub const STATUS_WONT_FIX: &'static str = "不處理";

    /// 讀檔案內容（附件、會議檔案、白板、空間檔案、問題回報截圖），有快取與限流。
    pub fn files(&self) -> Rc<FileReader> {
        self.0.files.borrow_mut().get_or_insert_with(|| Rc::new(FileReader::new(self.0.api.clone()))).clone()
    }

    /// 在工作台打開任務詳細。
    pub fn open_task_in_workspace(&self, key: &str) {
        self.request_workspace("tasks", Some(super::WorkspaceArg::text(key)));
    }

    /// 要開小視窗處理測試結果（等提示收起來再開）。
    pub fn request_test_action(&self, action: &str, key: &str, title: &str) {
        self.raise_later(
            |ev| &ev.test_action_requested,
            TestActionRequest { action: action.to_owned(), key: key.to_owned(), title: title.to_owned() },
        );
    }

    /// 指派給我、還沒修好的測試結果（待修復、修復中），面板今天分頁的卡片用。
    pub fn my_fixes(&self) -> Vec<TaskSummary> {
        self.task_list("test")
            .iter()
            .filter(|t| {
                t.kind() == TaskKind::TestResult
                    && TaskKinds::bucket(&t.status, Some(&t.status_category), None) == "fixing"
            })
            .cloned()
            .collect()
    }

    /// 指派給我、還沒測完的測試任務。
    pub fn my_tests(&self) -> Vec<TaskSummary> {
        self.task_list("test").iter().filter(|t| t.kind() == TaskKind::Test).cloned().collect()
    }

    /// 我是回報的人（或測試任務的執行者）或專案管理員：可以通過、退回、結案。
    pub fn can_retest(&self, task_key: &str, reporter: Option<&str>) -> bool {
        let s = self.s();
        let is_reporter = match (reporter.filter(|r| !r.is_empty()), &s.user) {
            (Some(r), Some(me)) => r == me.username || r == me.display_name,
            _ => false,
        };
        is_reporter
            || s.projects.iter().any(|p| {
                eq_ic(&p.key, KeyFinder::project_of(task_key))
                    && matches!(p.role.as_str(), "ADMIN" | "MANAGER" | "OWNER")
            })
    }

    /// 開始修：狀態改成修復中，可以順手填預估工時；start_timer 一起開始計時。
    pub async fn start_fix(&self, key: &str, hours: Option<f64>, start_timer: bool) -> Result<String, CoreError> {
        let mut fields = status_fields(Self::STATUS_FIXING);
        if let Some(h) = hours {
            fields.insert("estimatedHours".into(), json!(h));
        }
        let message = self.0.api.update_task(key, fields).await?;
        if start_timer && !self.running_keys().contains(key) {
            self.start_task(key).await;
        }
        log::info!("測試結果 {key} 開始修{}", hours.map(|h| format!("，預估 {h} 小時")).unwrap_or_default());
        self.after_test_change(key).await;
        Ok(non_empty(message).unwrap_or_else(|| {
            if start_timer { format!("{key} 開始修，已開始計時") } else { format!("{key} 開始修") }
        }))
    }

    /// 專案的測試子狀態（讀過的專案才有，不會打 Hayaku；沒設定子狀態是空的）。
    pub fn cached_sub_statuses(&self, task_key: &str) -> Vec<TestSubStatus> {
        self.try_get_cached_project(KeyFinder::project_of(task_key))
            .and_then(|p| p.test_sub_statuses.clone())
            .unwrap_or_default()
    }

    /// 修好了之後要到哪個狀態：修復中這一段還有下一個子狀態（例 待更新測試機）就到那個，沒有就是待復驗。
    /// 內建的修復中固定是這一段的第一列。專案還沒讀過時當作沒有子狀態。
    pub fn fix_done_step(&self, task_key: &str, status: &str) -> Option<TestSubStatus> {
        TaskKinds::next_in_stage(&self.cached_sub_statuses(task_key), Self::STATUS_FIXING, status).cloned()
    }

    /// 復驗通過後要到的子狀態：待復驗這一段還有下一個（例 待上正式機）就是那個，沒有（直接完成）是 None。
    /// 內建的待復驗固定是這一段的第一列。
    pub fn pass_step(&self, task_key: &str, status: &str) -> Option<TestSubStatus> {
        TaskKinds::next_in_stage(&self.cached_sub_statuses(task_key), Self::STATUS_RETEST, status).cloned()
    }

    /// 這個狀態可以批次推到下一步（例 待更新測試機〔已更新到測試機〕）就回傳那個子狀態；
    /// 內建狀態（修復中、待復驗、已完成）不能批次推（待復驗要一筆一筆復驗）。
    pub fn batch_step(&self, task_key: &str, status: &str) -> Option<TestSubStatus> {
        self.cached_sub_statuses(task_key)
            .into_iter()
            .find(|s| s.name == status && s.name != s.stage && s.batch_text().is_some())
    }

    /// 修好了：專案在修復中後面還有子狀態（例 待更新測試機）就改到那個，等更新完測試機再批次送復驗；
    /// 沒有就直接送復驗（待復驗，等回報的人驗）。status：目前的狀態。
    pub async fn submit_retest(&self, key: &str, status: Option<&str>) -> Result<String, CoreError> {
        let next = status.and_then(|s| self.fix_done_step(key, s));
        let target = next.as_ref().map_or(Self::STATUS_RETEST.to_owned(), |n| n.name.clone());
        let message = self.0.api.update_task(key, status_fields(&target)).await?;
        if self.running_keys().contains(key) {
            self.pause_task(key).await;
        }
        log::info!(
            "{}",
            if next.is_none() {
                format!("測試結果 {key} 送復驗")
            } else {
                format!("測試結果 {key} 修好了，改成{target}")
            }
        );
        self.after_test_change(key).await;
        Ok(non_empty(message).unwrap_or_else(|| {
            if next.is_none() { format!("{key} 已送復驗") } else { format!("{key} 修好了，改成{target}") }
        }))
    }

    /// 批次推到下一步（例 已更新到測試機：待更新測試機 → 待復驗，通知回報的人復驗）。
    /// tasks 為 None 是專案裡那個子狀態的全部；comment 是更新說明（例版本號）。
    pub async fn advance_test_results(
        &self,
        project: &str,
        from: &TestSubStatus,
        tasks: Option<&[String]>,
        comment: Option<&str>,
    ) -> Result<String, CoreError> {
        let message = self.0.api.advance_test_results(project, &from.name, tasks, comment).await?;
        let batch = from.batch_text().unwrap_or_default();
        log::info!("{project} 測試結果批次{batch}（{}）", tasks.map_or("全部".to_owned(), |t| t.join("、")));
        for key in tasks.unwrap_or_default() {
            let read: Vec<String> = self
                .notifications()
                .iter()
                .filter(|n| n.unread && n.is_test_result() && n.task_key.as_deref().is_some_and(|k| eq_ic(k, key)))
                .map(|n| n.id.clone())
                .collect();
            if !read.is_empty() {
                self.mark_notifications_read(Some(read)).await;
            }
        }
        self.after_external_change().await;
        Ok(non_empty(message).unwrap_or_else(|| match tasks {
            Some([only]) => format!("{only} {batch}"),
            _ => format!("{}的測試結果{batch}", from.name),
        }))
    }

    /// 復驗：通過（已完成；待復驗後面還有子狀態時是到下一個，例 待上正式機）或退回（待修復，輪數 +1，說明變成一則留言）。
    pub async fn retest(&self, key: &str, pass: bool, comment: Option<&str>) -> Result<String, CoreError> {
        let message = self.0.api.retest(key, pass, comment, None, None).await?;
        log::info!("測試結果 {key} 復驗：{}", if pass { "通過" } else { "退回" });
        self.after_test_change(key).await;
        Ok(non_empty(message).unwrap_or_else(|| {
            if pass { format!("{key} 復驗通過") } else { format!("{key} 已退回，等工程師再修") }
        }))
    }

    /// 不處理（結案，不修了）：只有回報的人或專案管理員能用。
    pub async fn wont_fix(&self, key: &str) -> Result<String, CoreError> {
        let message = self.0.api.update_task(key, status_fields(Self::STATUS_WONT_FIX)).await?;
        self.after_test_change(key).await;
        Ok(non_empty(message).unwrap_or_else(|| format!("{key} 標為不處理")))
    }

    /// 處理完一筆測試結果：它的未讀通知標為已讀，任務清單重抓。
    async fn after_test_change(&self, key: &str) {
        let read: Vec<String> = self
            .notifications()
            .iter()
            .filter(|n| n.unread && n.is_test_result() && n.task_key.as_deref().is_some_and(|k| eq_ic(k, key)))
            .map(|n| n.id.clone())
            .collect();
        if !read.is_empty() {
            self.mark_notifications_read(Some(read)).await;
        }
        self.after_external_change().await;
    }

    /// 從提示按的動作：結果用提示告知（成功或失敗）。
    async fn run_from_toast(&self, action: impl Future<Output = Result<String, CoreError>>, what: &str) {
        match action.await {
            Ok(message) => self.toast(ToastRequest::new(what, message, Vec::new())),
            Err(err) => {
                log::error!("{what}失敗: {err}");
                self.toast(ToastRequest::new(format!("{what}失敗"), err.message(), Vec::new()));
            }
        }
    }

    /// 測試結果的通知提示：被退回 →〔開始修〕；進待復驗 →〔通過〕〔退回〕；其他 →〔查看〕。
    /// 查看是在工作台打開那個測試結果（詳細裡也有同樣的按鈕）。
    pub(super) fn test_toast(&self, n: &NotificationItem, duration: Option<Duration>, sticky: bool) -> ToastRequest {
        let key = n.task_key.clone().unwrap_or_default();
        let title = n.text.clone();
        let mut actions = Vec::new();
        if n.is_reopened() {
            let (e, k, t) = (self.clone(), key.clone(), title.clone());
            actions.push(
                ToastAction::new("開始修", move || {
                    e.request_test_action("fix", &k, &t);
                    async {}
                })
                .primary(),
            );
        } else if n.is_ready_to_retest() {
            let (e, k) = (self.clone(), key.clone());
            actions.push(
                ToastAction::new("通過", move || {
                    let (e, k) = (e.clone(), k.clone());
                    async move {
                        let e2 = e.clone();
                        e.run_from_toast(async move { e2.retest(&k, true, None).await }, "復驗通過").await
                    }
                })
                .primary(),
            );
            let (e, k, t) = (self.clone(), key.clone(), title.clone());
            actions.push(ToastAction::new("退回", move || {
                e.request_test_action("reopen", &k, &t);
                async {}
            }));
        }
        let (e, item) = (self.clone(), n.clone());
        let primary = actions.is_empty();
        actions.push(
            ToastAction::new("查看", move || {
                let (e, n) = (e.clone(), item.clone());
                async move { e.open_notification(&n).await }
            })
            .primary_if(primary),
        );
        let who = n.actor.as_ref().map(|a| format!("{a}：")).unwrap_or_default();
        ToastRequest::new(format!("{}・{key}", n.label), format!("{who}{title}"), actions)
            .sticky(sticky || n.is_reopened() || n.is_ready_to_retest())
            .duration(duration)
            .from_bottom()
            .tag(format!("n:{}", n.id))
    }
}
