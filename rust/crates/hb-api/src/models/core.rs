//! 對應 C# `Mcp/HayakuApi.cs` 的 record。

use chrono::{NaiveDate, NaiveTime};

use super::DateTimeOffset;

#[derive(Debug, Clone, PartialEq)]
pub struct TimeEntry {
    pub id: String,
    pub task: String,
    pub title: String,
    pub started_at: Option<DateTimeOffset>,
    pub ended_at: Option<DateTimeOffset>,
    pub duration_minutes: Option<i32>,
    pub running: bool,
    pub stop_reason: Option<String>,
    pub note: Option<String>,
    pub user: Option<String>,
    pub source: Option<String>,
}

impl TimeEntry {
    /// C# 建構子的必填欄位；Note、User、Source 預設 null。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        task: impl Into<String>,
        title: impl Into<String>,
        started_at: Option<DateTimeOffset>,
        ended_at: Option<DateTimeOffset>,
        duration_minutes: Option<i32>,
        running: bool,
        stop_reason: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            task: task.into(),
            title: title.into(),
            started_at,
            ended_at,
            duration_minutes,
            running,
            stop_reason,
            note: None,
            user: None,
            source: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkSegment {
    pub start: NaiveTime,
    pub end: NaiveTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaySchedule {
    pub date: NaiveDate,
    pub working: Vec<WorkSegment>,
    pub expected_minutes: i32,
    pub on_leave: bool,
    pub holiday: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    pub key: String,
    pub title: String,
    pub project: String,
    pub status: String,
}

/// 目前的使用者；hayaku_version：Hayaku 伺服器的版本（whoami 的 appVersion，舊版沒有）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub username: String,
    pub display_name: String,
    pub hayaku_version: Option<String>,
}
