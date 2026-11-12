//! 對應 C# `UI/Work/TasksPage.Calendar.cs`：我的任務的日曆（月曆，星期一開頭）：任務從預計開始畫到預計結束，
//! 拖到別天起訖一起平移（天數不變）；沒有預計日期的列在右邊，拖到日期上就把起訖都設成那天。
//! 放不下的那天顯示「+N」，點了選要打開哪個。

use chrono::{Datelike, Months, NaiveDate, TimeDelta};
use hb_core::{Drag, Segment, TaskSchedule};
use hb_ui_common::TaskText;

use super::Page;
use super::schedule::DragState;
use crate::shared::{self, vec_model};
use crate::{TkCalDay, TkMore, TkPick};

pub(super) const DATE_ROW: f32 = 20.0;
pub(super) const LANE_ROW: f32 = 17.0;
pub(super) const MORE_ROW: f32 = 14.0;
/// 日曆上方的工具列與星期標題（calendar.slint：工具列 40、星期 18＋間距 4）。
const CHROME_HEIGHT: f32 = 40.0 + 22.0;

/// 每週能放幾列任務：看日曆實際高度（C# RenderCalendar）。
pub(super) fn lanes_for(height: f32, weeks: i32) -> i32 {
    let available = height - CHROME_HEIGHT;
    let week_height = if available > 0.0 { available / weeks.max(1) as f32 } else { 110.0 };
    (((week_height - DATE_ROW - MORE_ROW) / LANE_ROW).floor() as i32).max(1)
}

/// 日曆一格的標題：1 號寫「M/d」，其他只寫日。
pub(super) fn day_label(day: NaiveDate) -> String {
    if day.day() == 1 { format!("{}/{}", day.month(), day.day()) } else { day.day().to_string() }
}

/// 一個月的排版：每週的任務條（放得下的）、每天放不下的（「+N」）、沒有日期的。
/// items：(index, 開始, 結束)；回傳的都是 items 的 index。
pub(super) struct MonthLayout {
    /// (第幾週, 那一條)
    pub bars: Vec<(i32, Segment<usize>)>,
    /// (第幾週, 星期幾, 放不下的)
    pub more: Vec<(i32, i32, Vec<usize>)>,
}

pub(super) fn layout_month(
    items: &[(usize, NaiveDate, NaiveDate)],
    first: NaiveDate,
    weeks: i32,
    lanes: i32,
) -> MonthLayout {
    let mut bars = Vec::new();
    let mut more = Vec::new();
    for w in 0..weeks {
        let week_start = first + TimeDelta::days(w as i64 * 7);
        let mut hidden: [Vec<usize>; 7] = Default::default();
        for s in TaskSchedule::week_lanes(items.iter().copied(), week_start) {
            if s.lane >= lanes {
                for c in s.column..s.column + s.days {
                    hidden[c as usize].push(s.item);
                }
                continue;
            }
            bars.push((w, s));
        }
        for (c, list) in hidden.into_iter().enumerate() {
            if !list.is_empty() {
                more.push((w, c as i32, list));
            }
        }
    }
    MonthLayout { bars, more }
}

