//! 批次修改小視窗（跟上 Hayaku 1.5.5；C# 版沒有）：專案頁勾選的任務一次改一個欄位。
//! 狀態、優先級、類型、預計日期（設成同一天，或往前、往後移幾個工作日）、預估工時、里程碑（加入或移除）、自訂欄位；
//! 用 update_tasks 一次送（最多 100 筆，逐筆成功或失敗）。按鈕直接寫要套用到幾項；
//! 只有改成完成、清空欄位、移動日期時先提醒一次（再按一次才送）。外觀在 `ui/windows/batch-edit.slint`。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, NaiveDate, TimeDelta, Weekday};
use futures_util::future::join_all;
use hb_api::{BatchResult, MilestoneRef, ProjectDetail};
use hb_core::TimerEngine;
use serde_json::{Map, Value, json};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::date_range::{from_int, today_int};
use crate::dialog;
use crate::inputs;
use crate::ui::BatchEditWindow;
use crate::window_util::{self, Placement};

/// 勾選的一個任務（移日期要知道原本的預計日期）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchTask {
    pub key: String,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
}

/// 移日期改哪一個。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateTarget {
    Both,
    Start,
    End,
}

/// 要做的修改。
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Status { name: String, done: bool },
    Priority(String),
    Type(String),
    /// 預計開始、完成都設成這天；None 是清空
    SameDay(Option<NaiveDate>),
    /// 往後（正）或往前（負）移幾個工作日
    Shift { days: i64, target: DateTarget },
    /// 預估工時（小時）；None 是清空
    Hours(Option<f64>),
    Milestone { add: bool, name: String },
    /// 自訂欄位；值是 null 時清空
    Custom { name: String, value: Value },
}

impl Edit {
    /// 送出後的訊息用：「狀態改成「已完成」」。
    pub fn what(&self) -> String {
        match self {
            Self::Status { name, .. } => format!("狀態改成「{name}」"),
            Self::Priority(p) => format!("優先級改成「{p}」"),
            Self::Type(t) => format!("類型改成「{t}」"),
            Self::SameDay(Some(d)) => format!("預計日期改成 {}", inputs::date_text(Some(*d))),
            Self::SameDay(None) => "清空預計日期".to_owned(),
            Self::Shift { days, .. } if *days >= 0 => format!("預計日期往後移 {days} 個工作日"),
            Self::Shift { days, .. } => format!("預計日期往前移 {} 個工作日", -days),
            Self::Hours(Some(h)) => format!("預估工時改成 {h} 小時"),
            Self::Hours(None) => "清空預估工時".to_owned(),
            Self::Milestone { add: true, name } => format!("加入里程碑「{name}」"),
            Self::Milestone { add: false, name } => format!("移除里程碑「{name}」"),
            Self::Custom { name, value: Value::Null } => format!("清空「{name}」"),
            Self::Custom { name, .. } => format!("改了「{name}」"),
        }
    }

    /// 要先提醒的（改成完成、清空欄位、移動日期）：提醒的話；其他直接送。
    pub fn confirm(&self, n: usize) -> Option<String> {
        match self {
            Self::Status { done: true, name } => Some(format!("{n} 個任務會改成「{name}」。")),
            Self::SameDay(None) => Some(format!("{n} 個任務的預計日期會被清空。")),
            Self::Shift { .. } => Some(format!("{n} 個任務的{}。", self.what())),
            Self::Hours(None) => Some(format!("{n} 個任務的預估工時會被清空。")),
            Self::Custom { name, value: Value::Null } => Some(format!("{n} 個任務的「{name}」會被清空。")),
            _ => None,
        }
    }
}

/// 加減工作日（跳過週六、週日）；0 不動。
pub fn add_workdays(d: NaiveDate, days: i64) -> NaiveDate {
    let step = if days >= 0 { 1 } else { -1 };
    let mut left = days.abs();
    let mut cur = d;
    while left > 0 {
        cur += TimeDelta::days(step);
        if !matches!(cur.weekday(), Weekday::Sat | Weekday::Sun) {
            left -= 1;
        }
    }
    cur
}

