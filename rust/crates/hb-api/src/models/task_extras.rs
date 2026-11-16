//! 對應 C# `Mcp/HayakuApi.TaskExtras.cs` 的 record（相依、附件、異動紀錄、文件、子任務樹）。

use chrono::NaiveDate;

use super::DateTimeOffset;

/// 相依關係的一筆。other_key／other_title 是另一端的任務；rule 是規則的說明（例「完成後才能開始」）；
/// blocking：這條相依目前擋著（前置還沒到條件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyItem {
    pub id: String,
    pub other_key: String,
    pub other_title: String,
    pub other_status: Option<String>,
    pub rule: String,
    pub blocking: bool,
}

/// predecessors：這個任務在等的；successors：在等這個任務的；related：關聯任務（不擋開始、可以跨專案）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyInfo {
    pub predecessors: Vec<DependencyItem>,
    pub successors: Vec<DependencyItem>,
    pub related: Option<Vec<DependencyItem>>,
}

impl DependencyInfo {
    pub fn related_tasks(&self) -> &[DependencyItem] {
        self.related.as_deref().unwrap_or(&[])
    }
}

/// 附件；id 給 read_file 讀內容。
#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentItem {
    pub name: String,
    pub size: Option<i64>,
    pub uploader: Option<String>,
    pub at: Option<DateTimeOffset>,
    pub url: Option<String>,
    pub id: Option<String>,
}

/// 任務異動紀錄的一則：text 已經換成中文說明。
/// 跟上 Hayaku 1.5.5：同一張變更申請的幾則（提出、核准、拒絕…）帶同一個 change_request，畫面收成一張卡。
#[derive(Debug, Clone, PartialEq)]
pub struct TaskActivityItem {
    pub at: DateTimeOffset,
    pub actor: String,
    pub text: String,
    /// 留言（不是異動）。
    pub comment: bool,
    /// 屬於哪張變更申請（changeRequestId）。
    pub change_request: Option<String>,
    /// 這則對變更申請做了什麼；提出時帶申請類型（修改／新增／刪除）。
    pub request_step: Option<RequestStep>,
}

/// 變更申請在異動紀錄裡的一步。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestStep {
    /// 提出（申請類型：修改／新增／刪除；認不得是空字串）。
    Submit(String),
    Approve,
    Reject,
    Withdraw,
    /// 修改申請內容或其他。
    Other,
}

impl RequestStep {
    /// 這一步之後申請的狀態（用詞跟 Hayaku 1.5.5 統一）。
    pub fn state_text(&self) -> &'static str {
        match self {
            Self::Approve => "已核准",
            Self::Reject => "已拒絕",
            Self::Withdraw => "已撤回",
            Self::Submit(_) | Self::Other => "待審核",
        }
    }
}

/// 任務的文件（Markdown）與白板文字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDoc {
    pub title: String,
    pub text: String,
}

/// 子任務樹攤平的一列（get_task_tree）；depth：1 是直接子任務。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTreeRow {
    pub key: String,
    pub title: String,
    pub status: String,
    pub status_category: Option<String>,
    pub depth: i32,
    pub assignees: Vec<String>,
    /// 跟上 Hayaku 1.5.5：子任務清單顯示優先級、預計完成日（概況用來判斷逾期、延誤）
    pub priority: Option<String>,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
}
