//! 對應 C# `Core/MyTaskTabs.cs`：「我的任務」四個分頁，從一份未完成的任務清單（my_tasks 不給分頁）在本機算，
//! 跟網頁同一套規則：進行中（含我正在計時的）、測試（指派給我的測試任務與測試結果）、未開始（待處理，不分日期）、
//! 逾期（預計結束已過，含進行中的）、該開始還沒開始。順序照清單原本的順序（跟網頁一樣）。

use chrono::NaiveDate;
use hb_api::{TaskKind, TaskSummary};
use indexmap::IndexMap;

pub struct MyTaskTabs;

fn in_progress(t: &TaskSummary) -> bool {
    t.status_category == "IN_PROGRESS" || t.timing
}

fn not_started(t: &TaskSummary) -> bool {
    t.status_category == "TODO" && !t.timing
}

impl MyTaskTabs {
    pub fn split(open: &[TaskSummary], today: NaiveDate) -> IndexMap<String, Vec<TaskSummary>> {
        let pick = |f: &dyn Fn(&TaskSummary) -> bool| open.iter().filter(|t| f(t)).cloned().collect::<Vec<_>>();
        let mut tabs = IndexMap::new();
        tabs.insert("in_progress".to_owned(), pick(&in_progress));
        tabs.insert("test".to_owned(), pick(&|t| t.kind() != TaskKind::Normal));
        tabs.insert("not_started".to_owned(), pick(&not_started));
        tabs.insert("overdue".to_owned(), pick(&|t| t.planned_end.is_some_and(|end| end < today)));
        tabs.insert(
            "should_start".to_owned(),
            pick(&|t| not_started(t) && t.planned_start.is_some_and(|start| start <= today)),
        );
        tabs
    }

    /// 「全部」分頁（Hayaku 1.5.1）：所有還沒完成的任務，進行中的排前面，其餘照清單原本的順序。
    pub fn all(open: &[TaskSummary]) -> Vec<TaskSummary> {
        open.iter().filter(|t| in_progress(t)).chain(open.iter().filter(|t| !in_progress(t))).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::TestInfo;

    // 對應 MyTaskTabsTests、Hayaku151Tests、TestingTests.MyTaskTabs_測試分頁放測試任務與測試結果

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 30).unwrap()
    }

    fn date(s: Option<&str>) -> Option<NaiveDate> {
        s.map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap())
    }

    fn task(key: &str, category: &str, start: Option<&str>, end: Option<&str>, timing: bool) -> TaskSummary {
        TaskSummary::new(key, key, "F2P", "", category, "", date(start), date(end), timing, "")
    }

    fn keys(list: &[TaskSummary]) -> Vec<&str> {
        list.iter().map(|t| t.key.as_str()).collect()
    }

    #[test]
    fn 四個分頁照網頁的規則分() {
        let tabs = MyTaskTabs::split(
            &[
                task("A", "IN_PROGRESS", Some("2026-10-01"), Some("2026-10-14"), false), // 進行中、逾期
                task("B", "TODO", None, None, false),                                    // 未開始，沒有日期
                task("C", "TODO", Some("2026-10-29"), Some("2026-11-05"), false),        // 未開始、該開始
                task("D", "TODO", Some("2026-10-30"), None, true), // 正在計時：算進行中，不算未開始、該開始
                task("E", "TODO", Some("2026-11-02"), Some("2026-10-30"), false), // 今天到期：不算逾期；還沒到開始日
                task("F", "TODO", Some("2026-10-20"), Some("2026-10-25"), false), // 未開始、該開始、逾期
            ],
            today(),
        );
        assert_eq!(vec!["A", "D"], keys(&tabs["in_progress"]));
        assert_eq!(vec!["B", "C", "E", "F"], keys(&tabs["not_started"]));
        assert_eq!(vec!["A", "F"], keys(&tabs["overdue"]));
        assert_eq!(vec!["C", "F"], keys(&tabs["should_start"]));
    }

    #[test]
    fn 分頁涵蓋所有引擎用到的分頁() {
        let mut engine: Vec<&str> = crate::TimerEngine::TASK_TABS.iter().map(|t| t.0).collect();
        engine.sort();
        let split = MyTaskTabs::split(&[], today());
        let mut tabs: Vec<&str> = split.keys().map(String::as_str).collect();
        tabs.sort();
        assert_eq!(engine, tabs);
    }

    #[test]
    fn 全部分頁_進行中排前面_其餘照原本順序() {
        let all = MyTaskTabs::all(&[
            task("A", "TODO", None, None, false),
            task("B", "IN_PROGRESS", None, None, false),
            task("C", "TODO", None, None, false),
            task("D", "TODO", None, None, true), // 正在計時算進行中
            task("E", "IN_PROGRESS", None, None, false),
        ]);
        assert_eq!(vec!["B", "D", "E", "A", "C"], keys(&all));
    }

    #[test]
    fn 全部分頁_沒有漏掉任何任務() {
        // 不屬於任何狀態分頁的類別也要在「全部」裡
        let all = MyTaskTabs::all(&[task("A", "TODO", None, None, false), task("B", "OTHER", None, None, false)]);
        assert_eq!(vec!["A", "B"], keys(&all));
    }

    #[test]
    fn my_task_tabs_測試分頁放測試任務與測試結果() {
        let normal = TaskSummary::new("A-1", "一般", "A", "處理中", "IN_PROGRESS", "無", None, None, false, "");
        let mut test = TaskSummary::new("A-2", "測試", "A", "測試中", "IN_PROGRESS", "無", None, None, false, "");
        test.test = TestInfo::new(TaskKind::Test, None, None, 0, None);
        let mut result = TaskSummary::new("A-3", "問題", "A", "待修復", "TODO", "無", None, None, false, "");
        result.test = TestInfo::new(TaskKind::TestResult, Some("Bug"), Some("一般"), 1, None);
        let tabs = MyTaskTabs::split(&[normal, test, result], NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
        assert_eq!(vec!["A-2", "A-3"], keys(&tabs["test"]));
        assert!(tabs["in_progress"].iter().any(|t| t.key == "A-2")); // 其他分頁照樣列出
    }
}