fn date_value(d: Option<NaiveDate>) -> Value {
    d.map_or(Value::Null, |d| Value::String(inputs::date_text(Some(d))))
}

fn milestone_value(m: &MilestoneRef) -> Value {
    Value::String(m.id.clone().unwrap_or_else(|| m.name.clone()))
}

/// 每個任務要送的欄位；第二個值是略過的任務與原因。milestones：加入、移除時每個任務目前的里程碑。
pub fn plan(
    edit: &Edit,
    tasks: &[BatchTask],
    milestones: &HashMap<String, Vec<MilestoneRef>>,
) -> (Vec<Map<String, Value>>, Vec<String>) {
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for t in tasks.iter().take(100) {
        let mut m = Map::new();
        match edit {
            Edit::Status { name, .. } => {
                m.insert("status".into(), json!(name));
            }
            Edit::Priority(p) => {
                m.insert("priority".into(), json!(p));
            }
            Edit::Type(ty) => {
                m.insert("type".into(), json!(ty));
            }
            Edit::SameDay(d) => {
                m.insert("plannedStart".into(), date_value(*d));
                m.insert("plannedEnd".into(), date_value(*d));
            }
            Edit::Shift { days, target } => {
                let start = if *target != DateTarget::End { t.start } else { None };
                let end = if *target != DateTarget::Start { t.end } else { None };
                if start.is_none() && end.is_none() {
                    skipped.push(format!("{} 沒有預計日期", t.key));
                    continue;
                }
                if let Some(s) = start {
                    m.insert("plannedStart".into(), date_value(Some(add_workdays(s, *days))));
                }
                if let Some(e) = end {
                    m.insert("plannedEnd".into(), date_value(Some(add_workdays(e, *days))));
                }
            }
            Edit::Hours(h) => {
                m.insert("estimatedHours".into(), h.map_or(Value::Null, |h| json!(h)));
            }
            Edit::Milestone { add, name } => {
                let Some(current) = milestones.get(&t.key) else {
                    skipped.push(format!("{} 讀不到目前的里程碑", t.key));
                    continue;
                };
                let has = current.iter().any(|c| c.name.eq_ignore_ascii_case(name.trim()));
                if *add == has {
                    skipped.push(format!("{} {}", t.key, if *add { "已經有這個里程碑" } else { "沒有這個里程碑" }));
                    continue;
                }
                let mut list: Vec<Value> = current
                    .iter()
                    .filter(|c| *add || !c.name.eq_ignore_ascii_case(name.trim()))
                    .map(milestone_value)
                    .collect();
                if *add {
                    list.push(json!(name.trim()));
                }
                m.insert("milestones".into(), Value::Array(list));
            }
            Edit::Custom { name, value } => {
                let mut cf = Map::new();
                cf.insert(name.clone(), value.clone());
                m.insert("customFields".into(), Value::Object(cf));
            }
        }
        m.insert("task".into(), json!(t.key));
        items.push(m);
    }
    (items, skipped)
}

/// 預估工時欄：空白＝清空（Ok(None)）；不是 0～1666 的數字回 Err。
fn parse_hours(text: &str) -> Result<Option<f64>, ()> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    match t.parse::<f64>() {
        Ok(h) if h.is_finite() && (0.0..=1666.0).contains(&h) => Ok(Some(h)),
        _ => Err(()),
    }
}

/// 套用完成：結果、做了什麼、略過的任務。
pub type BatchDone = Rc<dyn Fn(Vec<BatchResult>, String, Vec<String>)>;

struct State {
    project: Rc<ProjectDetail>,
    tasks: Vec<BatchTask>,
    types: Vec<String>,
    /// 按過一次、看過提醒了（再按就送）；改了任何設定就重來
    armed: bool,
}

const PRIORITIES: [&str; 5] = ["緊急", "高", "中", "低", "無"];

fn strings(list: impl IntoIterator<Item = String>) -> ModelRc<slint::SharedString> {
    ModelRc::new(VecModel::from(list.into_iter().map(Into::into).collect::<Vec<slint::SharedString>>()))
}

