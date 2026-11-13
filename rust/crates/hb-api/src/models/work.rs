//! 對應 C# `Mcp/HayakuApi.Work.cs` 的 record（任務、提醒、請假、團隊、工作日誌、聊天）。

use chrono::NaiveDate;
use std::fmt;

use super::{DateTimeOffset, TaskKind, TestInfo, TestSubStatus};

/// 我的任務列表的一筆（my_tasks）；ancestors 是由上而下的上層任務。
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSummary {
    pub key: String,
    pub title: String,
    pub project: String,
    pub status: String,
    pub status_category: String,
    pub priority: String,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
    pub timing: bool,
    pub path: String,
    pub ancestors: Option<Vec<(String, String)>>,
    pub child_count: Option<i32>,
    pub test: TestInfo,
}

impl TaskSummary {
    /// C# 建構子的必填欄位；Ancestors、ChildCount 預設 null，Test 預設 default。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        project: impl Into<String>,
        status: impl Into<String>,
        status_category: impl Into<String>,
        priority: impl Into<String>,
        planned_start: Option<NaiveDate>,
        planned_end: Option<NaiveDate>,
        timing: bool,
        path: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            project: project.into(),
            status: status.into(),
            status_category: status_category.into(),
            priority: priority.into(),
            planned_start,
            planned_end,
            timing,
            path: path.into(),
            ancestors: None,
            child_count: None,
            test: TestInfo::default(),
        }
    }

    pub fn kind(&self) -> TaskKind {
        self.test.kind
    }

    /// 沒有子任務（真正要做的事）；舊版回應沒有子任務數時當作沒有。
    pub fn is_leaf(&self) -> bool {
        matches!(self.child_count, None | Some(0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PersonRef {
    pub username: String,
    pub display_name: String,
}

impl PersonRef {
    pub fn new(username: impl Into<String>, display_name: impl Into<String>) -> Self {
        Self { username: username.into(), display_name: display_name.into() }
    }
}

/// 專案裡的一個任務（list_tasks）；parent 是直接上層的編號，最上層為 None。
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectTask {
    pub key: String,
    pub title: String,
    pub status: String,
    pub status_category: String,
    pub parent: Option<String>,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
    pub is_category: bool,
    pub assignees: Option<Vec<String>>,
    pub priority: Option<String>,
    pub test: TestInfo,
}

impl ProjectTask {
    /// C# 建構子的必填欄位；IsCategory 預設 false，Assignees、Priority 預設 null，Test 預設 default。
    pub fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        status: impl Into<String>,
        status_category: impl Into<String>,
        parent: Option<String>,
        planned_start: Option<NaiveDate>,
        planned_end: Option<NaiveDate>,
    ) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            status: status.into(),
            status_category: status_category.into(),
            parent,
            planned_start,
            planned_end,
            is_category: false,
            assignees: None,
            priority: None,
            test: TestInfo::default(),
        }
    }

    pub fn kind(&self) -> TaskKind {
        self.test.kind
    }
}

/// 任務完整資料（get_task）。
#[derive(Debug, Clone, PartialEq)]
pub struct TaskDetail {
    pub key: String,
    pub title: String,
    pub status: String,
    pub status_category: String,
    pub priority: String,
    pub assignees: Vec<PersonRef>,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
    pub actual_start: Option<NaiveDate>,
    pub actual_end: Option<NaiveDate>,
    pub estimated_hours: Option<f64>,
    pub logged_text: String,
    pub note: Option<String>,
    pub approval_state: String,
    pub path: Vec<(String, String)>,
    /// (Key, Title, Status)
    pub children: Vec<(String, String, String)>,
    pub pending_change_requests: i32,
    pub logged_minutes: Option<f64>,
    pub is_category: bool,
    pub content_block_count: i32,
    pub test: TestInfo,
    pub created_by: Option<String>,
}

impl TaskDetail {
    pub fn kind(&self) -> TaskKind {
        self.test.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusOption {
    pub name: String,
    pub category: String,
    pub is_default: bool,
}

/// 專案；icon、icon_color 是 Hayaku 的圖示與顏色名稱（例 rocket、moss）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    pub key: String,
    pub name: String,
    pub role: String,
    pub icon: Option<String>,
    pub icon_color: Option<String>,
    pub is_archived: bool,
}

impl ProjectInfo {
    pub fn new(key: impl Into<String>, name: impl Into<String>, role: impl Into<String>) -> Self {
        Self { key: key.into(), name: name.into(), role: role.into(), icon: None, icon_color: None, is_archived: false }
    }

    /// 專案管理員或編輯者：可以審核變更申請。
    pub fn can_review(&self) -> bool {
        matches!(self.role.as_str(), "ADMIN" | "MANAGER" | "OWNER" | "EDITOR")
    }
}

impl fmt::Display for ProjectInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// 專案的成員、狀態清單；test_sub_statuses：測試結果的子狀態（沒設定是空的）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDetail {
    pub key: String,
    pub name: String,
    pub members: Vec<PersonRef>,
    pub statuses: Vec<StatusOption>,
    pub test_sub_statuses: Option<Vec<TestSubStatus>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub key: String,
    pub title: String,
    pub project: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReminderFull {
    pub id: String,
    pub text: String,
    pub at: Option<DateTimeOffset>,
    pub task: Option<String>,
    pub repeat_text: Option<String>,
    pub paused: bool,
    pub editable: bool,
}

/// 請假或出差（list_leaves）；kind：「請假」「出差」（或 LEAVE、TRIP），舊的回應沒有。
#[derive(Debug, Clone, PartialEq)]
pub struct LeaveItem {
    pub id: String,
    pub user: PersonRef,
    pub period: String,
    pub start_at: DateTimeOffset,
    pub end_at: DateTimeOffset,
    pub lost_text: String,
    pub kind: Option<String>,
}

impl LeaveItem {
    /// 出差：照常算上班時數。
    pub fn is_trip(&self) -> bool {
        self.kind.as_deref().is_some_and(|k| k.contains("出差") || k.eq_ignore_ascii_case("TRIP"))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TeamTiming {
    pub task: String,
    pub title: String,
    pub project: String,
    pub since: Option<DateTimeOffset>,
    pub can_open: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TeamMember {
    pub user: PersonRef,
    pub job_title: Option<String>,
    pub online: bool,
    pub on_leave: bool,
    pub leaves: Vec<String>,
    pub timing: Vec<TeamTiming>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkLogEntry {
    pub task: String,
    pub minutes: i32,
    /// (Start, End)
    pub ranges: Vec<(DateTimeOffset, DateTimeOffset)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkLog {
    pub date: NaiveDate,
    pub total_minutes: i32,
    pub entries: Vec<WorkLogEntry>,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub author: PersonRef,
    pub body: String,
    pub created_at: DateTimeOffset,
    pub deleted: bool,
    pub poll_question: Option<String>,
    pub poll_id: Option<String>,
}
