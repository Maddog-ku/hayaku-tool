//! 對應 C# `UI/Work/TasksPage.Gantt.cs`：我的任務的甘特圖：左邊任務名稱（照目前的排序），右邊日期軸（週末底色、今天一條線，
//! 天數照寬度）。拖整條平移、拖兩端改預計開始或結束；沒有預計日期的任務在自己那列拉出一段就是設定起訖。
//! 不在畫面範圍的任務在邊上標日期，點了跳過去。

use chrono::{Datelike, NaiveDate, TimeDelta};
use hb_api::TaskSummary;
use hb_core::{Drag, TaskSchedule};
use hb_ui_common::TaskText;
use slint::Model;

use super::Page;
use super::schedule::{DragState, is_done, is_overdue};
use crate::shared::vec_model;
use crate::{TasksPageData, TkGanttDay, TkGanttRow};

pub(super) const NAME_WIDTH: f32 = 240.0;
pub(super) const MIN_DAY_WIDTH: f32 = 26.0;
/// 右邊捲軸
const SCROLLBAR: f32 = 22.0;
const WEEKDAY_NAMES: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];

/// 甘特圖預設從上週的星期一開始。
pub(super) fn default_start(today: NaiveDate) -> NaiveDate {
    TaskSchedule::week_start(today) - TimeDelta::days(7)
}

/// 照寬度算天數與每天多寬（C# RenderGantt）：天數 7～120；還沒量到寬度用 42 天、每天 26。
pub(super) fn days_for_width(view_width: f32) -> (i32, f32) {
    let width = view_width - NAME_WIDTH - SCROLLBAR;
    if width > 0.0 {
        let days = ((width / MIN_DAY_WIDTH) as i32).clamp(7, 120);
        (days, width / days as f32)
    } else {
        (42, MIN_DAY_WIDTH)
    }
}

/// 日期標題的一格：月份（最左邊或每月 1 號）、日期、星期。
pub(super) fn header_day(start: NaiveDate, i: i32, today: NaiveDate) -> TkGanttDay {
    let day = start + TimeDelta::days(i as i64);
    let dow = day.weekday().num_days_from_monday() as usize;
    TkGanttDay {
        day: day.day().to_string().into(),
        weekday: WEEKDAY_NAMES[dow].into(),
        month: if i == 0 || day.day() == 1 {
            format!("{}/{}", day.year(), day.month()).into()
        } else {
            Default::default()
        },
        weekend: dow >= 5,
        today: day == today,
    }
}

/// 一列在日期軸上的樣子：Some((開始格, 結束格)) 有條；Err(before) 不在範圍（true＝在前面）；None 沒有日期。
pub(super) fn row_span(
    dates: (Option<NaiveDate>, Option<NaiveDate>),
    start: NaiveDate,
    days: i32,
) -> Option<Result<(i32, i32), bool>> {
    let (s, e) = TaskSchedule::span(dates.0, dates.1)?;
    let last = start + TimeDelta::days(days as i64 - 1);
    if e < start || s > last {
        return Some(Err(e < start));
    }
    Some(Ok(((s - start).num_days() as i32, (e - start).num_days() as i32)))
}

impl Page {
    pub(super) fn render_gantt(&self) {
        let today = Self::today();
        let (width, start, tasks) = {
            let st = self.st.borrow();
            (st.schedule.gantt_width, st.schedule.gantt_start, st.schedule.tasks.clone())
        };
        let (days, day_width) = days_for_width(width);
        let end = start + TimeDelta::days(days as i64 - 1);
        let header: Vec<TkGanttDay> = (0..days).map(|i| header_day(start, i, today)).collect();
        let today_index = (today - start).num_days() as i32;
        let rows: Vec<TkGanttRow> = tasks.iter().map(|t| self.gantt_row(t, start, days, today)).collect();
        {
            let mut st = self.st.borrow_mut();
            st.schedule.gantt_days = days;
            st.schedule.gantt_day_width = day_width;
            st.schedule.gantt_rows = tasks;
        }
        self.with_data(|d| {
            d.set_schedule_title(format!("{} – {}", TaskSchedule::md(start), TaskSchedule::md(end)).into());
            d.set_schedule_prev_tip("往前一週".into());
            d.set_schedule_next_tip("往後一週".into());
            d.set_schedule_hint("拖整條平移，拖兩端改開始／結束；沒有日期的在那列拉出一段".into());
            d.set_gantt_days(days);
            d.set_gantt_day_width(day_width);
            d.set_gantt_today(if (0..days).contains(&today_index) { today_index } else { -1 });
            d.set_gantt_header(vec_model(header));
            d.set_gantt_rows(vec_model(rows));
        });
    }

