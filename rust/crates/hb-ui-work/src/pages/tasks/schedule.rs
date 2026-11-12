//! 對應 C# `UI/Work/TasksPage.Schedule.cs`：我的任務的日曆、甘特圖共用：任務條的外觀、拖拉、改預計日期（Hayaku 1.5.2）。
//! 資料跟清單同一份（目前分頁＋專案、狀態、只顯示沒有子任務的篩選），不多讀；放開滑鼠才送一次 update_task。

use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, NaiveDate};
use hb_api::{KeyFinder, TaskSummary};
use hb_core::TaskSchedule;
use hb_ui_common::{ProjectIcons, TaskText, inputs};
use serde_json::{Map, Value};

use super::Page;
use crate::shared;
use crate::{TasksPageData, TkBar};

/// 拖拉中的任務條。
#[derive(Debug, Clone)]
pub(super) struct DragState {
    pub key: String,
    pub grab_day: Option<NaiveDate>,
    /// 沒有預計日期的任務：拖到哪一天（或拉出哪一段）就設成那樣。
    pub undated: bool,
    pub started: bool,
    pub preview: (Option<NaiveDate>, Option<NaiveDate>),
}

/// 日曆、甘特的狀態（C# Schedule.cs、Calendar.cs、Gantt.cs 的欄位）。
pub(super) struct ScheduleState {
    /// 剛拖完、還在等伺服器的新日期：先畫在新位置，同步完（或失敗）才拿掉。鍵是大寫編號。
    pub overrides: HashMap<String, (Option<NaiveDate>, Option<NaiveDate>)>,
    pub tasks: Vec<TaskSummary>,
    pub view: Option<&'static str>,
    pub drag: Option<DragState>,
    // 日曆
    pub calendar_month: NaiveDate,
    pub calendar_first: NaiveDate,
    pub calendar_weeks: i32,
    pub calendar_height: f32,
    /// 每個「+N」放不下的任務（編號、名稱）
    pub calendar_more: Vec<Vec<(String, String)>>,
    // 甘特
    pub gantt_start: NaiveDate,
    pub gantt_days: i32,
    pub gantt_day_width: f32,
    pub gantt_width: f32,
    /// 甘特圖每一列的任務（index 跟 gantt-rows 一樣）
    pub gantt_rows: Vec<TaskSummary>,
}

impl ScheduleState {
    pub fn new(today: NaiveDate) -> Self {
        Self {
            overrides: HashMap::new(),
            tasks: Vec::new(),
            view: None,
            drag: None,
            calendar_month: NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today),
            calendar_first: today,
            calendar_weeks: 5,
            calendar_height: 0.0,
            calendar_more: Vec::new(),
            gantt_start: super::gantt::default_start(today),
            gantt_days: 42,
            gantt_day_width: super::gantt::MIN_DAY_WIDTH,
            gantt_width: 0.0,
            gantt_rows: Vec::new(),
        }
    }

    pub fn dates_of(&self, t: &TaskSummary) -> (Option<NaiveDate>, Option<NaiveDate>) {
        self.overrides.get(&t.key.to_uppercase()).copied().unwrap_or((t.planned_start, t.planned_end))
    }

    pub fn task(&self, key: &str) -> Option<TaskSummary> {
        self.tasks.iter().find(|t| t.key.eq_ignore_ascii_case(key)).cloned()
    }
}

pub(super) fn is_done(t: &TaskSummary) -> bool {
    matches!(t.status_category.as_str(), "DONE" | "CANCELLED")
}

/// 逾期：還沒完成、畫出來的範圍結束在今天以前。
pub(super) fn is_overdue(t: &TaskSummary, dates: (Option<NaiveDate>, Option<NaiveDate>), today: NaiveDate) -> bool {
    !is_done(t) && TaskSchedule::span(dates.0, dates.1).is_some_and(|s| s.1 < today)
}

/// 任務條的提示（C# ScheduleBar 的 ToolTip）。
pub(super) fn bar_tip(t: &TaskSummary, dates: (Option<NaiveDate>, Option<NaiveDate>)) -> String {
    format!(
        "{} {}\n{}・{}\n預計 {}\n點一下查看；拖拉改預計日期",
        t.key,
        TaskText::title_of(t),
        t.project,
        t.status,
        TaskSchedule::range_text(dates.0, dates.1)
    )
}

