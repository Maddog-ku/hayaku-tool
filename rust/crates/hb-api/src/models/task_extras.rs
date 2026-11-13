//! 對應 C# `Mcp/HayakuApi.TaskExtras.cs` 的 record（相依、附件、異動紀錄、文件、子任務樹）。

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
#[derive(Debug, Clone, PartialEq)]
pub struct TaskActivityItem {
    pub at: DateTimeOffset,
    pub actor: String,
    pub text: String,
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
}
