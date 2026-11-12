//! 對應 C# `Mcp/HayakuApi.Testing.cs` 的型別（任務類型、測試子狀態、測試結果、測試總覽、讀檔、問題回報）。

use chrono::NaiveDate;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use super::DateTimeOffset;

/// 任務類型：一般、測試（QA 開的測試任務）、測試結果（測試任務底下的一個問題）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskKind {
    #[default]
    Normal,
    Test,
    TestResult,
}

/// 專案自訂的測試子狀態名稱 → 所屬階段（例 待更新測試機 → 修復中、待更新正式機 → 待復驗）。
/// 讀到專案設定或測試總覽時記下；只有狀態名稱、沒有 testStage 的回應（我的任務、任務列表）靠它分對組。
static KNOWN_STAGES: LazyLock<RwLock<HashMap<String, String>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// 任務類型、測試結果狀態的文字與分組（跟網頁同一套名稱）。
pub struct TaskKinds;

impl TaskKinds {
    pub const SEVERITIES: [&'static str; 4] = ["致命", "嚴重", "一般", "輕微"];
    pub const CATEGORIES: [&'static str; 3] = ["待判斷", "Bug", "需求變更"];

    /// 測試結果的狀態（網頁的順序）；「不處理」是結案的一種。
    pub const RESULT_STATUSES: [&'static str; 7] =
        ["未開始", "測試中", "待修復", "修復中", "待復驗", "已完成", "不處理"];

    /// 類型的值可能是代碼（TEST、TEST_RESULT）或中文（測試、測試結果）；認不得的當一般任務。
    pub fn parse(value: Option<&str>) -> TaskKind {
        let Some(value) = value.filter(|v| !crate::json::is_blank(v)) else {
            return TaskKind::Normal;
        };
        let v = value.trim().to_uppercase();
        if v.contains("RESULT") || v.contains("結果") || v == "BUG" || v == "ISSUE" {
            return TaskKind::TestResult;
        }
        if v.contains("TEST") || v.contains("測試") || v == "QA" {
            return TaskKind::Test;
        }
        TaskKind::Normal
    }

    /// update_task、create_task 的 type 參數。
    pub fn label(kind: TaskKind) -> &'static str {
        match kind {
            TaskKind::Test => "測試",
            TaskKind::TestResult => "測試結果",
            TaskKind::Normal => "一般",
        }
    }

    /// 標題前的類型圖示；一般任務沒有。
    pub fn icon(kind: TaskKind) -> &'static str {
        match kind {
            TaskKind::Test => "🧪",
            TaskKind::TestResult => "🐞",
            TaskKind::Normal => "",
        }
    }

    pub fn learn_stages(sub_statuses: &[TestSubStatus]) {
        let Ok(mut known) = KNOWN_STAGES.write() else { return };
        for s in sub_statuses {
            if s.name != s.stage {
                known.insert(s.name.clone(), s.stage.clone());
            }
        }
    }

    /// 同一段裡 status 的下一個子狀態：內建狀態（修復中、待復驗、已完成）固定是每段第一列，後面接專案自訂的子狀態
    /// （Hayaku 1.4.2 起，不用再自己建同名的狀態；回應有沒有列出內建那一列、或舊專案自己建了同名的都一樣）。
    /// status 不在這一段或已經是最後一個是 None。
    pub fn next_in_stage<'a>(
        sub_statuses: &'a [TestSubStatus],
        stage: &str,
        status: &str,
    ) -> Option<&'a TestSubStatus> {
        let custom: Vec<&TestSubStatus> = sub_statuses.iter().filter(|s| s.stage == stage && s.name != stage).collect();
        let i: isize =
            if status == stage { -1 } else { custom.iter().position(|s| s.name == status).map_or(-1, |p| p as isize) };
        if i < 0 && status != stage {
            return None;
        }
        custom.get((i + 1) as usize).copied()
    }

    /// 狀態所屬的測試階段（子狀態換成階段名稱）；stage 有給就用它。
    pub fn stage_of(status: &str, stage: Option<&str>) -> String {
        if let Some(s) = stage.filter(|s| !s.is_empty()) {
            return s.to_owned();
        }
        if let Ok(known) = KNOWN_STAGES.read()
            && let Some(k) = known.get(status)
        {
            return k.clone();
        }
        status.to_owned()
    }

    /// 測試結果狀態 → 分組：pass（已完成）、closed（不處理）、fixing（待修復、修復中）、retest（待復驗）、testing（未開始、測試中）。
    /// stage：回應的 testStage（子狀態所屬的階段，例 待上正式機 屬於待復驗）；沒有時查記下的子狀態，再認不得用狀態類別判斷。
    pub fn bucket(status: &str, category: Option<&str>, stage: Option<&str>) -> &'static str {
        match Self::stage_of(status, stage).as_str() {
            "已完成" | "通過" => "pass",
            "不處理" => "closed",
            "待修復" | "修復中" => "fixing",
            "待復驗" => "retest",
            "未開始" | "測試中" => "testing",
            _ => match category {
                Some("DONE") => "pass",
                Some("CANCELLED") => "closed",
                Some("IN_PROGRESS") => "fixing",
                _ => "testing",
            },
        }
    }

    /// 嚴重度的顏色（brush 名稱）：致命、嚴重紅色，一般預設，輕微淡色。
    pub fn severity_brush(severity: Option<&str>) -> &'static str {
        match severity {
            Some("致命" | "嚴重") => "ErrorBrush",
            Some("輕微") => "MutedBrush",
            _ => "TextBrush",
        }
    }
}

