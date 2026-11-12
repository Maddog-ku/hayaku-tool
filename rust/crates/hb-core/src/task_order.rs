//! 對應 C# `Core/TaskOrder.cs`：工作台「我的任務」清單的排序、狀態篩選與狀態統計。

use std::collections::HashMap;

use hb_api::{KeyFinder, TaskSummary};
use indexmap::IndexSet;

use crate::ci_set::{CiSet, cmp_ic};

/// 狀態篩選：可以混著選狀態類別（進行中、完成…）與各專案自己的狀態（處理中、待客戶回應…）；
/// 符合任一個就顯示，都沒選＝全部。
#[derive(Debug, Clone, Default)]
pub struct StatusFilter {
    /// 狀態類別（不分大小寫）。
    pub categories: CiSet,
    pub statuses: IndexSet<String>,
}

impl StatusFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.categories.is_empty() && self.statuses.is_empty()
    }

    pub fn count(&self) -> usize {
        self.categories.len() + self.statuses.len()
    }

    pub fn matches(&self, t: &TaskSummary) -> bool {
        self.is_empty() || self.categories.contains(&t.status_category) || self.statuses.contains(&t.status)
    }

    /// 選的項目在新清單裡都不見了就清掉（換分頁、任務改狀態）。
    pub fn keep_only<'a>(
        &mut self,
        categories: impl IntoIterator<Item = &'a str>,
        statuses: impl IntoIterator<Item = &'a str>,
    ) {
        self.categories.intersect_with(categories);
        let keep: IndexSet<&str> = statuses.into_iter().collect();
        self.statuses.retain(|s| keep.contains(s.as_str()));
    }

    pub fn clear(&mut self) {
        self.categories.clear();
        self.statuses.clear();
    }
}

pub struct TaskOrder;

impl TaskOrder {
    pub const SORTS: [(&'static str, &'static str); 7] = [
        ("default", "預設排序"),
        ("due", "預計結束：近→遠"),
        ("dueDesc", "預計結束：遠→近"),
        ("priority", "優先級：高→低"),
        ("status", "依狀態"),
        ("key", "任務編號"),
        ("timing", "計時中優先"),
    ];

    /// 狀態類別與顯示名稱，依工作流程的順序。
    pub const CATEGORY_LABELS: [(&'static str, &'static str); 4] =
        [("TODO", "待處理"), ("IN_PROGRESS", "進行中"), ("DONE", "完成"), ("CANCELLED", "已取消")];

