//! Hayaku 1.5.0 的範本中心：把子任務範本套到任務上。對應 C# `Mcp/HayakuApi.Templates.cs`。
//! 帳號目前沒有範本（list_subtask_templates 回 {"templates":[]}），範本與套用結果的欄位照工具說明寬鬆讀。

use chrono::{Datelike, NaiveDate};
use serde_json::{Value, json};

use super::HayakuApi;
use crate::json::*;
use crate::models::*;
use crate::transport::McpError;

/// 樹狀節點總數（含 children 底下的）。
fn count_nodes(nodes: &[Value]) -> i32 {
    nodes.iter().map(|n| 1 + count_nodes(items(n, &["children", "items", "nodes"]))).sum()
}

fn md(d: NaiveDate) -> String {
    format!("{}/{}", d.month(), d.day())
}

fn dates(n: &Value) -> String {
    match (day(n, "plannedStart"), day(n, "plannedEnd")) {
        (Some(a), Some(b)) if a != b => format!(" {}～{}", md(a), md(b)),
        (_, Some(b)) => format!(" {}", md(b)),
        (Some(a), None) => format!(" {}", md(a)),
        _ => String::new(),
    }
}

impl HayakuApi {
    /// 看得到的子任務範本（全站與我是成員的專案）；給 project 時只列能套到那個專案的。
    pub async fn list_subtask_templates(&self, project: Option<&str>) -> Result<Vec<SubtaskTemplate>, McpError> {
        let args = match project {
            None => json!({}),
            Some(p) => json!({ "project": p }),
        };
        Ok(Self::parse_subtask_templates(&self.call("list_subtask_templates", args).await?))
    }

    /// 把範本套到任務上（最多 500 個）。dry_run 只回會建哪些、日期，不寫入；skip_existing 略過已經有子任務的任務
    /// （false 也套用，第一層同名的略過）。執行者一次只能套一個任務，建成待核准。
    pub async fn apply_subtask_template(
        &self,
        tasks: &[String],
        template: &str,
        skip_existing: bool,
        dry_run: bool,
    ) -> Result<TemplateApplyResult, McpError> {
        let args = json!({ "tasks": tasks, "template": template, "skipExisting": skip_existing, "dryRun": dry_run });
        Ok(Self::parse_template_apply(&self.call("apply_subtask_template", args).await?))
    }