fn custom_options(project: &ProjectDetail, index: i32) -> Vec<String> {
    match usize::try_from(index).ok().and_then(|i| project.custom_fields.get(i)) {
        Some(f) if !f.options.is_empty() => std::iter::once("（清空）".to_owned()).chain(f.options.iter().cloned()).collect(),
        _ => Vec::new(),
    }
}

/// 從畫面讀出要做的修改；看不懂時回錯誤訊息。
fn read_edit(ui: &BatchEditWindow, st: &State) -> Result<Edit, String> {
    let pick = |list: &[String], i: i32| usize::try_from(i).ok().and_then(|i| list.get(i).cloned());
    match ui.get_field() {
        0 => {
            let names: Vec<String> = st.project.statuses.iter().map(|s| s.name.clone()).collect();
            let name = pick(&names, ui.get_status_index()).ok_or("請選狀態")?;
            let done = st.project.statuses.iter().any(|s| s.name == name && s.category == "DONE");
            Ok(Edit::Status { name, done })
        }
        1 => Ok(Edit::Priority(
            pick(&PRIORITIES.map(str::to_owned), ui.get_priority_index()).ok_or("請選優先級")?,
        )),
        2 => Ok(Edit::Type(pick(&st.types, ui.get_type_index()).ok_or("請選類型")?)),
        3 => match ui.get_date_mode() {
            0 => Ok(Edit::SameDay(from_int(ui.get_date_day()))),
            mode => {
                let days: i64 = ui.get_days_text().trim().parse().map_err(|_| "工作日要填整數，例 1、5")?;
                if !(1..=365).contains(&days) {
                    return Err("工作日要填 1～365".to_owned());
                }
                let target = match ui.get_date_target() {
                    1 => DateTarget::Start,
                    2 => DateTarget::End,
                    _ => DateTarget::Both,
                };
                Ok(Edit::Shift { days: if mode == 2 { -days } else { days }, target })
            }
        },
        4 => parse_hours(&ui.get_hours_text())
            .map(Edit::Hours)
            .map_err(|()| "預估工時要填 0～1666 的數字，例 4、1.5".to_owned()),
        5 => {
            let name = ui.get_milestone_text().trim().to_owned();
            if name.is_empty() {
                return Err("請填里程碑名稱".to_owned());
            }
            Ok(Edit::Milestone { add: ui.get_milestone_mode() == 0, name })
        }
        _ => {
            let i = ui.get_custom_index();
            let Some(def) = usize::try_from(i).ok().and_then(|i| st.project.custom_fields.get(i)) else {
                return Err("這個專案沒有自訂欄位".to_owned());
            };
            let value = if def.options.is_empty() {
                let text = ui.get_custom_text().trim().to_owned();
                if text.is_empty() {
                    Value::Null
                } else if def.kind.to_ascii_uppercase().contains("NUMBER")
                    && let Ok(n) = text.parse::<f64>()
                {
                    json!(n)
                } else {
                    json!(text)
                }
            } else {
                match custom_options(&st.project, i).get(usize::try_from(ui.get_custom_option_index()).unwrap_or(0)) {
                    Some(o) if ui.get_custom_option_index() > 0 => {
                        if def.is_multi() {
                            json!([o])
                        } else {
                            json!(o)
                        }
                    }
                    _ => Value::Null,
                }
            };
            Ok(Edit::Custom { name: def.name.clone(), value })
        }
    }
}

fn reset_arm(ui: &BatchEditWindow, st: &mut State) {
    st.armed = false;
    ui.set_warning("".into());
    ui.set_error("".into());
    ui.set_apply_text(format!("套用到 {} 項", st.tasks.len().min(100)).into());
}

