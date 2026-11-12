//! 統計（跟網頁統計頁同一套數字）。對應 C# `Mcp/HayakuApi.Stats.cs`。

use chrono::NaiveDate;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

use super::HayakuApi;
use crate::json::*;
use crate::models::*;
use crate::transport::McpError;

/// { minutes } 物件的分鐘數（只認 32 位元整數）。
fn minutes(e: &Value, name: &str) -> i32 {
    match prop(e, name) {
        Some(v @ Value::Object(_)) => prop(v, "minutes").and_then(as_i32).unwrap_or(0),
        _ => 0,
    }
}

fn int(e: &Value, name: &str) -> i32 {
    prop(e, name).and_then(as_i32).unwrap_or(0)
}

/// taskCounts 物件裡是整數的欄位（照原本順序）。
fn task_counts(e: &Value) -> Option<Vec<(String, i32)>> {
    match prop(e, "taskCounts") {
        Some(Value::Object(o)) => Some(o.iter().filter_map(|(k, v)| Some((k.clone(), as_i32(v)?))).collect()),
        _ => None,
    }
}

fn get(counts: &[(String, i32)], key: &str) -> i32 {
    counts.iter().find(|(k, _)| k == key).map_or(0, |(_, v)| *v)
}

impl HayakuApi {
    /// perspective：project 或 assignee；preset（thisWeek、lastWeek、thisMonth…）或 from/to 擇一。
    pub async fn stats(
        &self,
        perspective: &str,
        preset: Option<&str>,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
        granularity: &str,
    ) -> Result<StatsReport, McpError> {
        let mut args = Map::new();
        args.insert("perspective".into(), json!(perspective));
        args.insert("granularity".into(), json!(granularity));
        if let (Some(f), Some(t)) = (from, to) {
            args.insert("from".into(), json!(iso(f)));
            args.insert("to".into(), json!(iso(t)));
        } else {
            args.insert("preset".into(), json!(preset.unwrap_or("thisMonth")));
        }
        Ok(Self::parse_stats(&self.call("get_stats", Value::Object(args)).await?))
    }

