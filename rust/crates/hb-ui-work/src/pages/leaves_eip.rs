//! 請假頁的「EIP 假單」分頁（對應 C# `UI/Work/EipLeavesPage.xaml(.cs)`）：直接讀 EIP 的假單，可以切換月份，用來確認同步讀到的資料。

use std::rc::{Rc, Weak};

use chrono::{Datelike, Local, Months, NaiveDate};
use hb_core::{CoreError, EipLeaveRecord};

use super::{Page, bind, day_text, first_of_month};
use crate::pages::meetings::w3_common::{model, spawn};
use crate::{EipRow, LeavesPageData};

pub(super) fn bind(page: &Rc<Page>, g: &LeavesPageData<'_>) {
    let page = page.clone();
    bind!(g.on_eip_prev, page, || page.move_month(-1));
    bind!(g.on_eip_next, page, || page.move_month(1));
    bind!(g.on_eip_this_month, page, || {
        page.st.borrow_mut().eip_month = first_of_month(Local::now().date_naive());
        spawn(async move { page.load_eip(false).await });
    });
    bind!(g.on_eip_reload, page, || spawn(async move { page.load_eip(true).await }));
}

impl Page {
    fn move_month(self: &Rc<Self>, months: i32) {
        {
            let mut st = self.st.borrow_mut();
            let m = st.eip_month;
            st.eip_month = if months < 0 {
                m.checked_sub_months(Months::new(months.unsigned_abs())).unwrap_or(m)
            } else {
                m.checked_add_months(Months::new(months as u32)).unwrap_or(m)
            };
        }
        let page = self.clone();
        spawn(async move { page.load_eip(false).await });
    }

    pub(super) async fn load_eip(self: Rc<Self>, reload: bool) {
        let (version, month) = {
            let mut st = self.st.borrow_mut();
            st.eip_version += 1;
            (st.eip_version, st.eip_month)
        };
        let this_month = first_of_month(Local::now().date_naive());
        self.with(|g| {
            g.set_eip_month(format!("{} 年 {} 月", month.year(), month.month()).into());
            g.set_eip_this_month_visible(month != this_month);
            g.set_eip_reload_enabled(false);
            g.set_eip_status("".into());
            g.set_eip_empty("讀取中…".into());
            g.set_eip_rows(model(Vec::new()));
            g.set_eip_summary("".into());
        });
        let engine = self.ctx.engine.clone();
        let result = engine.load_eip_year(month.year(), reload).await;
        match result {
            Ok(year) => {
                if version != self.st.borrow().eip_version {
                    return;
                }
                let records = month_records(&year, month);
                let rows: Vec<EipRow> = records.iter().map(|r| eip_row(r, engine.is_eip_synced(&r.leave.no))).collect();
                let count = rows.len();
                self.with(|g| {
                    g.set_eip_rows(model(rows));
                    g.set_eip_empty(if count == 0 { "這個月 EIP 上沒有你的假單" } else { "" }.into());
                    g.set_eip_summary(format!("這個月 {count} 筆 · {} 年共 {} 筆", month.year(), year.len()).into());
                });
            }
            Err(e) => {
                log::error!("讀取 EIP 假單失敗: {}", e.message());
                if version != self.st.borrow().eip_version {
                    return;
                }
                let text = match &e {
                    CoreError::EipLogin(m) => m.clone(),
                    other => format!("讀取失敗：{}", other.message()),
                };
                self.with(|g| {
                    g.set_eip_empty("".into());
                    g.set_eip_status(text.into());
                });
            }
        }
        if version == self.st.borrow().eip_version {
            self.with(|g| g.set_eip_reload_enabled(true));
        }
    }
}

/// 跟這個月有重疊的假單，照開始時間排（穩定排序）。
pub(crate) fn month_records(year: &[EipLeaveRecord], month: NaiveDate) -> Vec<&EipLeaveRecord> {
    let from = month.and_hms_opt(0, 0, 0).unwrap_or_default();
    let to = month.checked_add_months(Months::new(1)).unwrap_or(month).and_hms_opt(0, 0, 0).unwrap_or_default();
    let mut rows: Vec<&EipLeaveRecord> = year.iter().filter(|r| r.leave.start < to && r.leave.end > from).collect();
    rows.sort_by_key(|r| r.leave.start);
    rows
}

/// C# `EipLeaveRowVm`。
pub(crate) fn eip_row(r: &EipLeaveRecord, synced: bool) -> EipRow {
    let l = &r.leave;
    // 特休、事假…；不是一般請假（銷假、補假）時加註
    let title = format!(
        "{}{}",
        if l.r#type.is_empty() { &l.kind } else { &l.r#type },
        if l.kind == "請假" || l.kind.is_empty() { String::new() } else { format!("（{}）", l.kind) }
    );
    let start = format!("{}{}", day_text(l.start.date()), l.start.format("%H:%M"));
    let end = if l.end.date() == l.start.date() {
        l.end.format("%H:%M").to_string()
    } else {
        format!("{}{}", day_text(l.end.date()), l.end.format("%H:%M"))
    };
    let period =
        format!("{start} – {end}{}", if l.subtotal.is_empty() { String::new() } else { format!("　{}", l.subtotal) });
    let detail = [format!("單號 {}", l.no), l.reason.clone()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    EipRow {
        title: title.into(),
        period: period.into(),
        detail: detail.into(),
        state: r.state.as_str().into(),
        state_tone: match r.state.as_str() {
            "核准" => 1,
            "駁回" => 2,
            _ => 0,
        },
        synced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_eip::EipLeave;

    fn record(
        no: &str,
        kind: &str,
        ty: &str,
        start: (u32, u32, u32),
        end: (u32, u32, u32),
        state: &str,
    ) -> EipLeaveRecord {
        let at =
            |(m, d, h): (u32, u32, u32)| NaiveDate::from_ymd_opt(2026, m, d).unwrap().and_hms_opt(h, 0, 0).unwrap();
        EipLeaveRecord {
            leave: EipLeave {
                no: no.into(),
                person_no: "A1".into(),
                kind: kind.into(),
                r#type: ty.into(),
                reason: "".into(),
                start: at(start),
                end: at(end),
                status: "".into(),
                id: None,
                subtotal: "".into(),
            },
            state: state.into(),
        }
    }

    #[test]
    fn eip_row_標題與期間() {
        // 2026/11/5 是週四
        let r = record("L1", "請假", "特休", (11, 5, 9), (11, 5, 18), "核准");
        let row = eip_row(&r, true);
        assert_eq!("特休", row.title.as_str());
        assert_eq!("11/5（週四）09:00 – 18:00", row.period.as_str());
        assert_eq!("單號 L1", row.detail.as_str());
        assert_eq!(1, row.state_tone);
        let r = record("L2", "銷假", "", (11, 5, 9), (11, 6, 18), "駁回");
        let row = eip_row(&r, false);
        assert_eq!("銷假（銷假）", row.title.as_str());
        assert_eq!("11/5（週四）09:00 – 11/6（週五）18:00", row.period.as_str());
        assert_eq!(2, row.state_tone);
    }

    #[test]
    fn month_records_只列重疊的() {
        let year = vec![
            record("a", "請假", "", (11, 20, 9), (11, 20, 18), "待審"),
            record("b", "請假", "", (10, 31, 9), (11, 1, 18), "待審"),
            record("c", "請假", "", (12, 1, 9), (12, 1, 18), "待審"),
        ];
        let month = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let nos: Vec<_> = month_records(&year, month).iter().map(|r| r.leave.no.clone()).collect();
        assert_eq!(vec!["b", "a"], nos);
    }
}
