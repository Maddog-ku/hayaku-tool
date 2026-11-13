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