impl Page {
    pub(super) fn render_calendar(&self) {
        let today = Self::today();
        let (month, height) = {
            let st = self.st.borrow();
            (st.schedule.calendar_month, st.schedule.calendar_height)
        };
        let (first, weeks) = TaskSchedule::month_grid(month.year(), month.month());
        let tasks = self.st.borrow().schedule.tasks.clone();
        let mut dated: Vec<(usize, NaiveDate, NaiveDate)> = Vec::new();
        let mut undated: Vec<usize> = Vec::new();
        for (i, t) in tasks.iter().enumerate() {
            let (start, end) = self.st.borrow().schedule.dates_of(t);
            match TaskSchedule::span(start, end) {
                Some((s, e)) => dated.push((i, s, e)),
                None => undated.push(i),
            }
        }
        let lanes = lanes_for(height, weeks);
        let layout = layout_month(&dated, first, weeks, lanes);

        let days: Vec<TkCalDay> = (0..weeks * 7)
            .map(|i| {
                let day = first + TimeDelta::days(i as i64);
                TkCalDay {
                    label: day_label(day).into(),
                    in_month: day.month() == month.month(),
                    weekend: i % 7 >= 5,
                    today: day == today,
                }
            })
            .collect();
        let bars: Vec<_> = layout
            .bars
            .iter()
            .map(|(w, s)| {
                let mut bar = self.schedule_bar(&tasks[s.item], s.from_before, s.to_after, true);
                bar.week = *w;
                bar.column = s.column;
                bar.days = s.days;
                bar.lane = s.lane;
                bar
            })
            .collect();
        let more: Vec<TkMore> = layout
            .more
            .iter()
            .map(|(w, c, list)| TkMore {
                week: *w,
                column: *c,
                text: format!("+{}", list.len()).into(),
                tip: list
                    .iter()
                    .map(|i| format!("{} {}", tasks[*i].key, TaskText::title_of(&tasks[*i])))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
            })
            .collect();
        let undated_bars: Vec<_> = undated.iter().map(|i| self.schedule_bar(&tasks[*i], false, false, true)).collect();
        {
            let mut st = self.st.borrow_mut();
            st.schedule.calendar_first = first;
            st.schedule.calendar_weeks = weeks;
            st.schedule.calendar_more = layout
                .more
                .iter()
                .map(|(_, _, list)| {
                    list.iter().map(|i| (tasks[*i].key.clone(), TaskText::title_of(&tasks[*i]))).collect()
                })
                .collect();
        }
        self.with_data(|d| {
            d.set_schedule_title(format!("{} 年 {} 月", month.year(), month.month()).into());
            d.set_schedule_prev_tip("上個月".into());
            d.set_schedule_next_tip("下個月".into());
            d.set_schedule_hint("拖到別天改預計日期（天數不變）".into());
            d.set_cal_weeks(weeks);
            d.set_cal_lanes(lanes);
            d.set_cal_days(vec_model(days));
            d.set_cal_bars(vec_model(bars));
            d.set_cal_more(vec_model(more));
            d.set_cal_undated(vec_model(undated_bars));
            d.set_cal_undated_title(format!("沒有預計日期 {}", undated.len()).into());
            d.set_cal_hl_from(-1);
            d.set_cal_hl_to(-1);
        });
    }

    pub(super) fn shift_month(&self, dir: i32) {
        {
            let mut st = self.st.borrow_mut();
            let m = st.schedule.calendar_month;
            st.schedule.calendar_month = match dir {
                0 => {
                    let t = Self::today();
                    NaiveDate::from_ymd_opt(t.year(), t.month(), 1).unwrap_or(t)
                }
                d if d > 0 => m.checked_add_months(Months::new(d as u32)).unwrap_or(m),
                d => m.checked_sub_months(Months::new(d.unsigned_abs())).unwrap_or(m),
            };
        }
        self.render_calendar();
    }

    /// 日曆高度變了（超過 4px）才重排（每週能放幾列跟著變）。
    pub(super) fn calendar_resized(&self, height: f32) {
        let changed = {
            let mut st = self.st.borrow_mut();
            let changed = (height - st.schedule.calendar_height).abs() > 4.0;
            if changed {
                st.schedule.calendar_height = height;
            }
            changed && st.schedule.view == Some("calendar")
        };
        if changed && self.st.borrow().schedule.drag.is_none() {
            self.render_calendar();
        }
    }

    fn calendar_day(&self, index: i32) -> Option<NaiveDate> {
        (index >= 0).then(|| self.st.borrow().schedule.calendar_first + TimeDelta::days(index as i64))
    }

