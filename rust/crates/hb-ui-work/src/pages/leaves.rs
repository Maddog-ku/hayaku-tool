//! 工作台「請假」頁（對應 C# `UI/Work/LeavesPage.xaml(.cs)`；「EIP 假單」分頁在 `leaves_eip.rs`，對應 `EipLeavesPage.xaml(.cs)`）。
//! 請假與出差：列出期間內誰請假、出差，登記自己的請假或出差、取消。出差照常算上班時數。
//! 右上切到「EIP 假單」看公司 EIP 上自己的假單（切過去才讀）。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

#[path = "leaves_eip.rs"]
mod eip;

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use chrono::{Datelike, Duration, Local, Months, NaiveDate};
use hb_api::{DateTimeOffset, LeaveItem};
use hb_ui_common::date_range;
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use serde_json::{Map, json};
use slint::{ComponentHandle, Model};

use super::meetings::w3_common::{model, spawn, status_parts};
use crate::{LeaveRow, LeavesPageData, PageArg, WorkCtx};

/// 登記的時段（C# Span 的 Tag）。
const SPANS: [&str; 4] = ["DAY", "HALF_AM", "HALF_PM", "RANGE"];
/// 清單的期間（C# LeaveRange 的 Tag）。
const RANGES: [&str; 3] = ["2w", "month", "next"];

pub(crate) struct Page {
    ctx: WorkCtx,
    st: RefCell<State>,
}

struct State {
    trip_task: Option<String>,
    items: Vec<LeaveItem>,
    version: u64,
    eip_month: NaiveDate,
    eip_version: u64,
    confirm: Option<Box<dyn FnOnce()>>,
}

thread_local! {
    static PAGE: RefCell<Option<Rc<Page>>> = const { RefCell::new(None) };
}

macro_rules! bind {
    // 沒有參數的回呼：rustfmt 會把 `| |` 排成 `||`，兩種都接受
    ($g:ident . $on:ident, $page:ident, || $body:expr) => {
        bind!($g.$on, $page, | | $body)
    };
    ($g:ident . $on:ident, $page:ident, | $($arg:ident),* | $body:expr) => {{
        let weak: Weak<Page> = Rc::downgrade(&$page);
        $g.$on(move |$($arg),*| {
            if let Some($page) = weak.upgrade() {
                $body
            }
        });
    }};
}
pub(crate) use bind;

pub fn init(ctx: &WorkCtx) {
    let today = Local::now().date_naive();
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        st: RefCell::new(State {
            trip_task: None,
            items: Vec::new(),
            version: 0,
            eip_month: first_of_month(today),
            eip_version: 0,
            confirm: None,
        }),
    });
    PAGE.with(|p| *p.borrow_mut() = Some(page.clone()));
    let Some(w) = ctx.window() else { return };
    let g = w.global::<LeavesPageData>();
    let tomorrow = today + Duration::days(1);
    g.set_today(date_range::today_int());
    g.set_start_date(date_range::to_int(Some(tomorrow)));
    g.set_end_date(date_range::to_int(Some(tomorrow)));
    bind!(g.on_view_selected, page, |_i| spawn(async move { page.view_changed().await }));
    bind!(g.on_kind_selected, page, |i| page.kind_changed(i == 1));
    bind!(g.on_span_selected, page, |i| page.span_changed(i));
    bind!(g.on_pick_trip_task, page, || spawn(async move { page.pick_trip_task().await }));
    bind!(g.on_add, page, || page.add());
    bind!(g.on_range_selected, page, |_i| spawn(async move { page.load().await }));
    bind!(g.on_mine_toggled, page, |_c| spawn(async move { page.load().await }));
    bind!(g.on_cancel, page, |i| page.cancel(i as usize));
    bind!(g.on_confirm_ok, page, || {
        let action = page.st.borrow_mut().confirm.take();
        page.with(|g| g.set_confirm_open(false));
        if let Some(action) = action {
            action();
        }
    });
    bind!(g.on_confirm_cancel, page, || {
        page.st.borrow_mut().confirm = None;
        page.with(|g| g.set_confirm_open(false));
    });
    eip::bind(&page, &g);
}

