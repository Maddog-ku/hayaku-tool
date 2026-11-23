//! 對應 C# `Mcp/HayakuApi.Meetings.cs` 的 record（會議詳細、會議紀錄、檔案、白板）。

use super::DateTimeOffset;

/// 會議的參加者：狀態（會議中、已離開、還沒加入、請假）與這場會議記到的工時。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeetingAttendee {
    pub username: String,
    pub name: String,
    pub state: String,
    pub logged_minutes: i32,
}

/// 一場會議（get_meeting）。note 是主要的會議紀錄（Markdown）；recurrence 是重複規則的說明（不是重複會議為 None）；
/// can_edit：主辦、專案管理員、系統管理員才能改時間、參加者（紀錄參加者也能改）。
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingDetail {
    pub key: String,
    pub id: String,
    pub title: String,
    pub project: String,
    pub start_at: DateTimeOffset,
    pub end_at: DateTimeOffset,
    pub location: Option<String>,
    pub recurring: bool,
    pub recurrence: Option<String>,
    pub organizer: Option<String>,
    pub my_state: Option<String>,
    pub attendees: Vec<MeetingAttendee>,
    pub note: Option<String>,
    pub can_edit: bool,
    /// 綁到哪個團隊空間（沒綁是 None；跟上 Hayaku 1.5.9）
    pub space_binding: Option<SpaceBinding>,
    /// 相關任務（紀錄提到、內容提到、手動加入；跟上 Hayaku 1.5.9）
    pub tasks: Vec<RelatedTask>,
}

/// 會議紀錄的一份（一場會議可能有好幾份）。
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingNote {
    pub title: String,
    pub updated_at: Option<DateTimeOffset>,
    pub markdown: String,
}

/// 會議紀錄的一版更新（get_meeting_note_history）：誰、何時、摘要與差異（＋新增、－刪除、～改了）。
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingNoteVersion {
    pub editor: String,
    pub at: Option<DateTimeOffset>,
    pub summary: String,
    pub diff: String,
}

/// 會議的檔案：source 是 ATTACHMENT（會議附件）、CHAT（留言的附件）或紀錄裡上傳的。
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingFile {
    pub id: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub source: String,
    pub uploaded_by: Option<String>,
    pub uploaded_at: Option<DateTimeOffset>,
}

impl MeetingFile {
    pub fn source_text(&self) -> &str {
        match self.source.as_str() {
            "ATTACHMENT" => "會議附件",
            "CHAT" => "留言",
            "NOTE" | "NOTES" => "會議紀錄",
            "" => "",
            other => other,
        }
    }
}

/// 白板：文字（由上到下、由左到右）、圖形數量、圖片數、快照數。MCP 不能改白板。
/// id 給 read_file 看縮圖，image_ids 是白板上的圖片（read_file 看圖）。
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingWhiteboard {
    pub title: String,
    pub updated_at: Option<DateTimeOffset>,
    pub text: Vec<String>,
    pub element_count: i32,
    pub image_count: i32,
    pub snapshots: i32,
    pub id: Option<String>,
    pub image_ids: Option<Vec<String>>,
}

/// 會議綁到的團隊空間（get_meeting／get_meeting_notes 的 spaceBinding；跟上 Hayaku 1.5.9）。
/// 紀錄、白板也在那個空間的會議資料夾裡，跟會議頁同一份。沒有實際資料，欄位名稱照說明寬鬆猜。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceBinding {
    pub space_id: Option<String>,
    pub space_name: Option<String>,
    /// 空間裡的會議資料夾頁（get_space_page 給這個 id 回場次清單）
    pub folder_id: Option<String>,
}

impl SpaceBinding {
    /// 畫面上顯示的名稱：空間名稱，沒有就「已綁定」。
    pub fn label(&self) -> String {
        self.space_name.clone().unwrap_or_else(|| "已綁定".to_owned())
    }
}

/// 會議和任務怎麼關聯起來的（紀錄提到、內容提到、手動加入）→ 中文；看不懂的照原字。
pub fn link_source_text(source: &str) -> String {
    match source.trim().to_uppercase().replace(['-', ' '], "_").as_str() {
        "NOTE" | "NOTES" | "MEETING_NOTE" | "MEETING_NOTES" | "NOTE_MENTION" | "MENTIONED_IN_NOTE" | "MINUTES" => {
            "紀錄提到".to_owned()
        }
        "CONTENT" | "TASK_CONTENT" | "CARD" | "CONTENT_MENTION" | "MENTIONED_IN_CONTENT" | "EMBED" => {
            "內容提到".to_owned()
        }
        "MANUAL" | "MANUALLY" | "LINKED" | "LINK" => "手動加入".to_owned(),
        _ => source.trim().to_owned(),
    }
}

/// 任務詳情的「會議」：紀錄插入過這個任務、或任務內容插入了會議卡片、或手動加入的會議（get_task 的 meetings）。
/// 看不到的只有 id（visible 是 false）。
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedMeeting {
    /// 會議編號（REVIVAL-9）或 id，打開會議用
    pub key: String,
    pub id: Option<String>,
    pub title: String,
    pub project: Option<String>,
    pub start_at: Option<DateTimeOffset>,
    pub end_at: Option<DateTimeOffset>,
    /// 中文：紀錄提到、內容提到、手動加入
    pub sources: Vec<String>,
    pub visible: bool,
}

/// 會議詳情的「相關任務」：這場會議討論過的任務（get_meeting 的 tasks）。看不到的只有 id（visible 是 false）。
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedTask {
    pub key: String,
    pub title: String,
    pub status: String,
    pub status_category: String,
    pub assignees: Vec<String>,
    pub planned_end: Option<chrono::NaiveDate>,
    /// 中文：紀錄提到、內容提到、手動加入
    pub sources: Vec<String>,
    pub visible: bool,
}
