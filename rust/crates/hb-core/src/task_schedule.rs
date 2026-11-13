//! 對應 C# `Core/TaskSchedule.cs`：我的任務的日曆、甘特圖（Hayaku 1.5.2）：拖拉改預計日期的計算與日曆排版。
//! 純邏輯，畫面在工作台的 TasksPage。
//! 只有預計開始或只有預計結束的任務當作一天；兩個都沒有的不畫在日期上（另外列出來，拖到日期上就設成那天）。

use chrono::{Datelike, Months, NaiveDate, TimeDelta};

pub struct TaskSchedule;

/// 拖整條（起訖一起平移）、拖開始那端、拖結束那端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    Move,
    Start,
    End,
}

/// 日曆一週裡的一條：從第 column 天（0＝星期一）起佔 days 天，排在第 lane 列；from_before／to_after＝前一週、下一週還有。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment<T> {
    pub item: T,
    pub column: i32,
    pub days: i32,
    pub lane: i32,
    pub from_before: bool,
    pub to_after: bool,
}

impl<T> Segment<T> {
    pub fn new(item: T, column: i32, days: i32, lane: i32, from_before: bool, to_after: bool) -> Self {
        Self { item, column, days, lane, from_before, to_after }
    }
}

fn add_days(d: NaiveDate, days: i32) -> NaiveDate {
    d + TimeDelta::days(days as i64)
}

/// `DateOnly.DayNumber`（從 0001-01-01 起的天數）。
fn day_number(d: NaiveDate) -> i32 {
    d.num_days_from_ce() - 1
}

impl TaskSchedule {
    /// 畫出來的範圍；結束早於開始時對調；都沒有回 None。
    pub fn span(start: Option<NaiveDate>, end: Option<NaiveDate>) -> Option<(NaiveDate, NaiveDate)> {
        match (start, end) {
            (Some(s), Some(e)) => Some(if s <= e { (s, e) } else { (e, s) }),
            (Some(s), None) => Some((s, s)),
            (None, Some(e)) => Some((e, e)),
            (None, None) => None,
        }
    }

    /// 拖了 days 天之後的預計開始／結束。平移時沒填的欄位維持沒填；拖開始（結束）那端不會越過另一端。
    /// 只有結束的任務拖開始那端往前，會補上預計開始；兩個都沒有的不變（用 [`TaskSchedule::place`]）。
    pub fn apply(
        start: Option<NaiveDate>,
        end: Option<NaiveDate>,
        mode: Drag,
        days: i32,
    ) -> (Option<NaiveDate>, Option<NaiveDate>) {
        let Some(span) = Self::span(start, end).filter(|_| days != 0) else {
            return (start, end);
        };
        match mode {
            Drag::Move => (start.map(|s| add_days(s, days)), end.map(|e| add_days(e, days))),
            Drag::Start => {
                let mut s = add_days(span.0, days);
                if end.is_some() && s > span.1 {
                    s = span.1;
                }
                if start.is_none() && s == span.0 { (start, end) } else { (Some(s), end) }
            }
            Drag::End => {
                let mut e = add_days(span.1, days);
                if start.is_some() && e < span.0 {
                    e = span.0;
                }
                if end.is_none() && e == span.1 { (start, end) } else { (start, Some(e)) }
            }
        }
    }

    /// 沒有預計日期的任務拖到日期上（或在甘特圖拉出一段）：開始、結束照拖的範圍。
    pub fn place(from: NaiveDate, to: NaiveDate) -> (NaiveDate, NaiveDate) {
        if from <= to { (from, to) } else { (to, from) }
    }

    /// 要送 update_task 的欄位（plannedStart、plannedEnd，只放有變的）；拖拉不會把日期清空。
    pub fn changes(
        old_start: Option<NaiveDate>,
        old_end: Option<NaiveDate>,
        new_start: Option<NaiveDate>,
        new_end: Option<NaiveDate>,
    ) -> Vec<(&'static str, NaiveDate)> {
        let mut fields = Vec::new();
        if let Some(s) = new_start
            && Some(s) != old_start
        {
            fields.push(("plannedStart", s));
        }
        if let Some(e) = new_end
            && Some(e) != old_end
        {
            fields.push(("plannedEnd", e));
        }
        fields
    }

