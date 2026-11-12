//! 對應 C# `Mcp/HayakuApi.Stats.cs` 的 record。

use chrono::NaiveDate;

/// 統計表的一列（get_stats 的 rows）：依專案時 key 是專案代碼；依人時是帳號。
/// 依專案才有預計、累計、任務數與完成率；依人才有應出勤、工作天、有計時天數與指派、完成數。兩邊都有實際工時與逾期。
#[derive(Debug, Clone, PartialEq)]
pub struct StatsRow {
    pub key: String,
    pub name: String,
    pub actual_minutes: i32,
    pub estimated_minutes: i32,
    pub total_logged_minutes: i32,
    pub expected_minutes: i32,
    pub workdays: i32,
    pub active_days: i32,
    pub assigned: i32,
    pub done: i32,
    pub overdue: i32,
    pub completion_rate: Option<f64>,
}

/// 某列在某個區間（日、週、月）的工時；依人時是那個人在各專案照實相加（同時計時會重複算）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsCell {
    pub key: String,
    pub bucket: String,
    pub minutes: i32,
}

/// 一次統計（get_stats）：perspective 是 project（依專案）或 assignee（依執行者）；buckets 是區間的名稱（日期或週、月的起日）。
/// actual_minutes：依專案時用伺服器的合計；依人時把每人的實際（已經是時間聯集）加起來。
#[derive(Debug, Clone, PartialEq)]
pub struct StatsReport {
    pub perspective: String,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub granularity: String,
    pub buckets: Vec<String>,
    pub rows: Vec<StatsRow>,
    pub cells: Vec<StatsCell>,
    pub actual_minutes: i32,
    pub estimated_minutes: i32,
    pub expected_minutes: i32,
    pub done: i32,
    pub task_total: i32,
    pub overdue: i32,
}

impl StatsReport {
    pub fn by_project(&self) -> bool {
        self.perspective == "project"
    }
}