/// arg：「eip」切到 EIP 假單、「hayaku」切回 Hayaku；沒帶就留在目前的分頁。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    let view = arg.as_ref().and_then(|a| a.as_text()).map(str::to_owned);
    let showing_eip = page.with(|g| g.get_view() == 1).unwrap_or(false);
    match view.as_deref() {
        Some("eip") if !showing_eip => {
            page.with(|g| g.set_view(1));
            page.view_changed().await;
        }
        Some("hayaku") if showing_eip => {
            page.with(|g| g.set_view(0));
            page.view_changed().await;
        }
        _ if showing_eip => page.load_eip(false).await,
        _ => page.load().await,
    }
}

impl Page {
    pub(crate) fn with<R>(&self, f: impl FnOnce(&LeavesPageData<'_>) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let g = w.global::<LeavesPageData>();
        Some(f(&g))
    }

    fn show_form(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_form_status(text);
            g.set_form_tone(tone);
        });
    }

    fn show_list(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_list_status(text);
            g.set_list_tone(tone);
        });
    }

    async fn view_changed(self: Rc<Self>) {
        if self.with(|g| g.get_view() == 1).unwrap_or(false) {
            self.load_eip(false).await;
        } else {
            self.load().await;
        }
    }

    pub(crate) async fn load(self: Rc<Self>) {
        let version = {
            let mut st = self.st.borrow_mut();
            st.version += 1;
            st.version
        };
        self.with(|g| g.set_empty_text("讀取中…".into()));
        self.show_list(None);
        let Some((range, mine_only)) = self.with(|g| (g.get_range(), g.get_mine_only())) else { return };
        let (from, to) = list_range(RANGES[range.clamp(0, 2) as usize], Local::now().date_naive());
        let result = self.ctx.engine.api().list_leaves(from, to, None, None).await;
        match result {
            Ok(items) => {
                if version != self.st.borrow().version {
                    return;
                }
                let engine = &self.ctx.engine;
                let me = engine.user().map(|u| u.username);
                let now = Local::now().fixed_offset();
                let items: Vec<LeaveItem> =
                    items.into_iter().filter(|l| !mine_only || Some(&l.user.username) == me.as_ref()).collect();
                let rows: Vec<LeaveRow> = items
                    .iter()
                    .map(|l| {
                        let from_eip = engine.eip_sync().is_synced(&l.id);
                        leave_row(l, Some(&l.user.username) == me.as_ref(), from_eip, &now)
                    })
                    .collect();
                let empty = rows.is_empty();
                self.st.borrow_mut().items = items;
                let text = if !empty {
                    ""
                } else if mine_only {
                    "這段期間你沒有請假或出差"
                } else {
                    "這段期間沒有人請假或出差"
                };
                self.with(|g| {
                    g.set_rows(model(rows));
                    g.set_empty_text(text.into());
                });
            }
            Err(e) => {
                log::error!("讀取請假失敗: {e}");
                if version == self.st.borrow().version {
                    self.with(|g| g.set_empty_text("".into()));
                    self.show_list(Some(StatusMessage::error(format!("讀取失敗：{e}"))));
                }
            }
        }
    }

    fn kind_changed(&self, trip: bool) {
        self.with(|g| {
            g.set_form_title(if trip { "登記出差" } else { "登記請假" }.into());
            g.set_form_desc(
                if trip {
                    "只登記、不簽核；出差照常算上班時數、不扣應出勤，時段過了自動寫工時到選的任務"
                } else {
                    "只登記、不簽核；登記後當天不用上班，自動上下班也會跳過"
                }
                .into(),
            );
        });
    }

    fn span_changed(&self, span: i32) {
        let day = span == 0;
        self.with(|g| {
            g.set_single(!day); // 整天可以請一段；半天、指定時段只有一天
            g.set_date_label(if day { "第一天 ～ 最後一天（只請一天就點同一天）" } else { "日期" }.into());
        });
    }

    /// 出差的工時寫到哪個任務（add_leave 的 task，出差必填）。
    async fn pick_trip_task(self: Rc<Self>) {
        let tasks = self.ctx.engine.my_tasks();
        let Some(keys) =
            hb_ui_common::task_picker::pick(&tasks, "出差的工時要寫到哪個任務？（也可以直接輸入編號）", false).await
        else {
            return;
        };
        let Some(key) = keys.into_iter().next() else { return };
        let title = tasks.iter().find(|t| t.key.eq_ignore_ascii_case(&key)).map(|t| t.title.clone());
        let text = match title {
            None => key.clone(),
            Some(t) => format!("{key} {t}"),
        };
        self.st.borrow_mut().trip_task = Some(key);
        self.with(|g| {
            g.set_trip_task_text(text.into());
            g.set_trip_task_set(true);
        });
    }

    fn add(self: &Rc<Self>) {
        let Some((start, end, span, start_text, end_text, note, trip)) = self.with(|g| {
            (
                date_range::from_int(g.get_start_date()),
                date_range::from_int(g.get_end_date()),
                SPANS[g.get_span().clamp(0, 3) as usize],
                g.get_range_start().to_string(),
                g.get_range_end().to_string(),
                g.get_note().trim().to_owned(),
                g.get_kind() == 1,
            )
        }) else {
            return;
        };
        let Some(date) = start else {
            self.show_form(Some(StatusMessage::error("請選日期")));
            return;
        };
        let mut args = Map::new();
        args.insert("span".into(), json!(span));
        args.insert("date".into(), json!(inputs::date_text(Some(date))));
        let mut summary = day_text(date);
        match span {
            "DAY" => match end {
                Some(end_date) if end_date > date => {
                    args.insert("endDate".into(), json!(inputs::date_text(Some(end_date))));
                    summary += &format!(" 到 {} 整天", day_text(end_date));
                }
                _ => summary += " 整天",
            },
            "HALF_AM" => summary += " 上午",
            "HALF_PM" => summary += " 下午",
            _ => {
                let (Some(s), Some(e)) = (inputs::parse_time(Some(&start_text)), inputs::parse_time(Some(&end_text)))
                else {
                    self.show_form(Some(StatusMessage::error("時段請用 HH:mm，而且結束要晚於開始")));
                    return;
                };
                if e <= s {
                    self.show_form(Some(StatusMessage::error("時段請用 HH:mm，而且結束要晚於開始")));
                    return;
                }
                args.insert("start".into(), json!(s.format("%H:%M").to_string()));
                args.insert("end".into(), json!(e.format("%H:%M").to_string()));
                summary += &format!(" {}–{}", s.format("%H:%M"), e.format("%H:%M"));
            }
        }
        if !note.is_empty() {
            args.insert("note".into(), json!(note));
        }
        let what = if trip { "出差" } else { "請假" };
        if trip {
            let Some(task) = self.st.borrow().trip_task.clone() else {
                self.show_form(Some(StatusMessage::error("出差要選一個任務，出差時段的工時會寫到那裡")));
                return;
            };
            args.insert("kind".into(), json!("TRIP"));
            args.insert("task".into(), json!(task));
            summary += &format!("（工時寫到 {task}）");
        }
        let page = self.clone();
        let text = format!("登記{what}：{summary}？");
        self.confirm(&format!("登記{what}"), &text, move || {
            spawn(async move {
                let api = page.ctx.engine.api();
                let p = page.clone();
                let ok = inputs::run(
                    |b| {
                        p.with(|g| g.set_adding(b));
                    },
                    |m| p.show_form(m),
                    || async move { api.add_leave(args).await.map(drop).map_err(ActionError::from) },
                    Some(&format!("已登記：{summary}")),
                )
                .await;
                if !ok {
                    return;
                }
                page.with(|g| {
                    g.set_note("".into());
                    g.set_range_start("".into());
                    g.set_range_end("".into());
                });
                page.ctx.engine.after_external_change().await;
                page.load().await;
            });
        });
    }

    fn cancel(self: &Rc<Self>, i: usize) {
        let Some(item) = self.st.borrow().items.get(i).cloned() else { return };
        let Some(row) = self.with(|g| g.get_rows().row_data(i)).flatten() else { return };
        let cancel_text = row.cancel_text.to_string();
        let page = self.clone();
        self.confirm(&cancel_text.clone(), &format!("{cancel_text}：{}？", row.period), move || {
            spawn(async move {
                let api = page.ctx.engine.api();
                let p = page.clone();
                let id = item.id.clone();
                let ok = inputs::run(
                    |b| p.set_row_busy(i, b),
                    |m| p.show_list(m),
                    || async move { api.cancel_leave(&id).await.map_err(ActionError::from) },
                    Some(&format!("已{cancel_text}")),
                )
                .await;
                if ok {
                    page.ctx.engine.after_external_change().await;
                    page.load().await;
                }
            });
        });
    }

    fn set_row_busy(&self, i: usize, busy: bool) {
        self.with(|g| {
            let rows = g.get_rows();
            if let Some(mut row) = rows.row_data(i) {
                row.busy = busy;
                rows.set_row_data(i, row);
            }
        });
    }

    /// C# MessageBox.Show(OKCancel)：在頁面上蓋確認視窗，按「確定」才執行 action。
    fn confirm(&self, title: &str, text: &str, action: impl FnOnce() + 'static) {
        self.st.borrow_mut().confirm = Some(Box::new(action));
        self.with(|g| {
            g.set_confirm_title(title.into());
            g.set_confirm_text(text.into());
            g.set_confirm_open(true);
        });
    }
}