/// 開批次修改視窗（同一個專案只開一個）。types：可以改成的類型；done：送出後（成功或部分失敗）呼叫，視窗會關掉。
pub fn open(engine: &TimerEngine, project: Rc<ProjectDetail>, tasks: Vec<BatchTask>, types: Vec<String>, done: BatchDone) {
    let tag = format!("batch-edit:{}", project.key);
    if window_util::activate_tag(&tag) || tasks.is_empty() {
        return;
    }
    let Some(ui) = dialog::create("批次修改", BatchEditWindow::new) else { return };
    let n = tasks.len();
    ui.set_heading(
        if n > 100 {
            format!("批次修改 · 已選 {n} 個（一次最多改前 100 個）")
        } else {
            format!("批次修改 · 已選 {n} 個任務")
        }
        .into(),
    );
    ui.set_statuses(strings(project.statuses.iter().map(|s| s.name.clone())));
    ui.set_status_dots(crate::status_dots(&project.statuses));
    ui.set_priorities(strings(PRIORITIES.map(str::to_owned)));
    ui.set_types(strings(types.clone()));
    ui.set_today(today_int());
    ui.set_custom_fields(strings(project.custom_fields.iter().map(|f| f.name.clone())));
    ui.set_custom_options(strings(custom_options(&project, 0)));
    let st = Rc::new(RefCell::new(State { project, tasks, types, armed: false }));
    reset_arm(&ui, &mut st.borrow_mut());
    let id = window_util::keep(&ui, Some(tag), ());
    {
        let (weak, st) = (ui.as_weak(), st.clone());
        ui.on_changed(move || {
            if let Some(ui) = weak.upgrade() {
                reset_arm(&ui, &mut st.borrow_mut());
            }
        });
    }
    {
        let (weak, st) = (ui.as_weak(), st.clone());
        ui.on_custom_selected(move |i| {
            if let Some(ui) = weak.upgrade() {
                ui.set_custom_options(strings(custom_options(&st.borrow().project, i)));
                ui.set_custom_option_index(0);
                ui.set_custom_text("".into());
            }
        });
    }
    {
        let (weak, st, engine) = (ui.as_weak(), st.clone(), engine.clone());
        ui.on_apply(move || {
            let Some(ui) = weak.upgrade() else { return };
            let edit = match read_edit(&ui, &st.borrow()) {
                Ok(e) => e,
                Err(message) => {
                    ui.set_error(message.into());
                    return;
                }
            };
            let count = st.borrow().tasks.len().min(100);
            if !st.borrow().armed
                && let Some(warning) = edit.confirm(count)
            {
                st.borrow_mut().armed = true;
                ui.set_warning(format!("{warning}確定要改的話再按一次。").into());
                ui.set_apply_text(format!("確定套用到 {count} 項").into());
                return;
            }
            let tasks = st.borrow().tasks.clone();
            let (engine, done, weak) = (engine.clone(), done.clone(), weak.clone());
            ui.set_busy(true);
            ui.set_error("".into());
            dialog::spawn(async move {
                let api = engine.api();
                let result: Result<(Vec<BatchResult>, Vec<String>), String> = async {
                    let mut milestones = HashMap::new();
                    if matches!(edit, Edit::Milestone { .. }) {
                        for chunk in tasks.iter().take(100).collect::<Vec<_>>().chunks(5) {
                            for (t, r) in chunk.iter().zip(join_all(chunk.iter().map(|t| api.get_task(&t.key))).await) {
                                match r {
                                    Ok(d) => {
                                        milestones.insert(t.key.clone(), d.milestones);
                                    }
                                    Err(e) => log::error!("讀取 {} 的里程碑失敗: {}", t.key, e.0),
                                }
                            }
                        }
                    }
                    let (items, skipped) = plan(&edit, &tasks, &milestones);
                    if items.is_empty() {
                        return Err(format!("沒有要改的任務：{}", skipped.join("、")));
                    }
                    let results = api.update_tasks(&items).await.map_err(|e| e.0)?;
                    Ok((results, skipped))
                }
                .await;
                let Some(ui) = weak.upgrade() else { return };
                match result {
                    Ok((results, skipped)) => {
                        window_util::close(&ui, id);
                        done(results, edit.what(), skipped);
                    }
                    Err(message) => {
                        log::error!("批次修改失敗: {message}");
                        ui.set_error(message.into());
                        ui.set_busy(false);
                    }
                }
            });
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                window_util::close(&ui, id);
            }
        });
    }
    window_util::present(&ui, id, Placement::Screen, true, (460.0, 420.0), crate::apply_theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    fn task(key: &str, start: Option<NaiveDate>, end: Option<NaiveDate>) -> BatchTask {
        BatchTask { key: key.into(), start, end }
    }

    #[test]
    fn 工作日跳過週末() {
        // 2026-11-13 是週五
        assert_eq!(add_workdays(day(11, 13), 1), day(11, 16));
        assert_eq!(add_workdays(day(11, 16), -1), day(11, 13));
        assert_eq!(add_workdays(day(11, 14), 0), day(11, 14));
        assert_eq!(add_workdays(day(11, 9), 5), day(11, 16));
    }

    #[test]
    fn 移日期_沒有日期的略過_只改完成() {
        let tasks = [task("A-1", Some(day(11, 12)), Some(day(11, 13))), task("A-2", None, None)];
        let (items, skipped) = plan(&Edit::Shift { days: 2, target: DateTarget::Both }, &tasks, &HashMap::new());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["plannedStart"], json!("2026-11-16"));
        assert_eq!(items[0]["plannedEnd"], json!("2026-11-17"));
        assert_eq!(skipped, vec!["A-2 沒有預計日期"]);
        let (items, _) = plan(&Edit::Shift { days: -1, target: DateTarget::End }, &tasks, &HashMap::new());
        assert!(items[0].get("plannedStart").is_none());
        assert_eq!(items[0]["plannedEnd"], json!("2026-11-12"));
    }

    #[test]
    fn 同一天與清空_工時_自訂欄位() {
        let tasks = [task("A-1", None, None)];
        let (items, _) = plan(&Edit::SameDay(Some(day(11, 20))), &tasks, &HashMap::new());
        assert_eq!((items[0]["plannedStart"].clone(), items[0]["plannedEnd"].clone()), (json!("2026-11-20"), json!("2026-11-20")));
        let (items, _) = plan(&Edit::SameDay(None), &tasks, &HashMap::new());
        assert_eq!(items[0]["plannedEnd"], Value::Null);
        let (items, _) = plan(&Edit::Hours(Some(1.5)), &tasks, &HashMap::new());
        assert_eq!(items[0]["estimatedHours"], json!(1.5));
        let (items, _) = plan(&Edit::Custom { name: "客戶".into(), value: json!("甲") }, &tasks, &HashMap::new());
        assert_eq!(items[0]["customFields"], json!({ "客戶": "甲" }));
        assert_eq!(items[0]["task"], json!("A-1"));
    }

    #[test]
    fn 里程碑_加入移除_已經有或沒有的略過() {
        let tasks = [task("A-1", None, None), task("A-2", None, None), task("A-3", None, None)];
        let mut ms = HashMap::new();
        ms.insert("A-1".to_owned(), vec![MilestoneRef { id: Some("m1".into()), name: "上線".into() }]);
        ms.insert("A-2".to_owned(), vec![]);
        let (items, skipped) = plan(&Edit::Milestone { add: true, name: "交付".into() }, &tasks, &ms);
        assert_eq!(items[0]["milestones"], json!(["m1", "交付"]));
        assert_eq!(items[1]["milestones"], json!(["交付"]));
        assert_eq!(skipped, vec!["A-3 讀不到目前的里程碑"]);
        let (items, skipped) = plan(&Edit::Milestone { add: false, name: "上線".into() }, &tasks[..2], &ms);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["milestones"], json!([]));
        assert_eq!(skipped, vec!["A-2 沒有這個里程碑"]);
    }

    #[test]
    fn 只有完成清空移日期要提醒() {
        assert!(Edit::Status { name: "已完成".into(), done: true }.confirm(3).is_some());
        assert!(Edit::Status { name: "處理中".into(), done: false }.confirm(3).is_none());
        assert!(Edit::Priority("高".into()).confirm(3).is_none());
        assert!(Edit::SameDay(None).confirm(3).is_some());
        assert!(Edit::SameDay(Some(day(11, 1))).confirm(3).is_none());
        assert_eq!(
            Edit::Shift { days: -2, target: DateTarget::Both }.confirm(3).as_deref(),
            Some("3 個任務的預計日期往前移 2 個工作日。")
        );
        assert!(Edit::Hours(None).confirm(1).is_some());
        assert!(Edit::Custom { name: "x".into(), value: Value::Null }.confirm(1).is_some());
        assert_eq!(parse_hours(" "), Ok(None));
        assert!(parse_hours("abc").is_err());
    }
}