impl Page {
    pub(super) fn wire_schedule(self: &Rc<Self>, d: &TasksPageData) {
        super::on!(d, self, on_schedule_shift, |p, dir| p.schedule_shift(dir));
        super::on!(d, self, on_bar_press, |p, key, undated, day| p.calendar_press(&key, undated, day));
        super::on!(d, self, on_bar_drag, |p, key, day| p.calendar_drag(&key, day));
        super::on!(d, self, on_bar_release, |p, key, started| p.calendar_release(&key, started));
        super::on!(d, self, on_bar_cancel, |p| {
            p.with_data(|d| {
                d.set_cal_hl_from(-1);
                d.set_cal_hl_to(-1);
            });
            p.cancel_drag();
        });
        super::on!(d, self, on_cal_resized, |p, h| p.calendar_resized(h));
        super::on!(d, self, on_cal_more_clicked, |p, i| p.calendar_more(i as usize));
        super::on!(d, self, on_more_pick, |p, key| p.open_key(&key));
        super::on!(d, self, on_gantt_resized, |p, w| p.gantt_resized(w));
        super::on!(d, self, on_gantt_open, |p, key| p.open_key(&key));
        super::on!(d, self, on_gantt_jump, |p, row| p.gantt_jump(row as usize));
        super::on!(d, self, on_gantt_drag, |p, row, mode, delta| p.gantt_drag(row as usize, mode, delta));
        super::on!(d, self, on_gantt_draw_start, |p, row, day| p.gantt_draw_start(row as usize, day));
        super::on!(d, self, on_gantt_draw_move, |p, row, day| p.gantt_draw_move(row as usize, day));
        super::on!(d, self, on_gantt_release, |p, row| p.gantt_release(row as usize));
    }

    /// render_list 畫完清單後呼叫：view 是 calendar、gantt 或 None（不是這兩種）。
    pub(super) fn render_schedule(&self, view: Option<&'static str>, shown: Vec<TaskSummary>) {
        {
            let mut st = self.st.borrow_mut();
            st.schedule.view = view;
            st.schedule.tasks = shown;
            if st.schedule.drag.as_ref().is_some_and(|d| d.started) {
                return; // 拖拉中不重畫（會中斷拖拉）；放開時一定會重畫
            }
        }
        match view {
            Some("calendar") => self.render_calendar(),
            Some("gantt") => self.render_gantt(),
            _ => {}
        }
    }

    pub(super) fn rerender_schedule(&self) {
        let view = self.st.borrow().schedule.view;
        match view {
            Some("calendar") => self.render_calendar(),
            Some("gantt") => self.render_gantt(),
            _ => {}
        }
    }

    /// 任務條：專案顏色的淡底＋實線邊，逾期是紅邊，完成的變淡，計時中前面加 ●；甘特圖左邊已經有編號，條上只放名稱。
    pub(super) fn schedule_bar(&self, t: &TaskSummary, from_before: bool, to_after: bool, show_key: bool) -> TkBar {
        let dates = self.st.borrow().schedule.dates_of(t);
        let project =
            self.engine().project_of(&t.project).or_else(|| self.engine().project_of(KeyFinder::project_of(&t.key)));
        let color = ProjectIcons::color_of(
            project.as_ref().and_then(|p| p.icon_color.as_deref()),
            project.as_ref().map_or(KeyFinder::project_of(&t.key), |p| p.key.as_str()),
        );
        TkBar {
            key: t.key.as_str().into(),
            from_before,
            to_after,
            show_key,
            title: TaskText::title_of(t).into(),
            running: t.timing || self.engine().running_keys().contains(&t.key),
            tint: color.to_slint(),
            overdue: is_overdue(t, dates, Self::today()),
            done: is_done(t),
            tip: bar_tip(t, dates).into(),
            ..Default::default()
        }
    }

    fn schedule_shift(&self, dir: i32) {
        let view = self.st.borrow().schedule.view;
        match view {
            Some("calendar") => self.shift_month(dir),
            Some("gantt") => self.shift_gantt(dir),
            _ => {}
        }
    }

