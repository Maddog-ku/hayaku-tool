//! 對應 C# `Config/AppConfig.cs`：使用者設定，存在 %APPDATA%\HayakuBar\config.json。
//! JSON 跟 C# 版完全相容：camelCase、缺欄位用 C# 的預設值、`[JsonIgnore]` 的屬性不輸出。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::infra::app_paths::AppPaths;
use crate::infra::json_file;

/// 主資料夾（例 D:\repo）。裡面的專案會自動偵測。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RepoConfig {
    pub path: String,
    pub enabled: bool,
    /// 自動偵測裡面的專案（子資料夾是 git repo，或 Claude 曾在裡面開過 session）。
    pub include_subprojects: bool,
    /// 主資料夾本身的 session 改了程式、又對應不到任務時，詢問要不要建立任務。
    pub auto_create: bool,
    /// 子專案的個別設定（只存有改過的）。
    pub projects: Vec<ProjectConfig>,
}

impl Default for RepoConfig {
    fn default() -> Self {
        Self { path: String::new(), enabled: true, include_subprojects: true, auto_create: false, projects: Vec::new() }
    }
}

/// 主資料夾裡某個專案的個別設定，用資料夾名稱對應。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectConfig {
    pub name: String,
    pub enabled: bool,
    /// Claude 在這個專案改了程式、又對應不到任務時，詢問要不要建立任務。
    pub auto_create: bool,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self { name: String::new(), enabled: true, auto_create: false }
    }
}

impl ProjectConfig {
    /// 跟預設值一樣就不用存（C# `[JsonIgnore] IsDefault`，不寫進 JSON）。
    pub fn is_default(&self) -> bool {
        self.enabled && !self.auto_create
    }
}

/// 我的任務清單的一欄：Key 見 TaskColumns.All；Width 是 DIP（名稱欄 0＝填滿剩下的寬度）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskColumnConfig {
    pub key: String,
    pub visible: bool,
    #[serde(with = "json_file::double")]
    pub width: f64,
}

impl Default for TaskColumnConfig {
    fn default() -> Self {
        Self { key: String::new(), visible: true, width: 0.0 }
    }
}