    pub fn parse_subtask_templates(r: &Value) -> Vec<SubtaskTemplate> {
        let list = match r {
            Value::Array(a) => a.as_slice(),
            _ => items(r, &["templates", "items"]),
        };
        let mut out = Vec::new();
        for t in list {
            let id = id_text(t, "id");
            let Some(name) = first_string(t, &["name", "title"]).or_else(|| id.clone()) else { continue };
            let project = match prop(t, "project") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(p @ Value::Object(_)) => first_string(p, &["key", "code", "name"]),
                Some(_) => None,
                None => first_string(t, &["projectKey"]),
            };
            let nodes = items(t, &["items", "nodes", "tasks", "children"]);
            let count = number(t, &["count", "taskCount", "itemCount", "nodeCount"])
                .or_else(|| if nodes.is_empty() { None } else { Some(count_nodes(nodes)) });
            out.push(SubtaskTemplate {
                id: id.unwrap_or_else(|| name.clone()),
                name,
                project: project.filter(|p| !is_blank(p)),
                count,
                description: first_string(t, &["description", "note"]),
            });
        }
        out
    }

    /// 套用結果 → 每個任務一行：「F2P-5：建立 6 個子任務（需求確認 11/12～11/13、…）」「F2P-6：略過（已經有子任務）」「F2P-7：失敗：…」。
    pub fn parse_template_apply(r: &Value) -> TemplateApplyResult {
        // C# 的 Undefined（沒有內容）在這裡是 null
        let raw = match r {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => raw_text(other),
        };
        if !r.is_object() {
            return TemplateApplyResult { lines: Vec::new(), message: r.as_str().map(str::to_owned), raw };
        }
        let mut lines = Vec::new();
        for e in items(r, &["results", "tasks", "applied", "items", "preview"]) {
            let key = first_key(e, &["task", "key", "parent"])
                .or_else(|| first_string(e, &["task", "id"]))
                .unwrap_or_else(|| "?".to_owned());
            if let Some(error) = first_str(e, &["error"]) {
                lines.push(format!("{key}：失敗：{error}"));
                continue;
            }
            if let Some(s @ (Value::Bool(true) | Value::String(_))) = prop(e, "skipped") {
                let reason = match s.as_str() {
                    Some(r) => Some(r.to_owned()),
                    None => first_string(e, &["reason", "message"]),
                };
                let tail = match reason.filter(|r| !is_blank(r)) {
                    Some(r) => format!("（{r}）"),
                    None => String::new(),
                };
                lines.push(format!("{key}：略過{tail}"));
                continue;
            }
            let nodes = items(e, &["created", "tasks", "children", "items", "nodes"]);
            let count = number(e, &["createdCount", "count"])
                .or_else(|| if nodes.is_empty() { None } else { Some(count_nodes(nodes)) });
            let titles: Vec<String> = nodes
                .iter()
                .take(5)
                .filter_map(|n| first_str(n, &["title", "name"]).map(|t| format!("{t}{}", dates(n))))
                .collect();
            let mut text = match count {
                Some(c) => format!("{key}：{c} 個子任務"),
                None => format!("{key}：{}", first_str(e, &["message", "note", "status"]).unwrap_or("完成")),
            };
            if !titles.is_empty() {
                text += &format!("（{}{}）", titles.join("、"), if nodes.len() > titles.len() { "、…" } else { "" });
            }
            lines.push(text);
        }
        TemplateApplyResult { lines, message: first_string(r, &["message", "note", "summary"]), raw }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 Hayaku150Tests 的範本部分

    #[test]
    fn parse_subtask_templates_空清單() {
        assert!(HayakuApi::parse_subtask_templates(&j(r#"{"templates":[],"timezone":"Asia/Taipei"}"#)).is_empty());
    }

    #[test]
    fn parse_subtask_templates_讀名稱專案與任務數() {
        let list = HayakuApi::parse_subtask_templates(&j(r#"
            {"templates":[
              {"id":12,"name":"新功能開發","project":{"key":"F2P","name":"機密列印"},"items":[
                {"title":"需求確認"},{"title":"開發","children":[{"title":"前端"},{"title":"後端"}]}]},
              {"id":"t-2","name":"上線檢查","project":null,"count":5,"description":"上正式機前"}]}"#));
        assert_eq!(list.len(), 2);
        assert_eq!(
            (list[0].id.as_str(), list[0].name.as_str(), list[0].project.as_deref(), list[0].count.expect("count")),
            ("12", "新功能開發", Some("F2P"), 4)
        );
        assert_eq!(list[0].label(), "新功能開發（4 個任務） · F2P");
        assert_eq!(
            (list[1].id.as_str(), list[1].project.as_deref(), list[1].count, list[1].description.as_deref()),
            ("t-2", None, Some(5), Some("上正式機前"))
        );
        assert_eq!(list[1].label(), "上線檢查（5 個任務） · 全站");
    }

    #[test]
    fn parse_template_apply_每個任務一行() {
        let r = HayakuApi::parse_template_apply(&j(r#"
            {"results":[
              {"task":"F2P-5","created":[{"title":"需求確認","plannedStart":"2026-11-12","plannedEnd":"2026-11-13"},{"title":"開發","children":[{"title":"前端"}]}]},
              {"task":"F2P-6","skipped":true,"reason":"已經有子任務"},
              {"task":"F2P-7","error":"沒有權限"}]}"#));
        assert_eq!(
            r.lines,
            vec![
                "F2P-5：3 個子任務（需求確認 11/12～11/13、開發）",
                "F2P-6：略過（已經有子任務）",
                "F2P-7：失敗：沒有權限"
            ]
        );
    }

    #[test]
    fn parse_template_apply_認不出格式時留原文() {
        let r = HayakuApi::parse_template_apply(&j(r#"{"message":"已送出，等待核准"}"#));
        assert!(r.lines.is_empty());
        assert_eq!(r.message.as_deref(), Some("已送出，等待核准"));
        assert!(r.raw.contains("等待核准"));
    }
}
