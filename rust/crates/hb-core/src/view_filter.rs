//! 對應 C# `Core/ViewFilter.cs`：在本機套用專案視圖的篩選（狀態、狀態類別、執行者、優先級），
//! 不用為了換視圖再打一次 list_tasks。有認不得的條件時回 None，改請伺服器用 view 篩。

use hb_api::{ProjectTask, ProjectView};

use crate::ci_set::eq_ic;

const STATUS_KEYS: [&str; 2] = ["statuses", "status"];
const CATEGORY_KEYS: [&str; 3] = ["statusCategories", "statusCategory", "categories"];
const ASSIGNEE_KEYS: [&str; 2] = ["assignees", "assignee"];
const PRIORITY_KEYS: [&str; 2] = ["priorities", "priority"];

pub struct ViewFilter;

impl ViewFilter {
    pub fn apply<'a>(
        tasks: impl IntoIterator<Item = &'a ProjectTask>,
        view: &ProjectView,
        me: Option<&str>,
    ) -> Option<Vec<ProjectTask>> {
        let known = |k: &str| {
            STATUS_KEYS.iter().chain(&CATEGORY_KEYS).chain(&ASSIGNEE_KEYS).chain(&PRIORITY_KEYS).any(|x| eq_ic(x, k))
        };
        if view.has_other_conditions || view.filter.iter().any(|(k, _)| !known(k)) {
            return None;
        }
        let values = |keys: &[&str]| -> Option<Vec<String>> {
            keys.iter().filter_map(|k| view.filter_values(k)).find(|v| !v.is_empty()).map(<[String]>::to_vec)
        };
        let statuses = values(&STATUS_KEYS);
        let categories = values(&CATEGORY_KEYS);
        let assignees = values(&ASSIGNEE_KEYS).map(|list| {
            list.into_iter()
                .map(|a| match me {
                    Some(me) if a == "me" => me.to_owned(),
                    _ => a,
                })
                .collect::<Vec<_>>()
        });
        let priorities = values(&PRIORITY_KEYS);
        Some(
            tasks
                .into_iter()
                .filter(|t| {
                    statuses.as_ref().is_none_or(|s| s.contains(&t.status))
                        && categories.as_ref().is_none_or(|c| c.iter().any(|x| eq_ic(x, &t.status_category)))
                        && assignees.as_ref().is_none_or(|list| {
                            t.assignees.as_deref().unwrap_or_default().iter().any(|a| list.iter().any(|x| eq_ic(x, a)))
                        })
                        && priorities.as_ref().is_none_or(|p| t.priority.as_ref().is_some_and(|tp| p.contains(tp)))
                })
                .cloned()
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::HayakuApi;

    // 對應 ProjectViewsTests.視圖_認得的條件在本機套用（批次修改結果在 hb-api）

    fn task(
        key: &str,
        status: &str,
        category: &str,
        assignees: Option<&[&str]>,
        priority: Option<&str>,
    ) -> ProjectTask {
        let mut t = ProjectTask::new(key, key, status, category, None, None, None);
        t.assignees = assignees.map(|a| a.iter().map(|s| (*s).to_owned()).collect());
        t.priority = priority.map(str::to_owned);
        t
    }

    fn keys(list: &[ProjectTask]) -> Vec<&str> {
        list.iter().map(|t| t.key.as_str()).collect()
    }

    #[test]
    fn 視圖_認得的條件在本機套用() {
        let views = HayakuApi::parse_views(
            &serde_json::from_str(
                r#"{"views":[
              {"id":"v1","name":"列表","type":"列表","isDefault":true,"filter":{"statuses":["處理中","客戶驗證中"]}},
              {"id":"v2","name":"我的","type":"列表","filter":{"assignees":["me"],"priorities":["高"]}},
              {"id":"v3","name":"日期","type":"列表","filter":{"planned":{"from":"2026-10-01"}}},
              {"id":"v4","name":"成員任務","type":"成員任務","filter":{}}]}"#,
            )
            .unwrap(),
        );
        assert_eq!(4, views.len());
        let tasks = [
            task("A", "處理中", "IN_PROGRESS", Some(&["me.user"]), Some("高")),
            task("B", "待處理", "TODO", Some(&["other"]), None),
            task("C", "客戶驗證中", "IN_PROGRESS", Some(&["other"]), Some("高")),
        ];
        assert_eq!(vec!["A", "C"], keys(&ViewFilter::apply(&tasks, &views[0], Some("me.user")).unwrap()));
        assert_eq!(vec!["A"], keys(&ViewFilter::apply(&tasks, &views[1], Some("me.user")).unwrap()));
        assert!(ViewFilter::apply(&tasks, &views[2], Some("me.user")).is_none()); // 認不得的條件：交給伺服器
        assert_eq!(3, ViewFilter::apply(&tasks, &views[3], Some("me.user")).unwrap().len());
    }
}
