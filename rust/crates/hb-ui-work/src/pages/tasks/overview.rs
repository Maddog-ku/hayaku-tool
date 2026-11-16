//! 跟上 Hayaku 1.5.5（C# 版沒有）：任務詳細的「概況」——子任務進度、逾期、工時、會不會延誤、每週趨勢、各人投入，一塊看完。
//! 平常收起，展開才讀（同一個任務只讀一次）：get_task_tree（3 層）＋這個任務與底下任務的計時
//! （list_time_entries 帶 task，最多 30 個任務，一次 5 個並行）。

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{Datelike, Local, NaiveDate, TimeDelta};
use futures_util::future::join_all;
use hb_api::{TaskDetail, TaskTreeRow, TimeEntry, WorkMath};
use slint::SharedString;

use super::Page;
use super::detail::entry_minutes;
use crate::shared::{self, vec_model};
use crate::{TasksPageData, TkOvBar, TkLine, TkStat};

/// 概況最多讀幾個任務的計時（這個任務＋底下的）。
const MAX_TASKS: usize = 30;
/// 每週趨勢列幾週（含本週）。
const WEEKS: i64 = 8;

/// 會不會延誤的判斷：色調 0 正常、1 注意、2 已逾期、3 無法判斷。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Risk {
    pub tone: i32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Overview {
    /// 子任務（所有層）：完成（含不處理）、進行中、未開始、總數
    pub done: usize,
    pub doing: usize,
    pub todo: usize,
    pub total: usize,
    /// 逾期的子任務（編號、名稱、預計完成日），預計完成日早的在前
    pub overdue: Vec<(String, String, NaiveDate)>,
    /// 讀到的計時合計（分鐘）
    pub logged: f64,
    pub risk: Risk,
    /// 最近幾週（週一的日期、分鐘），舊的在前
    pub weeks: Vec<(NaiveDate, f64)>,
    /// 各人投入（名稱、分鐘），多的在前
    pub people: Vec<(String, f64)>,
}

fn is_closed(category: Option<&str>) -> bool {
    matches!(category, Some("DONE" | "CANCELLED"))
}

fn week_start(d: NaiveDate) -> NaiveDate {
    d - TimeDelta::days(i64::from(d.weekday().num_days_from_monday()))
}

/// 會不會延誤：完成了就不判斷；任務本身過了預計完成日是逾期；有子任務預計比任務晚完成、有子任務逾期、
/// 工時超出預估是注意；沒有預計完成日無法判斷。
pub(super) fn judge(d: &TaskDetail, tree: &[TaskTreeRow], overdue: usize, logged: f64, today: NaiveDate) -> Risk {
    let risk = |tone: i32, text: String| Risk { tone, text };
    if is_closed(Some(d.status_category.as_str())) {
        return risk(0, "已經完成".to_owned());
    }
    if let Some(end) = d.planned_end
        && end < today
    {
        return risk(2, format!("已逾期 {} 天（預計 {} 完成）", (today - end).num_days(), end.format("%m/%d")));
    }
    if let Some(end) = d.planned_end
        && let Some(late) = tree
            .iter()
            .filter(|r| !is_closed(r.status_category.as_deref()))
            .filter(|r| r.planned_end.is_some_and(|e| e > end))
            .max_by_key(|r| r.planned_end)
    {
        return risk(
            1,
            format!(
                "可能延誤：{} 預計 {} 才完成，比這個任務的預計完成日（{}）晚",
                late.key,
                late.planned_end.map(|e| e.format("%m/%d").to_string()).unwrap_or_default(),
                end.format("%m/%d")
            ),
        );
    }
    if overdue > 0 {
        return risk(1, format!("可能延誤：有 {overdue} 個子任務逾期"));
    }
    if let Some(est) = d.estimated_hours.filter(|h| *h > 0.0)
        && logged > est * 60.0
    {
        return risk(1, format!("工時已超出預估 {}", WorkMath::hours_minutes(logged - est * 60.0)));
    }
    match d.planned_end {
        None => risk(3, "沒有預計完成日，無法判斷會不會延誤".to_owned()),
        Some(end) => {
            let left = (end - today).num_days();
            let when = if left == 0 { "今天".to_owned() } else { format!("還有 {left} 天") };
            risk(0, format!("照目前進度不會延誤（預計 {} 完成，{when}）", end.format("%m/%d")))
        }
    }
}