/// 使用者設定，存在 %APPDATA%\HayakuBar\config.json。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub base_url: String,

    /// 依 Hayaku 班表自動上班／午休暫停／午休後恢復／下班停止。
    pub auto_schedule: bool,

    pub claude_enabled: bool,
    pub claude_only_during_work: bool,
    /// session 紀錄在幾分鐘內有變動才算「Claude 正在動」，用來決定要不要開始計時（不會因此暫停）。
    pub claude_idle_minutes: i32,

    /// 外觀：System（跟隨 Windows）、Light、Dark。
    pub theme_mode: String,

    /// 面板內容區的高度（DIP），切分頁時不會跳；0 表示跟著內容伸縮（最高 440）。
    pub panel_height: i32,

    /// 工作台「我的任務」的排序：default、due、dueDesc、priority、status、key、timing。
    pub task_sort: String,

    /// 工作台「我的任務」的顯示方式：list（清單）、tree（依專案分組的樹狀）、kanban（看板）、calendar（日曆）或 gantt（甘特圖）。
    pub task_view: String,

    /// 工作台「我的任務」只看哪個專案（專案名稱）；空字串＝全部專案。
    pub task_project: String,

    /// 我的任務（工作台與面板）只顯示沒有子任務的，只看真正要做的事。
    pub task_leaf_only: bool,

    /// 看板不顯示完成及不處理（已取消類）的任務。
    pub task_hide_done: bool,

    /// 專案樹（新增任務、移動任務、自動建立任務）不顯示完成及不處理的任務。
    pub tree_hide_done: bool,

    /// 工作台「我的任務」清單的欄位（順序就是顯示順序）；空的用預設。
    pub task_columns: Vec<TaskColumnConfig>,

    /// 清單的專案欄只顯示代碼（例 F2P）。
    pub task_project_code_only: bool,

    pub poll_seconds: i32,

    /// 只在上班時間（或有計時在跑）自動同步；下班、休息、放假不跟 Hayaku 要資料。
    pub sync_only_during_work: bool,

    /// 開關面板的全域快捷鍵，例 Win+Shift+H；空字串表示不用。
    pub hotkey: String,

    /// 會議開始前幾分鐘跳出提示（可一鍵加入並計時）。
    pub meeting_alerts: bool,
    pub meeting_lead_minutes: i32,

    /// 也提醒我專案裡、我不是參加者的會議（加入時 Hayaku 會自動把我加為參加者）。
    pub meeting_alerts_all_projects: bool,

    /// 按「加入並計時」時先暫停其他正在跑的計時，離開會議（或會議計時停了）後自動恢復。
    pub meeting_pauses_others: bool,

    /// 提醒到點時跳出提示（可稍後再提醒）。
    pub reminder_alerts: bool,

    /// 有新通知時跳出提示。
    pub notification_alerts: bool,

    /// 新通知的彈窗停留幾秒後自動收回；0 表示不自動收回。
    pub notification_popup_seconds: i32,

    /// 新增提醒的快速時間「今天下班前」是下班前幾分鐘（跟上 Hayaku 1.5.4；C# 版沒有）。
    pub reminder_before_end_minutes: i32,

    /// Claude 在改程式卻對應不到任務時跳出的「要建立任務嗎？」提示，停留幾秒後自動收起；0 表示要按掉才收起
    /// （C# 版沒有這個設定，是要按掉才收起）。
    pub new_work_toast_seconds: i32,

    /// 全螢幕、簡報、Windows 安靜時段或鎖定時先不跳提示，結束後再一起顯示（會議快開始除外）。
    pub quiet_when_busy: bool,

    /// 下班時工時沒滿應出勤，提示補登。
    pub prompt_unfilled: bool,
    pub hook_port: i32,

    /// 每天下班時讀 EIP 的假單，把下一個上班日的請假登記到 Hayaku（要先在設定存 EIP 帳密）。
    pub eip_leave_sync: bool,

    /// 把通知傳到 Slack 私訊（要先在設定連好 Slack App）：要動手處理的一律傳，其他的只在螢幕鎖定時傳。
    pub slack_forward: bool,

    pub repos: Vec<RepoConfig>,

    /// 在工作列上的基準位置：Left（最左邊）、BeforeStart（圖示左側）、AfterIcons（圖示右側）、BeforeTray（通知區域左側）。
    pub taskbar_anchor: String,

    /// 工作列上的顯示方式：TwoLine（兩排，較窄）或 OneLine（一排，較寬）。
    pub taskbar_layout: String,

    /// 拖曳微調的偏移量（DIP，正數往右）。
    #[serde(with = "json_file::double")]
    pub taskbar_offset: f64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            base_url: "https://hayaku.revivalsi.com".into(),
            auto_schedule: true,
            claude_enabled: true,
            claude_only_during_work: true,
            claude_idle_minutes: 10,
            theme_mode: "System".into(),
            panel_height: 440,
            task_sort: "default".into(),
            task_view: "list".into(),
            task_project: String::new(),
            task_leaf_only: false,
            task_hide_done: true,
            tree_hide_done: true,
            task_columns: Vec::new(),
            task_project_code_only: false,
            poll_seconds: 30,
            sync_only_during_work: true,
            hotkey: "Win+Shift+H".into(),
            meeting_alerts: true,
            meeting_lead_minutes: 5,
            meeting_alerts_all_projects: true,
            meeting_pauses_others: false,
            reminder_alerts: true,
            notification_alerts: true,
            notification_popup_seconds: 3,
            reminder_before_end_minutes: 15,
            new_work_toast_seconds: 300,
            quiet_when_busy: true,
            prompt_unfilled: true,
            hook_port: 47823,
            eip_leave_sync: true,
            slack_forward: true,
            repos: Vec::new(),
            taskbar_anchor: "BeforeTray".into(),
            taskbar_layout: "TwoLine".into(),
            taskbar_offset: 0.0,
        }
    }
}

impl AppConfig {
    /// 讀 %APPDATA%\HayakuBar\config.json（沒有或讀不懂用預設值）。
    pub fn load() -> Self {
        Self::load_from(&AppPaths::config())
    }

    pub fn save(&self) {
        self.save_to(&AppPaths::config());
    }

    pub fn load_from(path: &Path) -> Self {
        json_file::load(path)
    }

    pub fn save_to(&self, path: &Path) {
        json_file::save(path, self);
    }
}