    /// 放開：沒拖動算點一下（打開任務），拖了就改日期。
    pub(super) fn end_drag(&self) {
        let d = self.st.borrow_mut().schedule.drag.take();
        self.with_data(|x| x.set_drag_key(Default::default()));
        let Some(d) = d else { return };
        if !d.started {
            self.open_key(&d.key);
            return;
        }
        let Some(task) = self.st.borrow().schedule.task(&d.key) else {
            self.rerender_schedule();
            return;
        };
        let (old_start, old_end) = self.st.borrow().schedule.dates_of(&task);
        if d.preview.0.is_none() && d.preview.1.is_none()
            || TaskSchedule::changes(old_start, old_end, d.preview.0, d.preview.1).is_empty()
        {
            self.rerender_schedule();
            return;
        }
        if let Some(p) = self.rc() {
            shared::spawn("改預計日期", async move { p.move_dates(task, d.preview.0, d.preview.1).await });
        }
    }

    /// 拖拉被中斷（例如視窗失去焦點）：放棄，畫回原位。
    pub(super) fn cancel_drag(&self) {
        if self.st.borrow_mut().schedule.drag.take().is_none() {
            return;
        }
        self.with_data(|x| x.set_drag_key(Default::default()));
        self.rerender_schedule();
    }

    /// 送出新的預計日期（update_task）：先畫在新位置，伺服器的說明（例如執行者送出變更申請）照樣提示；
    /// 同步完照伺服器的資料重畫，失敗就畫回原位。
    async fn move_dates(self: Rc<Self>, t: TaskSummary, start: Option<NaiveDate>, end: Option<NaiveDate>) {
        let (old_start, old_end) = self.st.borrow().schedule.dates_of(&t);
        let mut fields = Map::new();
        for (name, day) in TaskSchedule::changes(old_start, old_end, start, end) {
            fields.insert(name.to_owned(), Value::String(inputs::date_text(Some(day))));
        }
        let new_start = start.or(old_start);
        let new_end = end.or(old_end);
        self.st.borrow_mut().schedule.overrides.insert(t.key.to_uppercase(), (new_start, new_end));
        self.rerender_schedule();
        match self.engine().api().update_task(&t.key, fields).await {
            Ok(message) => {
                log::info!(
                    "{} 拖拉改預計日期：{} → {}",
                    t.key,
                    TaskSchedule::range_text(old_start, old_end),
                    TaskSchedule::range_text(new_start, new_end)
                );
                let notice = message
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| format!("{} 預計 {}", t.key, TaskSchedule::range_text(new_start, new_end)));
                self.engine().show_notice(&notice);
                {
                    let mut st = self.st.borrow_mut();
                    if st.done_tasks.iter().any(|d| d.key.eq_ignore_ascii_case(&t.key)) {
                        st.done_fetched = None; // 已完成的那份不在同步範圍，下次畫的時候重讀
                    }
                }
                self.engine().after_external_change().await;
            }
            Err(e) => {
                log::error!("{} 改預計日期失敗: {}", t.key, e.0);
                self.engine().show_notice(&format!("{} 改預計日期失敗：{}", t.key, e.0));
            }
        }
        self.st.borrow_mut().schedule.overrides.remove(&t.key.to_uppercase());
        self.render_list(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    fn task(category: &str) -> TaskSummary {
        TaskSummary::new("F2P-1", "登入", "F2P", "處理中", category, "", None, None, false, "")
    }

    #[test]
    fn 逾期只看還沒完成的() {
        let today = d(11, 12);
        assert!(is_overdue(&task("IN_PROGRESS"), (Some(d(11, 1)), Some(d(11, 11))), today));
        assert!(!is_overdue(&task("IN_PROGRESS"), (Some(d(11, 1)), Some(today)), today));
        assert!(!is_overdue(&task("DONE"), (None, Some(d(11, 1))), today));
        assert!(!is_overdue(&task("TODO"), (None, None), today));
    }

    #[test]
    fn 任務條提示() {
        assert_eq!(
            "F2P-1 登入\nF2P・處理中\n預計 11/12 – 11/15\n點一下查看；拖拉改預計日期",
            bar_tip(&task("TODO"), (Some(d(11, 12)), Some(d(11, 15))))
        );
    }

    #[test]
    fn 拖完還沒同步的日期先畫在新位置() {
        let mut st = ScheduleState::new(d(11, 12));
        let t = task("TODO");
        assert_eq!((None, None), st.dates_of(&t));
        st.overrides.insert("F2P-1".into(), (Some(d(11, 3)), Some(d(11, 4))));
        assert_eq!((Some(d(11, 3)), Some(d(11, 4))), st.dates_of(&t));
        assert_eq!(d(11, 1), st.calendar_month);
    }
}
