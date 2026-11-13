//! 專案視圖、批次修改。對應 C# `Mcp/HayakuApi.Views.cs`。

use serde_json::{Map, Value, json};

use super::HayakuApi;
use crate::json::*;
use crate::models::*;
use crate::transport::McpError;

impl HayakuApi {
    pub async fn list_views(&self, project: &str) -> Result<Vec<ProjectView>, McpError> {
        Ok(Self::parse_views(&self.call("list_views", json!({ "project": project })).await?))
    }

    pub fn parse_views(r: &Value) -> Vec<ProjectView> {
        let mut list = Vec::new();
        for v in items(r, &["views"]) {
            let Some(id) = string(v, "id") else { continue };
            let mut filter: Vec<(String, Vec<String>)> = Vec::new();
            let mut other = false;
            if let Some(Value::Object(f)) = prop(v, "filter") {
                for (name, value) in f {
                    match value {
                        Value::Array(a) if a.iter().all(Value::is_string) => {
                            if !a.is_empty() {
                                let values = a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect();
                                ProjectView::set_filter(&mut filter, name, values);
                            }
                        }
                        Value::String(s) if !s.is_empty() => {
                            ProjectView::set_filter(&mut filter, name, vec![s.clone()])
                        }
                        Value::Null | Value::Bool(false) | Value::String(_) => {}
                        _ => other = true,
                    }
                }
            }
            list.push(ProjectView {
                id,
                name: string(v, "name").unwrap_or_else(|| "視圖".to_owned()),
                r#type: string(v, "type").unwrap_or_default(),
                is_default: bool_(v, "isDefault"),
                filter,
                has_other_conditions: other,
            });
        }
        list
    }

    /// 一次改多個任務（最多 100 筆，逐筆成功或失敗）；每筆是 { task, 欄位… }。
    pub async fn update_tasks(&self, tasks: &[Map<String, Value>]) -> Result<Vec<BatchResult>, McpError> {
        let sent: Vec<String> =
            tasks.iter().map(|t| t.get("task").map(element_to_string).unwrap_or_default()).collect();
        let r = self.call("update_tasks", json!({ "tasks": tasks })).await?;
        Ok(Self::parse_batch_results(&r, &sent))
    }

    /// 每筆結果在 results 或 items；沒有逐筆結果時當作全部成功。
    pub fn parse_batch_results(r: &Value, sent: &[String]) -> Vec<BatchResult> {
        let items_: Vec<&Value> = items(r, &["results", "items", "tasks"]).iter().filter(|i| i.is_object()).collect();
        if items_.is_empty() {
            return sent.iter().map(|t| BatchResult { task: t.clone(), error: None, message: None }).collect();
        }
        items_
            .iter()
            .enumerate()
            .map(|(n, i)| {
                let task = string(i, "task")
                    .or_else(|| string(i, "key"))
                    .unwrap_or_else(|| sent.get(n).cloned().unwrap_or_default());
                let error = string(i, "error").or_else(|| {
                    if is_false(i, "ok") {
                        Some(string(i, "message").unwrap_or_else(|| "失敗".to_owned()))
                    } else {
                        None
                    }
                });
                let message = if error.is_none() { string(i, "message").or_else(|| string(i, "note")) } else { None };
                BatchResult { task, error, message }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 ProjectViewsTests 的 API 解析部分（ViewFilter.Apply 屬於 hb-core）
    #[test]
    fn 視圖_認得的條件與其他條件() {
        let views = HayakuApi::parse_views(&j(r#"
            {"views":[
              {"id":"v1","name":"列表","type":"列表","isDefault":true,"filter":{"statuses":["處理中","客戶驗證中"]}},
              {"id":"v2","name":"我的","type":"列表","filter":{"assignees":["me"],"priorities":["高"]}},
              {"id":"v3","name":"日期","type":"列表","filter":{"planned":{"from":"2026-10-01"}}},
              {"id":"v4","name":"成員任務","type":"成員任務","filter":{}}]}"#));
        assert_eq!(views.len(), 4);
        assert!(views[0].is_default);
        assert_eq!(views[0].filter_values("STATUSES"), Some(&["處理中".to_owned(), "客戶驗證中".to_owned()][..]));
        assert_eq!(views[1].filter_values("assignees"), Some(&["me".to_owned()][..]));
        assert!(!views[1].has_other_conditions);
        assert!(views[2].has_other_conditions); // 認不得的條件：交給伺服器
        assert!(views[3].filter.is_empty());
    }

    #[test]
    fn 批次修改結果() {
        let sent = vec!["F2P-1".to_owned(), "F2P-2".to_owned()];
        let results = HayakuApi::parse_batch_results(
            &j(
                r#"{"results":[{"task":"F2P-1","ok":true,"message":"已送出變更申請"},{"task":"F2P-2","ok":false,"error":"沒有權限"}]}"#,
            ),
            &sent,
        );
        assert_eq!(
            (results[0].task.as_str(), results[0].error.as_deref(), results[0].message.as_deref()),
            ("F2P-1", None, Some("已送出變更申請"))
        );
        assert_eq!((results[1].task.as_str(), results[1].error.as_deref()), ("F2P-2", Some("沒有權限")));
        assert!(HayakuApi::parse_batch_results(&j("{}"), &sent).iter().all(|r| r.error.is_none()));
    }
}