    /// 一列：任務名稱＋日期軸上的條。
    fn gantt_row(&self, t: &TaskSummary, start: NaiveDate, days: i32, today: NaiveDate) -> TkGanttRow {
        let dates = self.st.borrow().schedule.dates_of(t);
        let mut row = TkGanttRow {
            key: t.key.as_str().into(),
            title: TaskText::title_of(t).into(),
            name_tip: format!(
                "{} {}\n{}・{}",
                t.key,
                TaskText::title_of(t),
                if t.path.is_empty() { t.project.clone() } else { format!("{} › {}", t.project, t.path) },
                t.status
            )
            .into(),
            done: is_done(t),
            ..Default::default()
        };
        match row_span(dates, start, days) {
            None => {
                row.kind = 2;
                row.bar = self.schedule_bar(t, false, false, false);
            }
            Some(Err(before)) => {
                // 不在畫面範圍：在邊上標日期，點了跳到那裡
                let range = TaskSchedule::range_text(dates.0, dates.1);
                row.kind = 1;
                row.jump_before = before;
                row.jump_text = if before { format!("‹ {range}") } else { format!("{range} ›") }.into();
                row.jump_overdue = is_overdue(t, dates, today);
            }
            Some(Ok((s, e))) => {
                row.kind = 0;
                row.start = s;
                row.end = e;
                row.bar = self.schedule_bar(t, s < 0, e >= days, false);
            }
        }
        row
    }

    pub(super) fn shift_gantt(&self, dir: i32) {
        {
            let mut st = self.st.borrow_mut();
            st.schedule.gantt_start = if dir == 0 {
                default_start(Self::today())
            } else {
                st.schedule.gantt_start + TimeDelta::days(7 * dir as i64)
            };
        }
        self.render_gantt();
    }

    /// 寬度變了（超過 4px）才重畫（天數跟著寬度）。
    pub(super) fn gantt_resized(&self, width: f32) {
        let changed = {
            let mut st = self.st.borrow_mut();
            let changed = (width - st.schedule.gantt_width).abs() > 4.0;
            if changed {
                st.schedule.gantt_width = width;
            }
            changed && st.schedule.view == Some("gantt") && st.schedule.drag.is_none()
        };
        if changed {
            self.render_gantt();
        }
    }

    /// 點了邊上的日期：跳到那個任務的日期（前一週開始）。
    pub(super) fn gantt_jump(&self, row: usize) {
        {
            let mut st = self.st.borrow_mut();
            let Some(t) = st.schedule.gantt_rows.get(row).cloned() else { return };
            let dates = st.schedule.dates_of(&t);
            let Some((s, _)) = TaskSchedule::span(dates.0, dates.1) else { return };
            st.schedule.gantt_start = default_start(s);
        }
        self.render_gantt();
    }

    /// 拖條（移動超過門檻才會叫到這裡）：mode 0 整條、1 開始那端、2 結束那端；delta 是拖了幾天。
    pub(super) fn gantt_drag(&self, row: usize, mode: i32, delta: i32) {
        let mode = match mode {
            1 => Drag::Start,
            2 => Drag::End,
            _ => Drag::Move,
        };
        let (t, preview) = {
            let mut st = self.st.borrow_mut();
            let Some(t) = st.schedule.gantt_rows.get(row).cloned() else { return };
            let dates = st.schedule.dates_of(&t);
            let preview = TaskSchedule::apply(dates.0, dates.1, mode, delta);
            match st.schedule.drag.as_mut().filter(|d| d.key.eq_ignore_ascii_case(&t.key)) {
                Some(d) => d.preview = preview,
                None => {
                    st.schedule.drag =
                        Some(DragState { key: t.key.clone(), grab_day: None, undated: false, started: true, preview })
                }
            }
            (t, preview)
        };
        if TaskSchedule::span(preview.0, preview.1).is_some() {
            self.update_gantt_row(row, preview, false);
            self.show_gantt_status(&t, preview);
        }
        self.with_data(|d| d.set_drag_key(t.key.as_str().into()));
    }

    /// 沒有預計日期的任務：在自己那列按住拉出一段（只點一下就是那一天）。
    pub(super) fn gantt_draw_start(&self, row: usize, day: i32) {
        let (t, preview) = {
            let mut st = self.st.borrow_mut();
            let Some(t) = st.schedule.gantt_rows.get(row).cloned() else { return };
            let day = st.schedule.gantt_start + TimeDelta::days(day as i64);
            let preview = (Some(day), Some(day));
            // 點一下也算設定那一天，不是打開任務
            st.schedule.drag =
                Some(DragState { key: t.key.clone(), grab_day: Some(day), undated: true, started: true, preview });
            (t, preview)
        };
        self.update_gantt_row(row, preview, true);
        self.show_gantt_status(&t, preview);
    }