    /// 那一週的星期一（日曆、甘特圖都從星期一開始）。
    pub fn week_start(day: NaiveDate) -> NaiveDate {
        day - TimeDelta::days(day.weekday().num_days_from_monday() as i64)
    }

    /// 月曆要畫的範圍：第一格（1 號那週的星期一）與週數（4～6 週）。
    pub fn month_grid(year: i32, month: u32) -> (NaiveDate, i32) {
        let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
        let start = Self::week_start(first);
        let last = first.checked_add_months(Months::new(1)).map_or(first, |d| d - TimeDelta::days(1));
        (start, (day_number(last) - day_number(start)) / 7 + 1)
    }

    /// 一週裡的任務排成不重疊的列：先開始的在上面，同一天開始的長的在上面，其餘照傳進來的順序。
    pub fn week_lanes<T>(
        items: impl IntoIterator<Item = (T, NaiveDate, NaiveDate)>,
        week_start: NaiveDate,
    ) -> Vec<Segment<T>> {
        let week_end = add_days(week_start, 6);
        let mut list: Vec<(T, NaiveDate, NaiveDate)> =
            items.into_iter().filter(|x| x.1 <= week_end && x.2 >= week_start).collect();
        list.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| b.2.cmp(&a.2)));
        let mut lane_ends: Vec<i32> = Vec::new(); // 每一列最後佔到第幾天
        let mut result = Vec::new();
        for (item, s, e) in list {
            let from = (day_number(s) - day_number(week_start)).max(0);
            let to = (day_number(e) - day_number(week_start)).min(6);
            let lane = match lane_ends.iter().position(|last| *last < from) {
                Some(i) => {
                    lane_ends[i] = to;
                    i
                }
                None => {
                    lane_ends.push(to);
                    lane_ends.len() - 1
                }
            };
            result.push(Segment::new(item, from, to - from + 1, lane as i32, s < week_start, e > week_end));
        }
        result
    }

    /// 「11/12」「11/12 – 11/15」；只有一端時標出另一端未指定。
    pub fn range_text(start: Option<NaiveDate>, end: Option<NaiveDate>) -> String {
        match (start, end) {
            (Some(s), Some(e)) if s == e => Self::md(s),
            (Some(s), Some(e)) => format!("{} – {}", Self::md(s), Self::md(e)),
            (Some(s), None) => format!("{} 開始（結束未指定）", Self::md(s)),
            (None, Some(e)) => format!("{} 結束（開始未指定）", Self::md(e)),
            (None, None) => "沒有預計日期".to_owned(),
        }
    }

    /// 「11/12」（不受地區設定的日期分隔符號影響）。
    pub fn md(d: NaiveDate) -> String {
        format!("{}/{}", d.month(), d.day())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 對應 Hayaku152Tests

    fn d(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    #[test]
    fn 平移_起訖一起移_天數不變() {
        assert_eq!(
            (Some(d(11, 14)), Some(d(11, 17))),
            TaskSchedule::apply(Some(d(11, 12)), Some(d(11, 15)), Drag::Move, 2)
        );
        assert_eq!(
            (Some(d(11, 9)), Some(d(11, 12))),
            TaskSchedule::apply(Some(d(11, 12)), Some(d(11, 15)), Drag::Move, -3)
        );
    }

    #[test]
    fn 平移_沒填的欄位維持沒填() {
        assert_eq!((None, Some(d(11, 16))), TaskSchedule::apply(None, Some(d(11, 15)), Drag::Move, 1));
        assert_eq!((Some(d(11, 13)), None), TaskSchedule::apply(Some(d(11, 12)), None, Drag::Move, 1));
    }

    #[test]
    fn 拖開始那端_不會越過結束() {
        assert_eq!(
            (Some(d(11, 10)), Some(d(11, 15))),
            TaskSchedule::apply(Some(d(11, 12)), Some(d(11, 15)), Drag::Start, -2)
        );
        assert_eq!(
            (Some(d(11, 15)), Some(d(11, 15))),
            TaskSchedule::apply(Some(d(11, 12)), Some(d(11, 15)), Drag::Start, 10)
        );
    }

    #[test]
    fn 拖結束那端_不會早於開始() {
        assert_eq!(
            (Some(d(11, 12)), Some(d(11, 20))),
            TaskSchedule::apply(Some(d(11, 12)), Some(d(11, 15)), Drag::End, 5)
        );
        assert_eq!(
            (Some(d(11, 12)), Some(d(11, 12))),
            TaskSchedule::apply(Some(d(11, 12)), Some(d(11, 15)), Drag::End, -10)
        );
    }

    #[test]
    fn 只有結束的任務_往前拉開始那端會補上開始() {
        assert_eq!((Some(d(11, 13)), Some(d(11, 15))), TaskSchedule::apply(None, Some(d(11, 15)), Drag::Start, -2));
        // 往後拉擋在結束那天，等於沒動，不要平白補一個開始
        assert_eq!((None, Some(d(11, 15))), TaskSchedule::apply(None, Some(d(11, 15)), Drag::Start, 3));
    }

    #[test]
    fn 沒有日期的任務拖拉不變_放到日期上才設定() {
        assert_eq!((None, None), TaskSchedule::apply(None, None, Drag::Move, 3));
        assert_eq!((d(11, 10), d(11, 14)), TaskSchedule::place(d(11, 14), d(11, 10)));
    }

    #[test]
    fn 送出的欄位只放有變的() {
        let fields = TaskSchedule::changes(Some(d(11, 12)), Some(d(11, 15)), Some(d(11, 12)), Some(d(11, 18)));
        assert_eq!(vec!["plannedEnd"], fields.iter().map(|f| f.0).collect::<Vec<_>>());
        assert_eq!(d(11, 18), fields[0].1);
        assert!(TaskSchedule::changes(Some(d(11, 12)), Some(d(11, 15)), Some(d(11, 12)), Some(d(11, 15))).is_empty());
        assert_eq!(
            vec!["plannedStart", "plannedEnd"],
            TaskSchedule::changes(None, None, Some(d(11, 3)), Some(d(11, 3))).iter().map(|f| f.0).collect::<Vec<_>>()
        );
    }

    #[test]
    fn 月曆從1號那週的星期一開始() {
        // 2026/11/1 是星期日 → 從 10/26（星期一）開始，到 11/30（星期一）那週，共 6 週
        assert_eq!((d(10, 26), 6), TaskSchedule::month_grid(2026, 11));
        // 2027/2/1 是星期一，剛好 4 週
        assert_eq!((NaiveDate::from_ymd_opt(2027, 2, 1).unwrap(), 4), TaskSchedule::month_grid(2027, 2));
    }

    #[test]
    fn 一週排列_不重疊的放同一列_跨週的標出來() {
        let week = d(11, 9); // 星期一
        let lanes = TaskSchedule::week_lanes(
            vec![
                ("A", d(11, 9), d(11, 10)),
                ("B", d(11, 11), d(11, 11)), // 跟 A 不重疊 → 第 0 列
                ("C", d(11, 10), d(11, 12)), // 跟 A、B 都重疊 → 第 1 列
                ("D", d(11, 5), d(11, 20)),  // 前後週都還有，最早開始 → 第 0 列
                ("E", d(11, 30), d(11, 30)), // 不在這週
            ],
            week,
        );
        let by_item = |item: &str| lanes.iter().find(|s| s.item == item).cloned();
        assert!(by_item("E").is_none());
        assert_eq!(Some(Segment::new("D", 0, 7, 0, true, true)), by_item("D"));
        assert_eq!(1, by_item("A").unwrap().lane);
        assert_eq!(1, by_item("B").unwrap().lane);
        assert_eq!(2, by_item("C").unwrap().lane);
        let c = by_item("C").unwrap();
        assert_eq!((1, 3), (c.column, c.days));
    }

    #[test]
    fn 只有一端的任務當作一天() {
        assert_eq!(Some((d(11, 5), d(11, 5))), TaskSchedule::span(None, Some(d(11, 5))));
        assert_eq!(Some((d(11, 5), d(11, 5))), TaskSchedule::span(Some(d(11, 5)), None));
        assert_eq!(None, TaskSchedule::span(None, None));
        assert_eq!("11/5 結束（開始未指定）", TaskSchedule::range_text(None, Some(d(11, 5))));
    }
}
