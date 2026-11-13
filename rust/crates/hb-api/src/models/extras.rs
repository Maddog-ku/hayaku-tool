//! 對應 C# `Mcp/HayakuApi.Extras.cs` 的 record（會議、待辦、提醒、通知）。

use super::DateTimeOffset;

#[derive(Debug, Clone, PartialEq)]
pub struct MeetingItem {
    pub key: String,
    pub id: String,
    pub title: String,
    pub project: String,
    pub start_at: DateTimeOffset,
    pub end_at: DateTimeOffset,
    pub location: Option<String>,
    pub my_status: Option<String>,
}

impl MeetingItem {
    pub fn in_meeting(&self) -> bool {
        self.my_status.as_deref() == Some("會議中")
    }

    /// 我專案裡的會議，但我不是參加者。
    pub fn not_attendee(&self) -> bool {
        self.my_status.as_deref() == Some("不是參加者")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TodoItem {
    pub id: String,
    pub text: String,
    pub task_code: Option<String>,
    pub remind_at: Option<DateTimeOffset>,
    pub source: Option<String>,
    pub assigned_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReminderItem {
    pub id: String,
    pub text: String,
    pub at: Option<DateTimeOffset>,
    pub task: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NotificationItem {
    pub id: String,
    pub r#type: String,
    pub label: String,
    pub text: String,
    pub actor: Option<String>,
    pub task_key: Option<String>,
    pub project_key: Option<String>,
    pub meeting_id: Option<String>,
    pub created_at: DateTimeOffset,
    pub unread: bool,
    pub count: i32,
    pub feedback: Option<i32>,
    pub ids: Option<Vec<String>>,
    pub users: Option<Vec<String>>,
    pub resolved: Option<bool>,
    pub quote: Option<String>,
}

impl NotificationItem {
    /// C# 建構子的必填欄位；Feedback、Ids、Users、Resolved、Quote 預設 null。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        r#type: impl Into<String>,
        label: impl Into<String>,
        text: impl Into<String>,
        actor: Option<String>,
        task_key: Option<String>,
        project_key: Option<String>,
        meeting_id: Option<String>,
        created_at: DateTimeOffset,
        unread: bool,
        count: i32,
    ) -> Self {
        Self {
            id: id.into(),
            r#type: r#type.into(),
            label: label.into(),
            text: text.into(),
            actor,
            task_key,
            project_key,
            meeting_id,
            created_at,
            unread,
            count,
            feedback: None,
            ids: None,
            users: None,
            resolved: None,
            quote: None,
        }
    }

    /// 同一組的全部通知 id（討論分頁一段討論一組；收件匣只有自己）。
    pub fn all_ids(&self) -> Vec<String> {
        match &self.ids {
            Some(ids) if !ids.is_empty() => ids.clone(),
            _ => vec![self.id.clone()],
        }
    }

    fn type_has(&self, word: &str) -> bool {
        self.r#type.to_uppercase().contains(word)
    }

    /// 測試結果的通知（被退回、進待復驗…）：代碼裡有 TEST，或標籤是測試結果。
    pub fn is_test_result(&self) -> bool {
        self.type_has("TEST")
    }

    /// 測試結果被退回（要再修）。
    pub fn is_reopened(&self) -> bool {
        self.is_test_result() && (self.type_has("REOPEN") || self.type_has("REJECT") || self.type_has("RETURN"))
    }

    /// 測試結果修好了、等復驗（回報的人要驗）。
    pub fn is_ready_to_retest(&self) -> bool {
        self.is_test_result()
            && !self.is_reopened()
            && (self.type_has("RETEST") || self.type_has("VERIFY") || self.type_has("READY"))
    }
}