    pub(super) fn gantt_draw_move(&self, row: usize, day: i32) {
        let (t, preview) = {
            let mut st = self.st.borrow_mut();
            let Some(t) = st.schedule.gantt_rows.get(row).cloned() else { return };
            let day = st.schedule.gantt_start + TimeDelta::days(day as i64);
            let Some(d) = st.schedule.drag.as_mut().filter(|d| d.key.eq_ignore_ascii_case(&t.key)) else { return };
            let Some(grab) = d.grab_day else { return };
            let (s, e) = TaskSchedule::place(grab, day);
            d.preview = (Some(s), Some(e));
            (t, d.preview)
        };
        self.update_gantt_row(row, preview, true);
        self.show_gantt_status(&t, preview);
    }

    /// 放開：沒拖動算點一下（打開任務），拖了就改日期。
    pub(super) fn gantt_release(&self, row: usize) {
        let key = self.st.borrow().schedule.gantt_rows.get(row).map(|t| t.key.clone());
        let Some(key) = key else { return };
        if self.st.borrow().schedule.drag.as_ref().is_some_and(|d| d.key.eq_ignore_ascii_case(&key)) {
            self.end_drag();
        } else {
            self.open_key(&key);
        }
    }

    /// 拖拉中：那一列的條畫在新位置（沒有日期的列畫出拉出的那一段）。
    fn update_gantt_row(&self, row: usize, preview: (Option<NaiveDate>, Option<NaiveDate>), drawing: bool) {
        let Some((s, e)) = TaskSchedule::span(preview.0, preview.1) else { return };
        let (start, days) = {
            let st = self.st.borrow();
            (st.schedule.gantt_start, st.schedule.gantt_days)
        };
        let (si, ei) = ((s - start).num_days() as i32, (e - start).num_days() as i32);
        self.with_data(|d: &TasksPageData| {
            let model = d.get_gantt_rows();
            if let Some(mut r) = model.row_data(row) {
                r.start = si;
                r.end = ei;
                r.bar.from_before = si < 0;
                r.bar.to_after = ei >= days;
                if drawing {
                    r.preview = true;
                }
                model.set_row_data(row, r);
            }
        });
    }

    /// 拖拉中把新的日期寫在標題（C# ShowGanttStatus）。
    fn show_gantt_status(&self, t: &TaskSummary, range: (Option<NaiveDate>, Option<NaiveDate>)) {
        let text = format!("{}：{}", t.key, TaskSchedule::range_text(range.0, range.1));
        self.with_data(|d| d.set_schedule_title(text.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn 預設從上週一開始() {
        // 2026/11/12 是星期四 → 那週一 11/9，再往前一週 11/2
        assert_eq!(d(11, 2), default_start(d(11, 12)));
    }

    #[test]
    fn 天數跟著寬度() {
        // 名稱 240＋捲軸 22，剩 780 → 30 天、每天 26
        assert_eq!((30, 26.0), days_for_width(240.0 + 22.0 + 780.0));
        // 太窄至少 7 天（每天變窄）
        let (days, w) = days_for_width(240.0 + 22.0 + 140.0);
        assert_eq!(7, days);
        assert!((w - 20.0).abs() < 0.01);
        // 還沒量到寬度
        assert_eq!((42, 26.0), days_for_width(0.0));
        // 很寬最多 120 天
        assert_eq!(120, days_for_width(10000.0).0);
    }

    #[test]
    fn 日期標題_月份只在最左邊與1號() {
        let start = d(10, 30);
        let first = header_day(start, 0, d(11, 2));
        assert_eq!(
            ("30", "五", "2026/10", false),
            (first.day.as_str(), first.weekday.as_str(), first.month.as_str(), first.weekend)
        );
        let sat = header_day(start, 1, d(11, 2));
        assert_eq!(("", true), (sat.month.as_str(), sat.weekend));
        let nov1 = header_day(start, 2, d(11, 2));
        assert_eq!("2026/11", nov1.month.as_str());
        assert!(header_day(start, 3, d(11, 2)).today);
    }

    #[test]
    fn 條的位置與範圍外() {
        let start = d(11, 2);
        assert_eq!(None, row_span((None, None), start, 14));
        assert_eq!(Some(Ok((1, 3))), row_span((Some(d(11, 3)), Some(d(11, 5))), start, 14));
        // 跨出左邊：開始格是負的（條被裁掉）
        assert_eq!(Some(Ok((-2, 0))), row_span((Some(d(10, 31)), Some(d(11, 2))), start, 14));
        assert_eq!(Some(Err(true)), row_span((Some(d(10, 1)), Some(d(10, 5))), start, 14));
        assert_eq!(Some(Err(false)), row_span((None, Some(d(11, 16))), start, 14));
    }
}
