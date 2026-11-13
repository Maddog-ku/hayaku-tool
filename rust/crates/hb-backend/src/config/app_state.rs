//! 對應 C# `Config/AppState.cs`：執行狀態，存在 %APPDATA%\HayakuBar\state.json，重開工具後延續。

use std::path::Path;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::ci_map::CiMap;
use crate::infra::app_paths::AppPaths;
use crate::infra::json_file;

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppState {
    /// 下面幾個「當天」欄位所屬的日期（yyyy-MM-dd）。
    pub date: String,

    /// 今天已處理過的班表時間點（S0＝第一段上班開始、E0＝第一段結束…）。
    pub handled_boundaries: Vec<String>,

    /// 今天手動按了下班：當天不再自動上下班。
    pub clocked_out: bool,

    /// 下一次上班／午休結束時要恢復的任務（跨日保留）。
    pub resume_set: Vec<String>,

    /// 由 Claude 偵測自動開始的計時（面板上標 ✦）。
    pub claude_owned: Vec<String>,

    /// 加入會議時暫停的計時，離開會議後恢復；None＝沒有。
    pub meeting_pause: Option<MeetingPause>,

    /// Claude session → 使用者手動指定的任務。
    pub session_tasks: IndexMap<String, String>,

    /// 問過「要不要建立任務」的 Claude session，不再重複問。
    pub asked_sessions: Vec<String>,

    /// 資料夾 → 在那裡做事時用的 Hayaku 專案與上次新任務的上層，建立任務時自動帶入（鍵不分大小寫）。
    pub folder_hints: CiMap<FolderHint>,

    /// 接下來不用上班的日子（yyyy-MM-dd → 「請假」、假日名稱，週末等是空字串），每天下班時更新。
    /// 這些日子不自動跟 Hayaku、EIP 要任何資料（按重新整理才會）。
    pub off_days: IndexMap<String, String>,

    /// OffDays 是哪天下班時更新的（yyyy-MM-dd）。
    pub off_days_updated: String,

    /// 上次登入的帳號與顯示名稱：不上班的日子不問 Hayaku，用這個顯示。
    pub user_name: Option<String>,
    pub user_display_name: Option<String>,
}

impl AppState {
    /// 換日時重設當天欄位；有換日回傳 true。
    pub fn roll_day(&mut self, today: &str) -> bool {
        if self.date == today {
            return false;
        }
        self.date = today.to_owned();
        self.handled_boundaries.clear();
        self.clocked_out = false;
        self.claude_owned.clear();
        self.meeting_pause = None;
        if self.session_tasks.len() > 200 {
            self.session_tasks.clear();
        }
        if self.asked_sessions.len() > 200 {
            let remove = self.asked_sessions.len() - 100;
            self.asked_sessions.drain(..remove);
        }
        true
    }

    /// 讀 %APPDATA%\HayakuBar\state.json（沒有或讀不懂用預設值）。
    pub fn load() -> Self {
        Self::load_from(&AppPaths::state())
    }

    pub fn save(&self) {
        self.save_to(&AppPaths::state());
    }

    pub fn load_from(path: &Path) -> Self {
        json_file::load(path)
    }

    pub fn save_to(&self, path: &Path) {
        json_file::save(path, self);
    }
}

/// 加入會議時暫停的計時：Meeting＝會議的 key，MeetingTasks＝會議在計時的任務（停了就恢復 Tasks）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MeetingPause {
    pub meeting: String,
    pub meeting_tasks: Vec<String>,
    pub tasks: Vec<String>,
}

/// 某個資料夾對應的 Hayaku 專案；Parent 是上次在這裡建立新任務時選的上層（None＝最上層）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FolderHint {
    pub project: String,
    pub parent: Option<String>,
}