fn first_of_month(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

/// 「11/5（週四）」（C# `M/d（ddd）`、zh-TW）。
pub(crate) fn day_text(d: NaiveDate) -> String {
    format!("{}/{}（{}）", d.month(), d.day(), inputs::weekday_short(d))
}

/// 清單的期間：接下來兩週、本月、下個月。
pub(crate) fn list_range(range: &str, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first = first_of_month(today);
    let add = |m: u32| first.checked_add_months(Months::new(m)).unwrap_or(first);
    match range {
        "month" => (first, add(1) - Duration::days(1)),
        "next" => (add(1), add(2) - Duration::days(1)),
        _ => (today, today + Duration::days(13)),
    }
}

/// C# `LeaveRowVm`。
pub(crate) fn leave_row(l: &LeaveItem, mine: bool, from_eip: bool, now: &DateTimeOffset) -> LeaveRow {
    let trip = l.is_trip();
    let name = &l.user.display_name;
    LeaveRow {
        initial: name.chars().next().map_or("?".to_owned(), |c| c.to_string()).into(),
        name: format!("{name}{}", if mine { "（我）" } else { "" }).into(),
        period: format!(
            "{}{}{}",
            if trip { "✈ 出差・" } else { "" },
            l.period,
            if from_eip { "　· 從 EIP 同步" } else { "" }
        )
        .into(),
        // 出差照常算上班，不顯示少掉的時數
        lost: if trip { "".into() } else { l.lost_text.as_str().into() },
        cancel_text: if trip { "取消出差" } else { "取消請假" }.into(),
        can_cancel: mine && l.end_at > *now,
        busy: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};
    use hb_api::PersonRef;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn list_range_兩週本月下個月() {
        let today = ymd(2026, 1, 20);
        assert_eq!((today, ymd(2026, 2, 2)), list_range("2w", today));
        assert_eq!((ymd(2026, 1, 1), ymd(2026, 1, 31)), list_range("month", today));
        assert_eq!((ymd(2026, 2, 1), ymd(2026, 2, 28)), list_range("next", today));
        assert_eq!((ymd(2027, 1, 1), ymd(2027, 1, 31)), list_range("next", ymd(2026, 12, 3)));
    }

    #[test]
    fn leave_row_請假與出差() {
        let tz = FixedOffset::east_opt(8 * 3600).unwrap();
        let now = tz.with_ymd_and_hms(2026, 11, 12, 9, 0, 0).unwrap();
        let mut l = LeaveItem {
            id: "1".into(),
            user: PersonRef::new("amy", "王小美"),
            period: "11/13（週五）整天".into(),
            start_at: tz.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap(),
            end_at: tz.with_ymd_and_hms(2026, 11, 13, 18, 0, 0).unwrap(),
            lost_text: "-8h".into(),
            kind: None,
        };
        let r = leave_row(&l, true, true, &now);
        assert_eq!("王", r.initial.as_str());
        assert_eq!("王小美（我）", r.name.as_str());
        assert_eq!("11/13（週五）整天　· 從 EIP 同步", r.period.as_str());
        assert_eq!("-8h", r.lost.as_str());
        assert_eq!("取消請假", r.cancel_text.as_str());
        assert!(r.can_cancel);
        l.kind = Some("TRIP".into());
        let r = leave_row(&l, false, false, &now);
        assert_eq!("✈ 出差・11/13（週五）整天", r.period.as_str());
        assert_eq!("", r.lost.as_str());
        assert_eq!("取消出差", r.cancel_text.as_str());
        assert!(!r.can_cancel);
        assert_eq!("11/5（週四）", day_text(ymd(2026, 11, 5)));
    }
}