pub(super) fn build_overview(
    d: &TaskDetail,
    tree: &[TaskTreeRow],
    entries: &[TimeEntry],
    now: &hb_api::DateTimeOffset,
) -> Overview {
    let today = now.with_timezone(&Local).date_naive();
    let (mut done, mut doing, mut todo) = (0, 0, 0);
    let mut overdue = Vec::new();
    for r in tree {
        match r.status_category.as_deref() {
            Some("DONE" | "CANCELLED") => done += 1,
            Some("IN_PROGRESS") => doing += 1,
            _ => todo += 1,
        }
        if !is_closed(r.status_category.as_deref())
            && let Some(end) = r.planned_end.filter(|e| *e < today)
        {
            overdue.push((r.key.clone(), r.title.clone(), end));
        }
    }
    overdue.sort_by_key(|o| o.2);
    let logged: f64 = entries.iter().map(|t| entry_minutes(t, now)).sum();
    let this_week = week_start(today);
    let weeks = (0..WEEKS)
        .rev()
        .map(|i| {
            let from = this_week - TimeDelta::weeks(i);
            let to = from + TimeDelta::days(7);
            let minutes = entries
                .iter()
                .filter(|t| {
                    t.started_at.is_some_and(|s| {
                        let d = s.with_timezone(&Local).date_naive();
                        d >= from && d < to
                    })
                })
                .map(|t| entry_minutes(t, now))
                .sum();
            (from, minutes)
        })
        .collect();
    let mut people: Vec<(String, f64)> = Vec::new();
    for t in entries {
        let Some(user) = t.user.as_deref().filter(|u| !u.is_empty()) else { continue };
        match people.iter_mut().find(|p| p.0 == user) {
            Some(p) => p.1 += entry_minutes(t, now),
            None => people.push((user.to_owned(), entry_minutes(t, now))),
        }
    }
    people.sort_by(|a, b| b.1.total_cmp(&a.1));
    let risk = judge(d, tree, overdue.len(), logged, today);
    Overview { done, doing, todo, total: tree.len(), overdue, logged, risk, weeks, people }
}

/// 長條：最大的是 1。
fn bars(rows: &[(String, f64)]) -> Vec<TkOvBar> {
    let max = rows.iter().map(|r| r.1).fold(0.0, f64::max);
    rows.iter()
        .map(|(label, m)| TkOvBar {
            label: label.as_str().into(),
            value: if *m > 0.0 { WorkMath::hours_minutes(*m) } else { "—".to_owned() }.into(),
            ratio: if max > 0.0 { (*m / max) as f32 } else { 0.0 },
        })
        .collect()
}

/// 每週趨勢的長條：標籤是那週週一（11/10），值是小時（一位小數，長條上方的小字放不下「小時 分」）。
fn week_bars(weeks: &[(NaiveDate, f64)]) -> Vec<TkOvBar> {
    let rows: Vec<(String, f64)> = weeks.iter().map(|(d, m)| (d.format("%m/%d").to_string(), *m)).collect();
    let mut list = bars(&rows);
    for (b, (_, m)) in list.iter_mut().zip(weeks) {
        b.value = format!("{:.1}", m / 60.0).trim_end_matches(".0").to_owned().into();
    }
    list
}

/// 讀的是哪個任務（換任務就作廢）。
#[derive(Default)]
struct OverviewState {
    key: Option<String>,
    version: u64,
}

thread_local! {
    static OVERVIEW: RefCell<OverviewState> = RefCell::new(OverviewState::default());
}

impl Page {
    pub(super) fn wire_overview(self: &Rc<Self>, d: &TasksPageData) {
        use super::on;
        on!(d, self, on_d_ov_toggled, |p, open| {
            if open {
                shared::spawn("讀取任務概況", async move { p.load_overview().await });
            }
        });
    }

    /// 換了任務：收起、清空，下次展開再讀；同一個任務重畫（例如改了欄位）就保留。
    pub(super) fn reset_overview(&self, d: &TaskDetail) {
        let reset = OVERVIEW.with(|o| {
            let mut o = o.borrow_mut();
            let reset = o.key.as_deref() != Some(d.key.as_str());
            if reset {
                *o = OverviewState { key: None, version: o.version + 1 };
            }
            reset
        });
        if reset {
            self.with_data(|x| {
                x.set_d_ov_expanded(false);
                x.set_d_ov_message(SharedString::new());
                x.set_d_ov_error(false);
                x.set_d_ov_risk(SharedString::new());
                x.set_d_ov_stats(vec_model(Vec::new()));
                x.set_d_ov_progress(0.0);
                x.set_d_ov_progress_text(SharedString::new());
                x.set_d_ov_weeks(vec_model(Vec::new()));
                x.set_d_ov_people(vec_model(Vec::new()));
                x.set_d_ov_overdue(vec_model(Vec::new()));
            });
        }
    }

