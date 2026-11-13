//! 對應 C# `Mcp/HayakuApi.Projects.cs` 的 record（釘選托盤、變更申請、專案動態、工作負荷）。

use chrono::{NaiveDate, TimeDelta};

use super::{DateTimeOffset, PersonRef};

/// 釘在托盤上的任務或會議（list_pins）：今天累積、是否正在計時、能不能計時。
/// 會議（is_meeting）多了起訖時間與是否已取消；▶ 是加入會議（開會前 10 分鐘起），已取消、已結束的不能加入。
#[derive(Debug, Clone, PartialEq)]
pub struct PinItem {
    pub key: String,
    pub title: String,
    pub project: String,
    pub status: String,
    pub today_minutes: i32,
    pub running: bool,
    pub can_time: bool,
    pub is_meeting: bool,
    pub start_at: Option<DateTimeOffset>,
    pub end_at: Option<DateTimeOffset>,
    pub cancelled: bool,
}

impl PinItem {
    /// 會議開始前多久可以加入（跟網頁一樣 10 分鐘）。
    pub const JOIN_LEAD: TimeDelta = TimeDelta::minutes(10);

    pub fn ended(&self, now: DateTimeOffset) -> bool {
        self.end_at.is_some_and(|end| now >= end)
    }

    /// 會議現在能不能從托盤加入：沒取消、沒結束、開始前 10 分鐘起。
    pub fn can_join(&self, now: DateTimeOffset) -> bool {
        self.is_meeting
            && !self.cancelled
            && !self.ended(now)
            && self.start_at.is_none_or(|start| now >= start - Self::JOIN_LEAD)
    }
}

/// 變更申請（list_change_requests）。type：CREATE、UPDATE、DELETE；status：PENDING、APPROVED、REJECTED、WITHDRAWN。
/// changes：改了哪些欄位（已換成中文名稱與值）。
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeRequest {
    pub id: String,
    pub project: String,
    pub r#type: String,
    pub status: String,
    pub task_key: Option<String>,
    pub task_title: String,
    pub requester: Option<PersonRef>,
    pub reviewer: Option<PersonRef>,
    pub created_at: DateTimeOffset,
    pub changes: Vec<String>,
    pub reason: Option<String>,
    pub review_note: Option<String>,
    pub logged_minutes: i32,
    pub descendant_count: i32,
}

impl ChangeRequest {
    pub fn type_text(&self) -> &str {
        match self.r#type.as_str() {
            "CREATE" => "新增任務",
            "UPDATE" => "修改任務",
            "DELETE" => "刪除任務",
            other => other,
        }
    }
}

/// 專案動態的一則（list_project_activity）；at 是伺服器排好的顯示文字（例「10/30 13:41」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityItem {
    pub at: String,
    pub project: String,
    pub text: String,
}

/// 專案動態的摘要數字與最活躍的人。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityFeed {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub created: i32,
    pub started: i32,
    pub completed: i32,
    pub overdue: i32,
    /// (Name, Count)
    pub top_members: Vec<(String, i32)>,
    pub items: Vec<ActivityItem>,
}

/// 工作負荷的一天：分配到或實際計時的分鐘、當天的容量（應出勤）、做了幾個任務。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadDay {
    pub date: NaiveDate,
    pub minutes: i32,
    pub capacity_minutes: i32,
    pub task_count: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkloadTask {
    pub key: String,
    pub title: String,
    pub minutes: i32,
}

/// 某人在一段期間的工作負荷（get_workload）；mode：planned 預計、actual 實際計時。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workload {
    pub mode: String,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub total_minutes: i32,
    pub capacity_minutes: i32,
    pub days: Vec<WorkloadDay>,
    pub tasks: Vec<WorkloadTask>,
    /// (Key, Title, Reason)
    pub incomplete: Vec<(String, String, String)>,
}
