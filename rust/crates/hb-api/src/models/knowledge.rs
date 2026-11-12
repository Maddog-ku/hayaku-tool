//! 對應 C# `Mcp/HayakuApi.Knowledge.cs` 的 record（筆記本、團隊空間、投票、論壇）。

use super::DateTimeOffset;

/// 我的筆記本裡的資料夾。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFolder {
    pub id: String,
    pub name: String,
}

/// 我的筆記本的一頁（清單用）；task_codes：相關任務編號。
#[derive(Debug, Clone, PartialEq)]
pub struct NotePage {
    pub id: String,
    pub title: String,
    pub folder_id: Option<String>,
    pub updated_at: Option<DateTimeOffset>,
    pub task_codes: Vec<String>,
}

/// 筆記本：資料夾、頁、隨手記與待辦的數字（list_my_notes）。
#[derive(Debug, Clone, PartialEq)]
pub struct Notebook {
    pub folders: Vec<NoteFolder>,
    pub pages: Vec<NotePage>,
    pub inbox: i32,
    pub todos: i32,
}

/// 筆記本的一頁內容（get_my_note）；todos：這頁的待辦與勾了沒。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteContent {
    pub id: String,
    pub title: String,
    pub markdown: String,
    pub task_codes: Vec<String>,
    /// (Text, Done)
    pub todos: Vec<(String, bool)>,
}

/// 筆記本、團隊空間的搜尋結果；space 只有團隊空間才有。
/// folder_id 有值是資料夾裡的檔案（search_spaces 的 files：id 是檔案 id，snippet 是所在路徑）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeHit {
    pub id: String,
    pub title: String,
    pub space: Option<String>,
    pub snippet: Option<String>,
    pub folder_id: Option<String>,
}

impl KnowledgeHit {
    pub fn is_file(&self) -> bool {
        self.folder_id.is_some()
    }
}

/// 團隊空間（list_spaces）；project：所屬專案的代碼（專案裡開的空間），archived：專案封存了、只能看。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceInfo {
    pub key: String,
    pub name: String,
    pub role: Option<String>,
    pub page_count: Option<i32>,
    pub project: Option<String>,
    pub archived: bool,
}

/// 空間頁面樹的一個節點（get_space_tree）；kind：DOC 文件、BOARD 白板、FOLDER 資料夾、CHANNEL 頻道、FORUM 論壇
/// （Hayaku 1.4 以前的檔案庫是 FILES，已經轉成資料夾、id 不變）。file_count：資料夾直接放在裡面的檔案數。
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceNode {
    pub id: String,
    pub title: String,
    pub parent_id: Option<String>,
    pub kind: Option<String>,
    pub updated_at: Option<DateTimeOffset>,
    pub file_count: Option<i32>,
}

impl SpaceNode {
    fn kind_upper(&self) -> Option<String> {
        self.kind.as_deref().map(str::to_uppercase)
    }

    pub fn is_folder(&self) -> bool {
        matches!(self.kind_upper().as_deref(), Some("FOLDER" | "FILES"))
    }

    /// 頁面樹上的圖示：資料夾、白板、頻道、論壇；文件不加。
    pub fn icon(&self) -> &'static str {
        match self.kind_upper().as_deref() {
            Some("FOLDER" | "FILES") => "📁 ",
            Some("BOARD") => "🖍 ",
            Some("CHANNEL") => "# ",
            Some("FORUM") => "💬 ",
            _ => "",
        }
    }
}

/// 空間的一頁（get_space_page）；path：上層的頁名。
#[derive(Debug, Clone, PartialEq)]
pub struct SpacePageContent {
    pub id: String,
    pub title: String,
    pub markdown: String,
    pub owner: Option<String>,
    pub updated_at: Option<DateTimeOffset>,
    pub path: Vec<String>,
}

/// 資料夾裡的一個東西（list_space_files）：上傳的檔案，或子頁（kind：FOLDER 子資料夾、DOC 文件、BOARD 白板…）。
/// path：遞迴列出時的所在路徑。
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceFile {
    pub id: String,
    pub name: String,
    pub is_folder: bool,
    pub size: Option<i64>,
    pub uploader: Option<String>,
    pub at: Option<DateTimeOffset>,
    pub kind: Option<String>,
    pub path: Option<String>,
}

impl SpaceFile {
    /// 子頁（不是上傳的檔案）。
    pub fn is_page(&self) -> bool {
        self.kind.is_some()
    }
}

/// 投票的一個選項；voters 匿名時是空的，mine：我投了這個。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollOption {
    pub label: String,
    pub votes: Option<i32>,
    pub voters: Vec<String>,
    pub mine: bool,
}

/// 投票（get_poll，唯讀）；votes 是 None 表示「投了才看得到結果」而我還沒投。
#[derive(Debug, Clone, PartialEq)]
pub struct PollInfo {
    pub question: String,
    pub place: Option<String>,
    pub r#type: Option<String>,
    pub anonymous: bool,
    pub status: Option<String>,
    pub closes_at: Option<DateTimeOffset>,
    pub created_by: Option<String>,
    pub options: Vec<PollOption>,
    pub voted: Option<i32>,
    pub pending: Option<i32>,
}

/// 論壇貼文（list_forum_posts）。
#[derive(Debug, Clone, PartialEq)]
pub struct ForumPost {
    pub id: String,
    pub title: String,
    pub author: String,
    pub created_at: Option<DateTimeOffset>,
    pub replies: i32,
    pub last_activity_at: Option<DateTimeOffset>,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub solved: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForumMessage {
    pub id: String,
    pub author: String,
    pub body: String,
    pub created_at: Option<DateTimeOffset>,
    pub deleted: bool,
}

/// 一篇貼文與底下的回應（get_forum_post）。
#[derive(Debug, Clone, PartialEq)]
pub struct ForumThread {
    pub title: String,
    pub root: Option<ForumMessage>,
    pub replies: Vec<ForumMessage>,
    pub solved: bool,
    pub tags: Vec<String>,
}