    const PRIORITY_RANK: [&'static str; 4] = ["緊急", "高", "中", "低"];

    /// 排序；同順位維持 Hayaku 給的原本順序。running：正在計時的任務（「計時中優先」用）。
    /// status_rank：狀態在專案設定裡的順序（「依狀態」用：同一類的不同狀態照專案的順序，不再照名稱穿插）。
    pub fn sort(
        tasks: impl IntoIterator<Item = TaskSummary>,
        sort: &str,
        running: &CiSet,
        status_rank: Option<&dyn Fn(&TaskSummary) -> i32>,
    ) -> Vec<TaskSummary> {
        let mut list: Vec<TaskSummary> = tasks.into_iter().collect();
        // 沒有日期的放最後
        let due = |a: &TaskSummary, b: &TaskSummary| {
            a.planned_end.is_none().cmp(&b.planned_end.is_none()).then_with(|| a.planned_end.cmp(&b.planned_end))
        };
        match sort {
            "due" => list.sort_by(due),
            "dueDesc" => list.sort_by(|a, b| {
                a.planned_end.is_none().cmp(&b.planned_end.is_none()).then_with(|| b.planned_end.cmp(&a.planned_end))
            }),
            "priority" => list.sort_by(|a, b| {
                Self::priority_of(&a.priority).cmp(&Self::priority_of(&b.priority)).then_with(|| due(a, b))
            }),
            "status" => list.sort_by(|a, b| {
                Self::category_rank(&a.status_category)
                    .cmp(&Self::category_rank(&b.status_category))
                    .then_with(|| {
                        let rank = |t: &TaskSummary| status_rank.map_or(0, |f| f(t));
                        rank(a).cmp(&rank(b))
                    })
                    .then_with(|| a.status.cmp(&b.status))
                    .then_with(|| due(a, b))
            }),
            "key" => list.sort_by(|a, b| {
                cmp_ic(KeyFinder::project_of(&a.key), KeyFinder::project_of(&b.key))
                    .then_with(|| Self::number_of(&a.key).cmp(&Self::number_of(&b.key)))
            }),
            "timing" => {
                let timing = |t: &TaskSummary| t.timing || running.contains(&t.key);
                list.sort_by_key(|t| std::cmp::Reverse(timing(t)));
            }
            _ => {}
        }
        list
    }

    pub fn category_rank(category: &str) -> usize {
        Self::CATEGORY_LABELS.iter().position(|c| c.0 == category).unwrap_or(Self::CATEGORY_LABELS.len())
    }

    /// 清單裡出現的狀態與各自的數量（多的在前）。
    pub fn statuses<'a>(tasks: impl IntoIterator<Item = &'a TaskSummary>) -> Vec<(String, usize)> {
        let mut order: Vec<String> = Vec::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for t in tasks {
            if t.status.is_empty() {
                continue;
            }
            let c = counts.entry(t.status.clone()).or_insert(0);
            if *c == 0 {
                order.push(t.status.clone());
            }
            *c += 1;
        }
        let mut list: Vec<(String, usize)> = order.into_iter().map(|s| (s.clone(), counts[&s])).collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        list
    }