    /// 按下任務條（還沒移動超過幾個像素前算點一下）。
    pub(super) fn calendar_press(&self, key: &str, undated: bool, day: i32) {
        let grab = self.calendar_day(day);
        self.st.borrow_mut().schedule.drag =
            Some(DragState { key: key.to_owned(), grab_day: grab, undated, started: false, preview: (None, None) });
    }

    /// 拖動：滑鼠下面是哪一天；沒有日期的設成那天，有日期的起訖一起平移。
    pub(super) fn calendar_drag(&self, key: &str, day: i32) {
        let day = self.calendar_day(day);
        let preview = {
            let mut st = self.st.borrow_mut();
            let Some(task) = st.schedule.task(key) else { return };
            let dates = st.schedule.dates_of(&task);
            let Some(d) = st.schedule.drag.as_mut().filter(|d| d.key.eq_ignore_ascii_case(key)) else { return };
            d.started = true;
            d.preview = match (day, d.undated, d.grab_day) {
                (None, _, _) => (None, None),
                (Some(day), true, _) => (Some(day), Some(day)),
                (Some(day), false, Some(grab)) => {
                    TaskSchedule::apply(dates.0, dates.1, Drag::Move, (day - grab).num_days() as i32)
                }
                (Some(_), false, None) => d.preview,
            };
            d.preview
        };
        let first = self.st.borrow().schedule.calendar_first;
        let (from, to) = match TaskSchedule::span(preview.0, preview.1) {
            Some((s, e)) => ((s - first).num_days() as i32, (e - first).num_days() as i32),
            None => (-1, -1),
        };
        self.with_data(|d| {
            d.set_drag_key(key.into());
            d.set_cal_hl_from(from);
            d.set_cal_hl_to(to);
        });
    }

    pub(super) fn calendar_release(&self, key: &str, _started: bool) {
        if !self.st.borrow().schedule.drag.as_ref().is_some_and(|d| d.key.eq_ignore_ascii_case(key)) {
            return;
        }
        self.with_data(|d| {
            d.set_cal_hl_from(-1);
            d.set_cal_hl_to(-1);
        });
        self.end_drag();
    }

    /// 「+N」：列出放不下的任務，選了打開。
    pub(super) fn calendar_more(&self, index: usize) {
        let items: Vec<TkPick> = self
            .st
            .borrow()
            .schedule
            .calendar_more
            .get(index)
            .map(|list| {
                list.iter().map(|(k, t)| TkPick { key: k.as_str().into(), text: format!("{k}  {t}").into() }).collect()
            })
            .unwrap_or_default();
        self.with_data(|d| d.set_more_items(shared::vec_model(items)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn 每週能放幾列_看高度() {
        // 6 週、高 662：每週 (662-62)/6 = 100 → (100-20-14)/17 = 3 列
        assert_eq!(3, lanes_for(662.0, 6));
        // 太矮至少 1 列；還沒量到高度用每週 110
        assert_eq!(1, lanes_for(100.0, 6));
        assert_eq!(4, lanes_for(0.0, 5));
    }

    #[test]
    fn 日期格標題() {
        assert_eq!("11/1", day_label(d(11, 1)));
        assert_eq!("12", day_label(d(11, 12)));
    }

    #[test]
    fn 放不下的那天顯示加號() {
        let first = d(11, 9); // 星期一
        let items =
            vec![(0, d(11, 9), d(11, 9)), (1, d(11, 9), d(11, 10)), (2, d(11, 9), d(11, 9)), (3, d(11, 20), d(11, 20))];
        let layout = layout_month(&items, first, 2, 2);
        // 第一週 11/9 有三條，放兩列：一條放不下（最短的那條排最後）
        assert_eq!(3, layout.bars.len());
        assert_eq!(vec![(0, 0, vec![2])], layout.more);
        // 第二週的 11/20 是星期五
        let week2: Vec<_> = layout.bars.iter().filter(|(w, _)| *w == 1).collect();
        assert_eq!(1, week2.len());
        assert_eq!(4, week2[0].1.column);
    }
}