    async fn load_overview(self: Rc<Self>) {
        let Some(detail) = self.st.borrow().detail.clone() else { return };
        let key = detail.key.clone();
        let Some(version) = OVERVIEW.with(|o| {
            let mut o = o.borrow_mut();
            if o.key.as_deref() == Some(key.as_str()) {
                return None;
            }
            o.key = Some(key.clone());
            o.version += 1;
            Some(o.version)
        }) else {
            return;
        };
        self.with_data(|d| {
            d.set_d_ov_message("讀取中…".into());
            d.set_d_ov_error(false);
        });
        let api = self.engine().api();
        let result = async {
            let (tree, truncated) =
                if detail.children.is_empty() { (Vec::new(), false) } else { api.task_tree(&key, 3).await? };
            let keys: Vec<String> =
                std::iter::once(key.clone()).chain(tree.iter().map(|r| r.key.clone())).take(MAX_TASKS).collect();
            let mut entries: Vec<TimeEntry> = Vec::new();
            for chunk in keys.chunks(5) {
                for r in join_all(chunk.iter().map(|k| api.list_task_entries(k))).await {
                    entries.extend(r?);
                }
            }
            let partial = truncated || tree.len() + 1 > MAX_TASKS;
            Ok::<_, hb_api::McpError>((tree, entries, partial))
        }
        .await;
        if OVERVIEW.with(|o| o.borrow().version) != version
            || self.st.borrow().detail.as_ref().is_none_or(|d| d.key != key)
        {
            return;
        }
        let (tree, entries, partial) = match result {
            Ok(r) => r,
            Err(e) => {
                log::error!("讀取 {key} 的概況失敗: {}", e.0);
                OVERVIEW.with(|o| o.borrow_mut().key = None);
                self.with_data(|d| {
                    d.set_d_ov_message(format!("讀取失敗：{}", e.0).into());
                    d.set_d_ov_error(true);
                });
                return;
            }
        };
        let now = Local::now().fixed_offset();
        let ov = build_overview(&detail, &tree, &entries, &now);
        let mut stats = Vec::new();
        if ov.total > 0 {
            stats.push(TkStat { label: "子任務".into(), value: format!("{} / {} 完成", ov.done, ov.total).into() });
            stats.push(TkStat { label: "逾期".into(), value: format!("{} 個", ov.overdue.len()).into() });
        }
        stats.push(TkStat { label: "工時".into(), value: WorkMath::hours_minutes(ov.logged).into() });
        if let Some(est) = detail.estimated_hours.filter(|h| *h > 0.0) {
            stats.push(TkStat { label: "預估".into(), value: WorkMath::hours_minutes(est * 60.0).into() });
        }
        let progress_text = if ov.total == 0 {
            String::new()
        } else {
            format!("完成 {}・進行中 {}・未開始 {}", ov.done, ov.doing, ov.todo)
        };
        let weeks = week_bars(&ov.weeks);
        let overdue: Vec<TkLine> = ov
            .overdue
            .iter()
            .take(8)
            .map(|(k, t, end)| TkLine {
                kind: 4,
                key: k.as_str().into(),
                text: t.as_str().into(),
                meta: format!("· 預計 {} 完成", end.format("%m/%d")).into(),
                ..Default::default()
            })
            .collect();
        let message = if partial { format!("工時只算這個任務和底下前 {} 個任務", MAX_TASKS - 1) } else { String::new() };
        self.with_data(|d| {
            d.set_d_ov_message(message.into());
            d.set_d_ov_risk(ov.risk.text.as_str().into());
            d.set_d_ov_risk_tone(ov.risk.tone);
            d.set_d_ov_stats(vec_model(stats));
            d.set_d_ov_progress(if ov.total > 0 { ov.done as f32 / ov.total as f32 } else { 0.0 });
            d.set_d_ov_progress_text(progress_text.into());
            d.set_d_ov_weeks(vec_model(weeks));
            d.set_d_ov_people(vec_model(bars(&ov.people)));
            d.set_d_ov_overdue(vec_model(overdue));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn detail(category: &str, end: Option<NaiveDate>, est: Option<f64>) -> TaskDetail {
        let mut d = hb_api::HayakuApi::parse_task_detail(&serde_json::json!({ "task": "F2P-1", "title": "t" }));
        d.status_category = category.to_owned();
        d.planned_end = end;
        d.estimated_hours = est;
        d
    }

    fn row(key: &str, category: &str, end: Option<NaiveDate>) -> TaskTreeRow {
        TaskTreeRow {
            key: key.into(),
            title: format!("{key} 名稱"),
            status: String::new(),
            status_category: Some(category.into()),
            depth: 1,
            assignees: Vec::new(),
            priority: None,
            planned_start: None,
            planned_end: end,
        }
    }

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn entry(user: &str, d: u32, minutes: i32) -> TimeEntry {
        let start = Local.with_ymd_and_hms(2026, 11, d, 10, 0, 0).unwrap().fixed_offset();
        let mut t = TimeEntry::new("e", "F2P-1", "t", Some(start), None, Some(minutes), false, None);
        t.user = Some(user.into());
        t
    }

    #[test]
    fn 概況_進度_逾期_每週_各人() {
        let now = Local.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap().fixed_offset();
        let d = detail("IN_PROGRESS", Some(day(11, 30)), None);
        let tree = vec![
            row("F2P-2", "DONE", Some(day(11, 1))),
            row("F2P-3", "IN_PROGRESS", Some(day(11, 10))),
            row("F2P-4", "TODO", Some(day(11, 5))),
            row("F2P-5", "CANCELLED", None),
        ];
        let entries = vec![entry("小明", 16, 60), entry("小華", 10, 30), entry("小明", 2, 90)];
        let ov = build_overview(&d, &tree, &entries, &now);
        assert_eq!((ov.done, ov.doing, ov.todo, ov.total), (2, 1, 1, 4));
        assert_eq!(ov.overdue.iter().map(|o| o.0.as_str()).collect::<Vec<_>>(), vec!["F2P-4", "F2P-3"]);
        assert_eq!(ov.logged, 180.0);
        assert_eq!(ov.weeks.len(), 8);
        assert_eq!(ov.weeks[7], (day(11, 16), 60.0));
        assert_eq!(ov.weeks[6], (day(11, 9), 30.0));
        assert_eq!(ov.weeks[5], (day(11, 2), 90.0));
        assert_eq!(ov.people, vec![("小明".to_owned(), 150.0), ("小華".to_owned(), 30.0)]);
        assert_eq!(ov.risk, Risk { tone: 1, text: "可能延誤：有 2 個子任務逾期".into() });
    }

    #[test]
    fn 會不會延誤() {
        let today = day(11, 16);
        let r = |d: &TaskDetail, tree: &[TaskTreeRow], overdue: usize, logged: f64| judge(d, tree, overdue, logged, today);
        assert_eq!(r(&detail("DONE", Some(day(11, 1)), None), &[], 0, 0.0).tone, 0);
        assert_eq!(
            r(&detail("IN_PROGRESS", Some(day(11, 13)), None), &[], 0, 0.0),
            Risk { tone: 2, text: "已逾期 3 天（預計 11/13 完成）".into() }
        );
        let late = [row("F2P-9", "TODO", Some(day(11, 25)))];
        assert_eq!(
            r(&detail("TODO", Some(day(11, 20)), None), &late, 0, 0.0).text,
            "可能延誤：F2P-9 預計 11/25 才完成，比這個任務的預計完成日（11/20）晚"
        );
        assert_eq!(r(&detail("TODO", Some(day(11, 20)), Some(1.0)), &[], 0, 90.0).text, "工時已超出預估 30 分");
        assert_eq!(r(&detail("TODO", None, None), &[], 0, 0.0).tone, 3);
        assert_eq!(
            r(&detail("TODO", Some(day(11, 16)), None), &[], 0, 0.0).text,
            "照目前進度不會延誤（預計 11/16 完成，今天）"
        );
    }

    #[test]
    fn 長條_最大的是一() {
        let b = bars(&[("a".into(), 30.0), ("b".into(), 0.0), ("c".into(), 60.0)]);
        assert_eq!(b.iter().map(|x| x.ratio).collect::<Vec<_>>(), vec![0.5, 0.0, 1.0]);
        assert_eq!(b[1].value.as_str(), "—");
        let w = week_bars(&[(day(11, 9), 90.0), (day(11, 16), 120.0)]);
        assert_eq!(
            w.iter().map(|x| (x.label.as_str(), x.value.as_str())).collect::<Vec<_>>(),
            vec![("11/09", "1.5"), ("11/16", "2")]
        );
    }
}