    pub fn parse_stats(r: &Value) -> StatsReport {
        let perspective = string(r, "perspective").unwrap_or_else(|| "project".to_owned());
        let by_project = perspective == "project";
        let range = obj(r, "range");
        let buckets: Vec<String> = match prop(range, "buckets") {
            Some(Value::Array(b)) => b.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
            _ => Vec::new(),
        };
        let mut project_names: HashMap<String, String> = HashMap::new();
        for p in items(r, &["projects"]) {
            if let Some(key) = string(p, "key") {
                project_names.entry(key.clone()).or_insert_with(|| string(p, "name").unwrap_or(key));
            }
        }
        let mut user_names: HashMap<String, String> = HashMap::new();
        for u in items(r, &["users"]) {
            if let Some(key) = string(u, "username") {
                user_names.entry(key.clone()).or_insert_with(|| string(u, "displayName").unwrap_or(key));
            }
        }
        let name_of = |key: &str| -> String {
            let names = if by_project { &project_names } else { &user_names };
            names.get(key).cloned().unwrap_or_else(|| key.to_owned())
        };

        let row_key = if by_project { "project" } else { "user" };
        let mut rows: Vec<StatsRow> = items(r, &["rows"])
            .iter()
            .map(|row| {
                let key = string(row, row_key).unwrap_or_default();
                let counts = task_counts(row).unwrap_or_default();
                let total = if by_project { counts.iter().map(|(_, v)| v).sum() } else { int(row, "assignedCount") };
                let done = if by_project { get(&counts, "DONE") } else { int(row, "doneCount") };
                StatsRow {
                    name: name_of(&key),
                    key,
                    actual_minutes: minutes(row, "actual"),
                    estimated_minutes: minutes(row, "estimated"),
                    total_logged_minutes: minutes(row, "totalLogged"),
                    expected_minutes: minutes(row, "expected"),
                    workdays: int(row, "workdays"),
                    active_days: int(row, "activeDays"),
                    assigned: total,
                    done,
                    overdue: int(row, "overdueCount"),
                    completion_rate: prop(row, "completionRate").and_then(as_f64),
                }
            })
            .collect();
        // 工時多的在前；同工時照名稱（C# 用目前文化的排序，這裡用字元順序）
        rows.sort_by(|a, b| b.actual_minutes.cmp(&a.actual_minutes).then_with(|| a.name.cmp(&b.name)));

        let mut cells: Vec<StatsCell> = Vec::new();
        for c in items(r, &["cells"]) {
            let key = string(c, row_key).unwrap_or_default();
            let bucket = string(c, "bucket").unwrap_or_default();
            let m = minutes(c, "duration");
            match cells.iter_mut().find(|x| x.key == key && x.bucket == bucket) {
                Some(cell) => cell.minutes += m,
                None => cells.push(StatsCell { key, bucket, minutes: m }),
            }
        }

        let summary = obj(r, "summary");
        let has_summary = summary.is_object();
        let summary_counts = if has_summary { task_counts(summary) } else { None };
        StatsReport {
            perspective: perspective.clone(),
            from: day(range, "from"),
            to: day(range, "to"),
            granularity: if range.is_object() {
                string(range, "granularity").unwrap_or_else(|| "day".to_owned())
            } else {
                "day".to_owned()
            },
            buckets,
            actual_minutes: if has_summary {
                minutes(summary, "actual")
            } else {
                rows.iter().map(|x| x.actual_minutes).sum()
            },
            estimated_minutes: if has_summary {
                minutes(summary, "estimated")
            } else {
                rows.iter().map(|x| x.estimated_minutes).sum()
            },
            expected_minutes: rows.iter().map(|x| x.expected_minutes).sum(),
            done: summary_counts.as_ref().map_or_else(|| rows.iter().map(|x| x.done).sum(), |c| get(c, "DONE")),
            task_total: summary_counts
                .as_ref()
                .map_or_else(|| rows.iter().map(|x| x.assigned).sum(), |c| c.iter().map(|(_, v)| v).sum()),
            overdue: if has_summary { int(summary, "overdueCount") } else { rows.iter().map(|x| x.overdue).sum() },
            rows,
            cells,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 StatsTests
    #[test]
    fn 依專案的統計() {
        let s = HayakuApi::parse_stats(&j(r#"
            {"perspective":"project","range":{"from":"2026-10-01","to":"2026-10-31","granularity":"week","buckets":["2026-09-28","2026-10-05"]},
             "projects":[{"key":"F2P","name":"臉部辨識"},{"key":"REV","name":"內部"}],
             "rows":[{"project":"REV","actual":{"minutes":76},"taskCounts":{"TODO":2}},
                     {"project":"F2P","actual":{"minutes":11300},"estimated":{"minutes":1710},"taskCounts":{"DONE":43,"IN_PROGRESS":16},"overdueCount":11}],
             "cells":[{"project":"F2P","bucket":"2026-09-28","duration":{"minutes":60}},{"project":"F2P","bucket":"2026-09-28","duration":{"minutes":30}}],
             "summary":{"actual":{"minutes":11376},"estimated":{"minutes":1710},"taskCounts":{"DONE":43,"IN_PROGRESS":16,"TODO":2},"overdueCount":11}}"#));
        assert!(s.by_project());
        assert_eq!(s.rows.iter().map(|x| x.key.as_str()).collect::<Vec<_>>(), vec!["F2P", "REV"]); // 工時多的在前
        assert_eq!(
            (s.rows[0].name.as_str(), s.rows[0].actual_minutes, s.rows[0].done, s.rows[0].assigned),
            ("臉部辨識", 11300, 43, 59)
        );
        assert_eq!(s.cells.len(), 1);
        assert_eq!(s.cells[0].minutes, 90);
        assert_eq!((s.actual_minutes, s.done, s.task_total, s.overdue), (11376, 43, 61, 11));
        assert_eq!(s.buckets, vec!["2026-09-28", "2026-10-05"]);
    }
}
