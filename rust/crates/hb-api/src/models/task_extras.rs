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
    /// 文件 id（get_content 的 documents[].id）；寫入時給它（跟上 Hayaku 1.5.12）。白板是 None
    pub id: Option<String>,
    /// 白板：只有文字清單，不能寫
    pub whiteboard: bool,
}

/// 文件裡可以單獨換掉的一段（update_content_document 的 section）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSection {
    /// 標題文字（不含 #）
    pub heading: String,
    /// 標題底下那一段目前的內容（不含標題那一行）
    pub body: String,
}

impl ContentDoc {
    /// 可以只換一段的標題：底下沒有子標題、文件裡沒有同名標題的才列
    /// （同名標題伺服器會回錯誤；有子標題的「那一段」範圍說明沒寫清楚，不列，避免蓋掉子段落）。
    /// 程式碼區塊（``` 或 ~~~）裡的 # 不算標題。
    pub fn editable_sections(&self) -> Vec<DocSection> {
        if self.whiteboard {
            return Vec::new();
        }
        // (層級, 標題, 標題那一行的索引)
        let lines: Vec<&str> = self.text.lines().collect();
        let mut heads: Vec<(usize, String, usize)> = Vec::new();
        let mut fence: Option<&str> = None;
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            let marker = if t.starts_with("```") { Some("```") } else if t.starts_with("~~~") { Some("~~~") } else { None };
            if let Some(m) = marker {
                match fence {
                    Some(f) if f == m => fence = None,
                    None => fence = Some(m),
                    _ => {}
                }
                continue;
            }
            if fence.is_some() || line.starts_with("    ") || line.starts_with('\t') {
                continue;
            }
            let level = t.chars().take_while(|c| *c == '#').count();
            if !(1..=6).contains(&level) {
                continue;
            }
            let rest = &t[level..];
            if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
                continue;
            }
            let heading = rest.trim().trim_end_matches('#').trim().to_owned();
            if !heading.is_empty() {
                heads.push((level, heading, i));
            }
        }
        let mut out = Vec::new();
        for (n, (level, heading, at)) in heads.iter().enumerate() {
            if heads.iter().filter(|h| h.1 == *heading).count() > 1 {
                continue;
            }
            let next = heads.get(n + 1);
            if next.is_some_and(|h| h.0 > *level) {
                continue;
            }
            let end = next.map_or(lines.len(), |h| h.2);
            let body = lines[at + 1..end].join("\n").trim_matches('\n').to_owned();
            out.push(DocSection { heading: heading.clone(), body });
        }
        out
    }
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
