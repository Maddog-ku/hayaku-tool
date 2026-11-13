//! 對應 C# `Core/TimerEngine.Slack.cs`：Slack 上按的按鈕。失敗時回傳錯誤（由 Slack 那則訊息顯示原因），不跳本機的錯誤提示。
//!
//! C# 的 `engine.Slack`（`SlackBridge`）在 hb-slack；hb-slack 依賴 hb-core，所以這裡只定義 [`SlackLink`]，
//! App 建好 SlackBridge 後用 [`TimerEngine::set_slack`] 掛上來。

use std::any::Any;
use std::rc::Rc;

use chrono::NaiveDateTime;
use hb_api::NotificationItem;

use super::TimerEngine;
use crate::error::CoreError;

/// TimerEngine 看得到的 SlackBridge（hb-slack 實作）。設定頁要的狀態與 App 要的動作都在這裡；
/// 其他用 [`SlackLink::as_any`] 轉回實際型別。
pub trait SlackLink {
    /// 螢幕鎖定中（離開座位）：一般通知也傳。
    fn locked(&self) -> bool;
    fn set_locked(&self, locked: bool);
    /// 有存 Slack 的金鑰。
    fn configured(&self) -> bool;
    /// 連線正常、上次傳送沒失敗。
    fn healthy(&self) -> bool;
    /// 設定頁顯示的狀態。
    fn status_text(&self) -> String;
    /// 重新讀金鑰、重新連線（設定頁存了新的金鑰、中斷連線後）。
    fn restart(&self);
    /// 關閉（程式結束時）。
    fn dispose(&self);
    /// 狀態變了（C# `StatusChanged`）；回傳編號給 [`SlackLink::unsubscribe_status_changed`]。
    fn subscribe_status_changed(&self, f: Box<dyn Fn()>) -> u64;
    fn unsubscribe_status_changed(&self, id: u64);
    fn as_any(&self) -> &dyn Any;
}

impl TimerEngine {
    /// 把通知傳到 Slack、處理 Slack 上的按鈕；App 啟動時建立。
    pub fn slack(&self) -> Option<Rc<dyn SlackLink>> {
        self.0.slack.borrow().clone()
    }

    pub fn set_slack(&self, slack: Option<Rc<dyn SlackLink>>) {
        *self.0.slack.borrow_mut() = slack;
    }

    /// 要關掉某個標記的提示（在 Slack 上處理掉了）。
    pub fn dismiss_toast(&self, tag: &str) {
        self.0.events.toast_dismiss_requested.invoke(&tag.to_owned());
    }

    /// 上次抓待我審核清單的時間；比訊息晚還不在清單裡，就是已經在別處處理了。
    pub fn reviews_fetched_at(&self) -> NaiveDateTime {
        self.s().reviews_fetched
    }

    pub async fn mark_read_from_slack(&self, ids: &[String]) -> Result<(), CoreError> {
        self.0.api.mark_notifications_read(Some(ids)).await?;
        {
            let mut s = self.s_mut();
            s.notifications =
                Rc::new(
                    s.notifications
                        .iter()
                        .map(|n| {
                            if ids.contains(&n.id) {
                                NotificationItem { unread: false, ..n.clone() }
                            } else {
                                n.clone()
                            }
                        })
                        .collect(),
                );
        }
        self.raise();
        Ok(())
    }

    pub async fn archive_from_slack(&self, ids: &[String]) -> Result<(), CoreError> {
        self.0.api.archive_notifications(ids).await?;
        {
            let mut s = self.s_mut();
            s.notifications = Rc::new(s.notifications.iter().filter(|n| !ids.contains(&n.id)).cloned().collect());
        }
        self.raise();
        Ok(())
    }

    /// 核准或駁回；待審清單裡找不到就重抓一次，還是沒有表示已經處理過或被撤回了。
    pub async fn review_from_slack(
        &self,
        id: &str,
        approve: bool,
        watch: bool,
        note: Option<&str>,
    ) -> Result<(), CoreError> {
        let mut found = self.pending_reviews().iter().find(|r| r.id == id).cloned();
        if found.is_none() {
            self.refresh_reviews().await;
            found = self.pending_reviews().iter().find(|r| r.id == id).cloned();
        }
        let Some(c) = found else {
            return Err(CoreError::InvalidOperation("這筆申請已經處理過或被撤回了".to_owned()));
        };
        let reject_action = if !approve && c.r#type == "CREATE" { Some("delete") } else { None };
        self.0.api.review_change_request(&c.id, approve, watch, note, reject_action).await?;
        log::info!(
            "已從 Slack {}變更申請 {}（{} {} {}）",
            if approve { "核准" } else { "駁回" },
            c.id,
            c.type_text(),
            c.task_key.as_deref().unwrap_or(""),
            c.task_title
        );
        self.after_review(&c).await;
        Ok(())
    }
}