/// 專案的測試子狀態（get_project 的 testSubStatuses）：stage 是所屬階段（修復中、待復驗、已完成），同一階段照順序；
/// batch_label 有值的可以批次推到下一步（畫面上是〔已{BatchLabel}〕，例 已更新到測試機）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSubStatus {
    pub stage: String,
    pub name: String,
    pub batch_label: Option<String>,
    pub color: Option<String>,
    pub count: Option<i32>,
    pub oldest_days: Option<i32>,
}

impl TestSubStatus {
    pub fn new(
        stage: impl Into<String>,
        name: impl Into<String>,
        batch_label: Option<&str>,
        color: Option<&str>,
    ) -> Self {
        Self {
            stage: stage.into(),
            name: name.into(),
            batch_label: batch_label.map(str::to_owned),
            color: color.map(str::to_owned),
            count: None,
            oldest_days: None,
        }
    }

    /// 批次按鈕的文字，例「已更新到測試機」。
    pub fn batch_text(&self) -> Option<String> {
        self.batch_label
            .as_deref()
            .filter(|b| !b.is_empty())
            .map(|b| if b.starts_with('已') { b.to_owned() } else { format!("已{b}") })
    }
}

/// 測試相關欄位：類型、分類、嚴重度、修復輪數（退回一次加一輪）、回報的人、所屬階段（testStage，子狀態才跟狀態不同）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestInfo {
    pub kind: TaskKind,
    pub category: Option<String>,
    pub severity: Option<String>,
    pub round: i32,
    pub reporter: Option<String>,
    pub stage: Option<String>,
}

impl TestInfo {
    pub fn new(
        kind: TaskKind,
        category: Option<&str>,
        severity: Option<&str>,
        round: i32,
        reporter: Option<&str>,
    ) -> Self {
        Self {
            kind,
            category: category.map(str::to_owned),
            severity: severity.map(str::to_owned),
            round,
            reporter: reporter.map(str::to_owned),
            stage: None,
        }
    }

    /// 第 3 輪起要醒目標示（網頁同一套）。
    pub fn hot(&self) -> bool {
        self.round >= 3
    }
}

/// 測試結果的一列（list_test_results）。group：fix 視圖是 fix；test 視圖是 retest（待復驗）、toTest（要測的測試結果）、
/// tests（要測的測試任務）；overview 的逾時清單是 stale。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResultRow {
    pub key: String,
    pub title: String,
    pub project: String,
    pub status: String,
    pub kind: TaskKind,
    pub category: Option<String>,
    pub severity: Option<String>,
    pub round: i32,
    pub assignees: Vec<String>,
    pub reporter: Option<String>,
    pub planned_end: Option<NaiveDate>,
    pub test_task: Option<String>,
    pub test_task_title: Option<String>,
    pub stale: bool,
    pub group: String,
    pub stage: Option<String>,
}

impl TestResultRow {
    /// 所屬階段（子狀態換成階段名稱）。
    pub fn stage_name(&self) -> String {
        TaskKinds::stage_of(&self.status, self.stage.as_deref())
    }
}

/// 測試總覽的一個測試任務：底下的測試結果各狀態數量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestTaskProgress {
    pub key: String,
    pub title: String,
    pub project: String,
    pub total: i32,
    pub passed: i32,
    pub fixing: i32,
    pub testing: i32,
    pub retest: i32,
}

/// 測試總覽（list_test_results view=overview）。
#[derive(Debug, Clone, PartialEq)]
pub struct TestOverview {
    pub open: i32,
    pub critical: i32,
    pub major: i32,
    pub retest: i32,
    pub oldest_retest_days: Option<i32>,
    pub created: i32,
    pub done: i32,
    pub wont_fix: i32,
    pub avg_round: Option<f64>,
    pub round3_plus: i32,
    pub stale: i32,
    /// (Severity, Count)
    pub by_severity: Vec<(String, i32)>,
    pub tests: Vec<TestTaskProgress>,
    pub stale_items: Vec<TestResultRow>,
    pub pending_release: i32,
    pub sub_statuses: Option<Vec<TestSubStatus>>,
}

/// read_file 的結果：圖片（image 有值）或文字（text，太長分段：next_offset 不是 None 就還有）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContent {
    pub id: String,
    pub name: Option<String>,
    pub mime_type: Option<String>,
    pub image: Option<Vec<u8>>,
    pub text: Option<String>,
    pub next_offset: Option<i32>,
    pub note: Option<String>,
}

/// 問題回報（get_feedback）：全文、處理紀錄與截圖 id（用 read_file 看）。
#[derive(Debug, Clone, PartialEq)]
pub struct FeedbackDetail {
    pub number: i32,
    pub title: String,
    pub status: String,
    pub reporter: Option<String>,
    pub created_at: Option<DateTimeOffset>,
    pub body: String,
    pub resolution: Option<String>,
    /// (Label, Value)
    pub environment: Vec<(String, String)>,
    pub history: Vec<String>,
    pub created_tasks: Vec<String>,
    pub screenshot_ids: Vec<String>,
}
