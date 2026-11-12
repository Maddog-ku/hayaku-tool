//! 對應 C# `Core/TaskTree.cs`：把 list_tasks 的平面清單組成專案樹，並依「顯示已完成」與關鍵字篩選。

use chrono::NaiveDate;
use hb_api::ProjectTask;
use std::collections::HashMap;

use crate::ci_set::fold;

/// 專案樹的一個節點。
#[derive(Debug, Clone, PartialEq)]
pub struct TaskTreeNode {
    pub task: ProjectTask,
    pub children: Vec<TaskTreeNode>,
}

impl TaskTreeNode {
    pub fn new(task: ProjectTask) -> Self {
        Self { task, children: Vec::new() }
    }
}

pub struct TaskTree;

impl TaskTree {
    /// 組樹：Parent 找不到（例如沒權限看）的當成最上層；順序照原清單。
    /// 隱藏已完成／不處理的任務，但底下還有要顯示的子任務時保留，樹才不會斷掉。
    /// filter：編號或名稱包含；符合的任務連同它的上層都會顯示。
    ///
    /// C# 的 `ToDictionary` 遇到重複的編號會丟例外；這裡同一個編號只取第一筆。
    pub fn build(
        tasks: impl IntoIterator<Item = ProjectTask>,
        include_done: bool,
        filter: Option<&str>,
    ) -> Vec<TaskTreeNode> {
        let mut all: Vec<ProjectTask> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for t in tasks {
            let key = fold(&t.key);
            if index.contains_key(&key) {
                log::info!("專案樹有重複的任務編號 {}，只取第一筆", t.key);
                continue;
            }
            index.insert(key, all.len());
            all.push(t);
        }
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); all.len()];
        let mut roots: Vec<usize> = Vec::new();
        for (i, t) in all.iter().enumerate() {
            match t.parent.as_deref().and_then(|p| index.get(&fold(p))) {
                Some(&parent) if parent != i => children[parent].push(i),
                _ => roots.push(i),
            }
        }
        let q = filter.map(str::trim).unwrap_or("");
        prune(&all, &children, &roots, include_done, &fold(q))
    }

    /// 已逾期：預計結束已過、還沒完成或不處理（分類不算，跟網頁一樣）。
    pub fn is_overdue(t: &ProjectTask, today: NaiveDate) -> bool {
        !matches!(t.status_category.as_str(), "DONE" | "CANCELLED")
            && !t.is_category
            && t.planned_end.is_some_and(|end| end < today)
    }
}

/// `text.Contains(q, OrdinalIgnoreCase)`；q 已經 fold 過。
fn contains_ic(text: &str, q_folded: &str) -> bool {
    fold(text).contains(q_folded)
}

fn prune(
    all: &[ProjectTask],
    children: &[Vec<usize>],
    nodes: &[usize],
    include_done: bool,
    q: &str,
) -> Vec<TaskTreeNode> {
    let mut kept = Vec::new();
    for &n in nodes {
        let kids = prune(all, children, &children[n], include_done, q);
        let task = &all[n];
        let done = matches!(task.status_category.as_str(), "DONE" | "CANCELLED");
        let matches = q.is_empty() || contains_ic(&task.key, q) || contains_ic(&task.title, q);
        if kids.is_empty() && (!matches || done && !include_done) {
            continue;
        }
        kept.push(TaskTreeNode { task: task.clone(), children: kids });
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    // 對應 TaskTreeTests（ParseProjectTasks 在 hb-api）與 Hayaku150Tests.IsOverdue

    fn t(key: &str, parent: Option<&str>, category: &str, title: Option<&str>) -> ProjectTask {
        ProjectTask::new(key, title.unwrap_or(key), "處理中", category, parent.map(str::to_owned), None, None)
    }

    fn flat(nodes: &[TaskTreeNode], depth: usize) -> Vec<String> {
        nodes
            .iter()
            .flat_map(|n| {
                std::iter::once(format!("{}{}", "-".repeat(depth), n.task.key)).chain(flat(&n.children, depth + 1))
            })
            .collect()
    }

    #[test]
    fn build_依上層組樹並保留順序_找不到上層的放最上層() {
        let tree = TaskTree::build(
            [
                t("A-1", None, "IN_PROGRESS", None),
                t("A-2", Some("A-1"), "IN_PROGRESS", None),
                t("A-3", Some("A-2"), "IN_PROGRESS", None),
                t("A-4", Some("A-1"), "IN_PROGRESS", None),
                t("A-5", Some("A-99"), "IN_PROGRESS", None),
            ],
            true,
            None,
        );
        assert_eq!(vec!["A-1", "-A-2", "--A-3", "-A-4", "A-5"], flat(&tree, 0));
    }

    #[test]
    fn build_隱藏已完成但保留還有子任務的上層() {
        let tree = TaskTree::build(
            [
                t("A-1", None, "DONE", None),
                t("A-2", Some("A-1"), "IN_PROGRESS", None),
                t("A-3", None, "CANCELLED", None),
                t("A-4", Some("A-1"), "DONE", None),
            ],
            false,
            None,
        );
        assert_eq!(vec!["A-1", "-A-2"], flat(&tree, 0));
    }

    #[test]
    fn build_篩選時顯示符合的任務與它的上層() {
        let tree = TaskTree::build(
            [
                t("A-1", None, "IN_PROGRESS", Some("文管")),
                t("A-2", Some("A-1"), "IN_PROGRESS", Some("列印")),
                t("A-3", Some("A-1"), "IN_PROGRESS", Some("掃描")),
                t("A-4", None, "IN_PROGRESS", Some("其他")),
            ],
            true,
            Some("列印"),
        );
        assert_eq!(vec!["A-1", "-A-2"], flat(&tree, 0));
    }

    #[test]
    fn is_overdue_預計結束已過且沒完成() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        let task = |category: &str, end: Option<NaiveDate>, is_category: bool| {
            let mut p = ProjectTask::new("F2P-1", "x", "處理中", category, None, None, end);
            p.is_category = is_category;
            p
        };
        let days = |n: i64| today + chrono::TimeDelta::days(n);
        assert!(TaskTree::is_overdue(&task("IN_PROGRESS", Some(days(-1)), false), today));
        assert!(!TaskTree::is_overdue(&task("IN_PROGRESS", Some(today), false), today));
        assert!(!TaskTree::is_overdue(&task("DONE", Some(days(-1)), false), today));
        assert!(!TaskTree::is_overdue(&task("TODO", None, false), today));
        assert!(!TaskTree::is_overdue(&task("TODO", Some(days(-3)), true), today));
    }
}