    /// 清單裡出現的狀態類別與數量，依工作流程的順序。
    pub fn categories<'a>(
        tasks: impl IntoIterator<Item = &'a TaskSummary>,
    ) -> Vec<(&'static str, &'static str, usize)> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for t in tasks {
            if !t.status_category.is_empty() {
                *counts.entry(t.status_category.as_str()).or_insert(0) += 1;
            }
        }
        Self::CATEGORY_LABELS.iter().filter_map(|(c, label)| counts.get(c).map(|n| (*c, *label, *n))).collect()
    }

    fn priority_of(priority: &str) -> usize {
        Self::PRIORITY_RANK.iter().position(|p| *p == priority).unwrap_or(Self::PRIORITY_RANK.len())
    }

    /// 編號的數字部分（`int.TryParse`；不是數字排最後）。
    fn number_of(key: &str) -> i32 {
        let tail = match key.rfind('-') {
            Some(i) => &key[i + 1..],
            None => key,
        };
        tail.trim().parse::<i32>().unwrap_or(i32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // 對應 TaskOrderTests

    fn task(key: &str, status: &str, priority: &str, due_day: Option<u32>, timing: bool) -> TaskSummary {
        TaskSummary::new(
            key,
            key,
            "P",
            status,
            "",
            priority,
            None,
            due_day.map(|d| NaiveDate::from_ymd_opt(2026, 10, d).unwrap()),
            timing,
            "",
        )
    }

    fn t(key: &str) -> TaskSummary {
        task(key, "處理中", "", None, false)
    }

    fn keys(list: &[TaskSummary]) -> Vec<&str> {
        list.iter().map(|t| t.key.as_str()).collect()
    }

    #[test]
    fn sort_預計結束近到遠且沒日期的放最後() {
        let input =
            || vec![t("A-1"), task("A-2", "處理中", "", Some(20), false), task("A-3", "處理中", "", Some(5), false)];
        let sorted = TaskOrder::sort(input(), "due", &CiSet::new(), None);
        assert_eq!(vec!["A-3", "A-2", "A-1"], keys(&sorted));
        let desc = TaskOrder::sort(input(), "dueDesc", &CiSet::new(), None);
        assert_eq!(vec!["A-2", "A-3", "A-1"], keys(&desc));
    }

    #[test]
    fn sort_優先級緊急在前沒設的在後() {
        let sorted = TaskOrder::sort(
            vec![
                task("A-1", "處理中", "無", None, false),
                task("A-2", "處理中", "低", None, false),
                task("A-3", "處理中", "緊急", None, false),
                task("A-4", "處理中", "中", None, false),
            ],
            "priority",
            &CiSet::new(),
            None,
        );
        assert_eq!(vec!["A-3", "A-4", "A-2", "A-1"], keys(&sorted));
    }

    #[test]
    fn sort_編號依數字大小不是字串() {
        let sorted = TaskOrder::sort(vec![t("F2P-21"), t("EG-7"), t("F2P-5"), t("EG-11")], "key", &CiSet::new(), None);
        assert_eq!(vec!["EG-7", "EG-11", "F2P-5", "F2P-21"], keys(&sorted));
    }

    #[test]
    fn sort_計時中優先且其餘維持原順序() {
        let running: CiSet = ["A-4"].into_iter().collect();
        let sorted = TaskOrder::sort(
            vec![t("A-1"), t("A-2"), task("A-3", "處理中", "", None, true), t("A-4")],
            "timing",
            &running,
            None,
        );
        assert_eq!(vec!["A-3", "A-4", "A-1", "A-2"], keys(&sorted));
    }

    #[test]
    fn statuses_依數量多到少() {
        let statuses = TaskOrder::statuses(&[
            task("A-1", "待客戶回應", "", None, false),
            t("A-2"),
            t("A-3"),
            task("A-4", "", "", None, false),
        ]);
        assert_eq!(vec![("處理中".to_owned(), 2), ("待客戶回應".to_owned(), 1)], statuses);
    }

    // 對應 StatusFilterAndColumnsTests

    fn sc(key: &str, status: &str, category: &str) -> TaskSummary {
        TaskSummary::new(key, key, "P", status, category, "", None, None, false, "")
    }

    #[test]
    fn 狀態篩選_類別與專案狀態混著選符合任一個就顯示() {
        let mut filter = StatusFilter::new();
        let tasks =
            [sc("A-1", "處理中", "IN_PROGRESS"), sc("A-2", "待客戶回應", "IN_PROGRESS"), sc("A-3", "未開始", "TODO")];
        assert!(tasks.iter().all(|t| filter.matches(t)));

        filter.statuses.insert("待客戶回應".to_owned());
        filter.categories.insert("TODO");
        assert_eq!(
            vec!["A-2", "A-3"],
            tasks.iter().filter(|t| filter.matches(t)).map(|t| t.key.as_str()).collect::<Vec<_>>()
        );

        filter.keep_only(["IN_PROGRESS"], ["處理中", "待客戶回應"]);
        assert_eq!(1, filter.count());
    }

    #[test]
    fn 依狀態排序_同一類照專案設定的順序() {
        let tasks = vec![
            sc("A-1", "待客戶回應", "IN_PROGRESS"),
            sc("A-2", "完成", "DONE"),
            sc("A-3", "處理中", "IN_PROGRESS"),
            sc("A-4", "未開始", "TODO"),
        ];
        let order: HashMap<&str, i32> =
            [("未開始", 0), ("處理中", 1), ("待客戶回應", 2), ("完成", 3)].into_iter().collect();
        let rank = |t: &TaskSummary| order[t.status.as_str()];
        let sorted = TaskOrder::sort(tasks, "status", &CiSet::new(), Some(&rank));
        assert_eq!(vec!["A-4", "A-3", "A-1", "A-2"], keys(&sorted));
    }

    #[test]
    fn 狀態類別依流程順序統計() {
        let categories = TaskOrder::categories(&[
            sc("A-1", "完成", "DONE"),
            sc("A-2", "處理中", "IN_PROGRESS"),
            sc("A-3", "不處理", "CANCELLED"),
        ]);
        assert_eq!(vec![("IN_PROGRESS", "進行中", 1), ("DONE", "完成", 1), ("CANCELLED", "已取消", 1)], categories);
    }
}
