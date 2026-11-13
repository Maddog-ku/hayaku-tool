//! 工作台「會議」頁（對應 C# `UI/Work/MeetingsPage.xaml(.cs)`；詳細在 `meetings_detail.rs`，對應 `MeetingsPage.Detail.cs`）。
//! 會議：接下來／參加過的、加入並計時、離開、取消、新增會議；點一場看詳細、修改。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

#[path = "w3_common.rs"]
pub(crate) mod w3_common;

#[path = "meetings_detail.rs"]
mod detail;

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use chrono::{Duration, Local, Timelike};
use hb_api::{DateTimeOffset, KeyFinder, MeetingDetail, MeetingItem, ProjectInfo};
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use serde_json::{Map, Value, json};
use slint::{ComponentHandle, Model};

use self::w3_common::{model, spawn, status_parts, strings, wire_md};
use crate::{MeetingRow, MeetingsPageData, PageArg, W3Check, WorkCtx};

pub(crate) struct Page {
    ctx: WorkCtx,
    st: RefCell<State>,
}

#[derive(Default)]
struct State {
    past: bool,
    version: u64,
    items: Vec<MeetingItem>,
    /// 新增表單的專案清單（C# ProjectBox.ItemsSource；null＝還沒讀）
    projects: Option<Vec<ProjectInfo>>,
    /// 新增表單參加者的帳號（跟 attendees 模型同順序）
    attendee_users: Vec<String>,
    member_version: u64,
    // 詳細（MeetingsPage.Detail.cs）
    detail_key: Option<String>,
    detail_version: u64,
    detail: Option<MeetingDetail>,
    /// 修改表單的參加者帳號（跟 edit-people 模型同順序；空＝沒讀到成員清單，不改參加者）
    edit_users: Vec<String>,
    section_loaded: [bool; 4],
    /// 確認視窗按「確定」要做的事
    confirm: Option<Box<dyn FnOnce()>>,
}

thread_local! {
    static PAGE: RefCell<Option<Rc<Page>>> = const { RefCell::new(None) };
}

/// 綁 callback：每個 callback 拿到還活著的 Page 才執行。
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

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page { ctx: ctx.clone(), st: RefCell::new(State::default()) });
    PAGE.with(|p| *p.borrow_mut() = Some(page.clone()));
    let Some(w) = ctx.window() else { return };
    let g = w.global::<MeetingsPageData>();
    g.set_empty_text("".into());
    bind!(g.on_tab_selected, page, |i| {
        page.st.borrow_mut().past = i == 1;
        spawn(async move { page.load().await });
    });
    bind!(g.on_new_clicked, page, || spawn(async move { page.show_form().await }));
    bind!(g.on_cancel_form, page, || {
        page.with(|g| g.set_form_open(false));
    });
    bind!(g.on_create, page, || spawn(async move { page.create().await }));
    bind!(g.on_project_selected, page, |i| spawn(async move { page.project_selected(i).await }));
    bind!(g.on_attendee_toggled, page, |i, c| {
        page.with(|g| set_check(&g.get_attendees(), i, c));
    });
    bind!(g.on_join, page, |i| spawn(async move { page.join(i, true).await }));
    bind!(g.on_leave, page, |i| spawn(async move { page.join(i, false).await }));
    bind!(g.on_pin_row, page, |i| {
        let key = page.st.borrow().items.get(i as usize).map(|m| m.key.clone());
        if let Some(key) = key {
            spawn(async move { page.toggle_pin(key, false).await });
        }
    });
    bind!(g.on_open_row, page, |i| {
        let id = page.st.borrow().items.get(i as usize).map(|m| m.id.clone());
        if let Some(id) = id {
            hb_win::open(&page.ctx.engine.meeting_url(&id));
        }
    });
    bind!(g.on_cancel_meeting, page, |i| page.cancel_meeting(i as usize));
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
    wire_md!(&g);
    detail::bind(&page, &g);
}

pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    page.activate(arg).await;
}

/// 勾選項目改勾或取消。
pub(crate) fn set_check(items: &slint::ModelRc<W3Check>, i: i32, checked: bool) {
    if let Some(mut c) = items.row_data(i as usize) {
        c.checked = checked;
        items.set_row_data(i as usize, c);
    }
}

impl Page {
    /// 對頁面 global 做事（視窗關掉時什麼都不做）。
    pub(crate) fn with<R>(&self, f: impl FnOnce(&MeetingsPageData<'_>) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let g = w.global::<MeetingsPageData>();
        Some(f(&g))
    }

    fn show_list(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_list_status(text);
            g.set_list_tone(tone);
        });
    }

    fn show_form_status(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_form_status(text);
            g.set_form_tone(tone);
        });
    }

    /// arg："new" 開新增表單；會議編號或 id（例如從會議通知點過來）直接打開那場的詳細。
    async fn activate(self: Rc<Self>, arg: Option<PageArg>) {
        let text = arg.as_ref().and_then(|a| a.as_text()).map(str::to_owned);
        if text.as_deref() == Some("new") {
            self.clone().show_form().await;
        }
        self.clone().load().await;
        if let Some(meeting) = text.filter(|t| t != "new") {
            self.open_detail(meeting).await;
        }
    }

    pub(crate) async fn load(self: Rc<Self>) {
        let version = {
            let mut st = self.st.borrow_mut();
            st.version += 1;
            st.version
        };
        self.with(|g| {
            g.set_empty_text("讀取中…".into());
            g.set_loading(true);
        });
        self.show_list(None);
        let past = self.st.borrow().past;
        let api = self.ctx.engine.api();
        let result = if past { api.past_meetings().await } else { api.upcoming_meetings().await };
        if version != self.st.borrow().version {
            return;
        }
        self.with(|g| g.set_loading(false));
        match result {
            Ok(items) => {
                self.st.borrow_mut().items = items;
                self.render_rows();
                let empty = self.st.borrow().items.is_empty();
                let text = if !empty {
                    ""
                } else if past {
                    "還沒有參加過的會議"
                } else {
                    "接下來沒有會議"
                };
                self.with(|g| g.set_empty_text(text.into()));
            }
            Err(e) => {
                log::error!("讀取會議失敗: {e}");
                self.with(|g| g.set_empty_text("".into()));
                self.show_list(Some(StatusMessage::error(format!("讀取失敗：{e}"))));
            }
        }
    }

    /// 清單整份重畫（讀完會議）。
    fn render_rows(&self) {
        let st = self.st.borrow();
        let now = Local::now().fixed_offset();
        let engine = &self.ctx.engine;
        let rows: Vec<MeetingRow> = st
            .items
            .iter()
            .map(|m| {
                meeting_row(
                    m,
                    st.past,
                    st.detail_key.as_deref() == Some(m.key.as_str()),
                    engine.is_pinned(&m.key),
                    &now,
                )
            })
            .collect();
        drop(st);
        self.with(|g| g.set_rows(model(rows)));
    }

    /// 托盤變了（釘選、取消）或換了選取：清單上的圖釘、底色跟著變，不重讀會議（C# RenderList、row.Selected）。
    pub(crate) fn refresh_rows(&self) {
        let st = self.st.borrow();
        let engine = &self.ctx.engine;
        self.with(|g| {
            let rows = g.get_rows();
            for i in 0..rows.row_count() {
                let Some(mut row) = rows.row_data(i) else { continue };
                let pinned = engine.is_pinned(&row.key);
                let selected = st.detail_key.as_deref() == Some(row.key.as_str());
                if row.pinned != pinned || row.selected != selected {
                    row.pinned = pinned;
                    row.pin_tip = pin_tip(pinned).into();
                    row.selected = selected;
                    rows.set_row_data(i, row);
                }
            }
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

    /// 加入並計時／離開（加入時暫停了其他計時的話，離開會恢復）。
    async fn join(self: Rc<Self>, i: i32, join: bool) {
        let Some(item) = self.st.borrow().items.get(i as usize).cloned() else { return };
        self.set_row_busy(i as usize, true);
        if join {
            self.ctx.engine.join_meeting(&item).await;
        } else {
            self.ctx.engine.leave_meeting(&item).await;
        }
        self.show_list(self.ctx.engine.error().map(StatusMessage::error));
        self.load().await;
    }

    fn cancel_meeting(self: &Rc<Self>, i: usize) {
        let Some(item) = self.st.borrow().items.get(i).cloned() else { return };
        let page = self.clone();
        self.confirm(
            "取消會議",
            &format!("取消會議「{}」？會通知所有參加者。", item.title),
            move || {
                spawn(async move {
                    let p = page.clone();
                    let ok = inputs::run(
                        |b| p.set_row_busy(i, b),
                        |m| p.show_list(m),
                        || async { page.ctx.engine.api().cancel_meeting(&item.key).await.map_err(ActionError::from) },
                        Some("已取消會議"),
                    )
                    .await;
                    if ok {
                        page.ctx.engine.after_external_change().await;
                        page.load().await;
                    }
                });
            },
        );
    }

    /// C# MessageBox.Show(OKCancel)：在頁面上蓋確認視窗，按「確定」才執行 action。
    pub(crate) fn confirm(&self, title: &str, text: &str, action: impl FnOnce() + 'static) {
        self.st.borrow_mut().confirm = Some(Box::new(action));
        self.with(|g| {
            g.set_confirm_title(title.into());
            g.set_confirm_text(text.into());
            g.set_confirm_open(true);
        });
    }

    // ───────────── 新增 ─────────────

    async fn show_form(self: Rc<Self>) {
        let start = Local::now() + Duration::hours(1);
        let start =
            start.with_minute(0).and_then(|t| t.with_second(0)).and_then(|t| t.with_nanosecond(0)).unwrap_or(start);
        let date = if start.date_naive() == Local::now().date_naive() {
            "今天".to_owned()
        } else {
            start.format("%-m/%-d").to_string()
        };
        let end = start + Duration::hours(1);
        self.with(|g| {
            g.set_form_open(true);
            g.set_date(date.into());
            g.set_start(start.format("%H:%M").to_string().into());
            g.set_end(end.format("%H:%M").to_string().into());
            g.set_focus_title(g.get_focus_title() + 1);
        });
        self.show_form_status(None);
        if self.st.borrow().projects.is_some() {
            return;
        }
        match self.ctx.engine.api().list_projects(false).await {
            Ok(projects) => {
                let preferred =
                    self.ctx.engine.running_keys().iter().next().map(|k| KeyFinder::project_of(k).to_owned());
                let index = projects
                    .iter()
                    .position(|p| Some(&p.key) == preferred.as_ref())
                    .or(if projects.is_empty() { None } else { Some(0) });
                let names = strings(projects.iter().map(|p| p.name.as_str()));
                self.st.borrow_mut().projects = Some(projects);
                self.with(|g| {
                    g.set_projects(names);
                    g.set_project_index(index.map_or(-1, |i| i as i32));
                });
                if let Some(i) = index {
                    self.project_selected(i as i32).await; // C# 設 SelectedItem 會觸發 SelectionChanged
                }
            }
            Err(e) => self.show_form_status(Some(StatusMessage::error(format!("讀不到專案清單：{e}")))),
        }
    }

    async fn project_selected(self: Rc<Self>, i: i32) {
        let Some(project) = self.st.borrow().projects.as_ref().and_then(|p| p.get(i as usize).cloned()) else { return };
        let version = {
            let mut st = self.st.borrow_mut();
            st.member_version += 1;
            st.member_version
        };
        match self.ctx.engine.get_project(&project.key).await {
            Ok(detail) => {
                if version != self.st.borrow().member_version {
                    return;
                }
                let me = self.ctx.engine.user().map(|u| u.username);
                let checks: Vec<W3Check> = detail
                    .members
                    .iter()
                    .map(|m| W3Check {
                        id: m.username.as_str().into(),
                        text: m.display_name.as_str().into(),
                        checked: Some(&m.username) == me.as_ref(),
                    })
                    .collect();
                self.st.borrow_mut().attendee_users = detail.members.iter().map(|m| m.username.clone()).collect();
                self.with(|g| g.set_attendees(model(checks)));
            }
            Err(e) => self.show_form_status(Some(StatusMessage::error(format!("讀不到專案成員：{}", e.message())))),
        }
    }

    async fn create(self: Rc<Self>) {
        let Some((title, date, start, end, location, note, index, attendees)) = self.with(|g| {
            (
                g.get_title().trim().to_owned(),
                g.get_date().to_string(),
                g.get_start().to_string(),
                g.get_end().to_string(),
                g.get_location().trim().to_owned(),
                g.get_note().trim().to_owned(),
                g.get_project_index(),
                g.get_attendees().iter().filter(|a| a.checked).map(|a| a.id.to_string()).collect::<Vec<_>>(),
            )
        }) else {
            return;
        };
        let Some(project) = self.st.borrow().projects.as_ref().and_then(|p| p.get(index.max(-1) as usize).cloned())
        else {
            self.show_form_status(Some(StatusMessage::error("請選專案")));
            return;
        };
        if title.is_empty() {
            self.show_form_status(Some(StatusMessage::error("請填標題")));
            self.with(|g| g.set_focus_title(g.get_focus_title() + 1));
            return;
        }
        let Some(date) = inputs::parse_date(Some(&date), None) else {
            self.show_form_status(Some(StatusMessage::error("日期看不懂，請用 今天、明天、11/5 這類寫法")));
            return;
        };
        let (Some(start), Some(end)) = (inputs::parse_time(Some(&start)), inputs::parse_time(Some(&end))) else {
            self.show_form_status(Some(StatusMessage::error("開始、結束請用 HH:mm，例 14:00")));
            return;
        };
        if end <= start {
            self.show_form_status(Some(StatusMessage::error("結束要晚於開始（會議要在同一天）")));
            return;
        }
        let mut attendees = attendees;
        if attendees.is_empty() {
            attendees.push("me".to_owned());
        }
        let mut args = Map::new();
        args.insert("project".into(), json!(project.key));
        args.insert("title".into(), json!(title));
        args.insert("startAt".into(), json!(inputs::iso(&inputs::local(date, start))));
        args.insert("endAt".into(), json!(inputs::iso(&inputs::local(date, end))));
        args.insert("attendees".into(), json!(attendees));
        if !location.is_empty() {
            args.insert("location".into(), json!(location));
        }
        if !note.is_empty() {
            args.insert("note".into(), json!(note));
        }
        let p = self.clone();
        let api = self.ctx.engine.api();
        let ok = inputs::run(
            |b| {
                p.with(|g| g.set_creating(b));
            },
            |m| p.show_form_status(m),
            || async move { api.create_meeting(args).await.map_err(ActionError::from) },
            None,
        )
        .await;
        if !ok {
            return;
        }
        self.with(|g| {
            g.set_title("".into());
            g.set_location("".into());
            g.set_note("".into());
            g.set_form_open(false);
        });
        self.show_list(Some(StatusMessage::ok(format!("已建立會議「{title}」"))));
        self.ctx.engine.after_external_change().await;
        self.load().await;
    }
}

/// 釘選圖示的提示（C# MeetingListVm.PinTip）。
pub(crate) fn pin_tip(pinned: bool) -> &'static str {
    if pinned {
        "已釘在托盤，點一下取消"
    } else {
        "釘到托盤：開會前 10 分鐘起在面板按 ▶ 就加入"
    }
}

/// C# `MeetingListVm`：清單上一場會議要顯示的東西。
pub(crate) fn meeting_row(
    m: &MeetingItem,
    past: bool,
    selected: bool,
    pinned: bool,
    now: &DateTimeOffset,
) -> MeetingRow {
    let start = m.start_at.with_timezone(&Local);
    let end = m.end_at.with_timezone(&Local);
    let detail: Vec<&str> = [Some(m.project.as_str()), m.location.as_deref(), m.my_status.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect();
    let ongoing = *now >= m.start_at && *now < m.end_at;
    MeetingRow {
        key: m.key.as_str().into(),
        title: m.title.as_str().into(),
        day: inputs::short_date(start.date_naive()).into(),
        time: format!("{}–{}", start.format("%H:%M"), end.format("%H:%M")).into(),
        time_tone: if ongoing {
            1
        } else if past {
            2
        } else {
            0
        },
        detail: detail.join(" · ").into(),
        join: !past && !m.in_meeting() && *now >= m.start_at - Duration::minutes(15) && *now < m.end_at,
        leave: m.in_meeting(),
        cancel: !past && *now < m.start_at,
        pinned,
        pin_tip: pin_tip(pinned).into(),
        selected,
        busy: false,
    }
}

/// C# `MeetingsPage.AttendeeChanges`：參加者名單的差異（要加、要拿掉的帳號，排序過）；沒讀到成員清單（loaded=false）時不改參加者。
pub(crate) fn attendee_changes<'a>(
    current: impl IntoIterator<Item = &'a str>,
    selected: impl IntoIterator<Item = &'a str>,
    loaded: bool,
) -> (Vec<String>, Vec<String>) {
    if !loaded {
        return (Vec::new(), Vec::new());
    }
    let fold = |s: &str| s.to_lowercase();
    let mut before: Vec<&str> = Vec::new();
    for u in current.into_iter().filter(|u| !u.is_empty()) {
        if !before.iter().any(|b| fold(b) == fold(u)) {
            before.push(u);
        }
    }
    let mut after: Vec<&str> = Vec::new();
    for u in selected.into_iter().filter(|u| !u.is_empty()) {
        if !after.iter().any(|a| fold(a) == fold(u)) {
            after.push(u);
        }
    }
    let mut add: Vec<String> =
        after.iter().filter(|u| !before.iter().any(|b| fold(b) == fold(u))).map(|u| u.to_string()).collect();
    let mut remove: Vec<String> =
        before.iter().filter(|u| !after.iter().any(|a| fold(a) == fold(u))).map(|u| u.to_string()).collect();
    add.sort();
    remove.sort();
    (add, remove)
}

/// C# `MeetingsPage.FileSize`：位元組 → 「20 KB」「1.4 MB」；不知道大小回空字串（KB 四捨五入到整數，.NET Math.Round 是銀行家捨入）。
pub(crate) fn file_size(bytes: i64) -> String {
    match bytes {
        b if b <= 0 => String::new(),
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", (b as f64 / 1024.0).round_ties_even() as i64),
        b => format!("{} MB", w3_common::one_decimal(b as f64 / 1024.0 / 1024.0)),
    }
}

/// 送出的欄位裡的字串值（顯示「已修改會議「…」」用）。
fn field_text(fields: &Map<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    fn at(h: u32, m: u32) -> DateTimeOffset {
        FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 11, 12, h, m, 0).unwrap()
    }

    fn meeting(status: Option<&str>) -> MeetingItem {
        MeetingItem {
            key: "M-1".into(),
            id: "m1".into(),
            title: "週會".into(),
            project: "F2P".into(),
            start_at: at(14, 0),
            end_at: at(15, 0),
            location: Some("  ".into()),
            my_status: status.map(str::to_owned),
        }
    }

    #[test]
    fn attendee_changes_算出要加與要拿掉的() {
        let (add, remove) = attendee_changes(["amy", "Bob", ""], ["bob", "carl", "Amy", "carl"], true);
        assert_eq!(vec!["carl".to_owned()], add);
        assert!(remove.is_empty());
        let (add, remove) = attendee_changes(["amy", "bob"], ["bob"], true);
        assert!(add.is_empty());
        assert_eq!(vec!["amy".to_owned()], remove);
        // 沒讀到成員清單：不改參加者
        let (add, remove) = attendee_changes(["amy"], [], false);
        assert!(add.is_empty() && remove.is_empty());
    }

    #[test]
    fn file_size_跟_csharp_一樣() {
        assert_eq!("", file_size(0));
        assert_eq!("", file_size(-5));
        assert_eq!("512 B", file_size(512));
        assert_eq!("20 KB", file_size(20 * 1024));
        // 2.5 KB → 銀行家捨入成 2
        assert_eq!("2 KB", file_size(2560));
        assert_eq!("1.4 MB", file_size(1_468_006));
    }

    #[test]
    fn meeting_row_按鈕跟時間顏色() {
        let m = meeting(None);
        // 開會前 20 分鐘：還不能加入，可以取消
        let r = meeting_row(&m, false, false, false, &at(13, 40));
        assert!(!r.join && r.cancel && !r.leave);
        assert_eq!(0, r.time_tone);
        assert_eq!("F2P", r.detail.as_str());
        // 開會前 10 分鐘：可以加入
        let r = meeting_row(&m, false, false, true, &at(13, 50));
        assert!(r.join && r.cancel);
        assert_eq!("已釘在托盤，點一下取消", r.pin_tip.as_str());
        // 進行中：強調色、不能取消
        let r = meeting_row(&m, false, true, false, &at(14, 30));
        assert!(r.join && !r.cancel && r.selected);
        assert_eq!(1, r.time_tone);
        // 會議中：離開
        let r = meeting_row(&meeting(Some("會議中")), false, false, false, &at(14, 30));
        assert!(!r.join && r.leave);
        assert_eq!("F2P · 會議中", r.detail.as_str());
        // 參加過的：淡色、沒有加入與取消
        let r = meeting_row(&m, true, false, false, &at(16, 0));
        assert!(!r.join && !r.cancel);
        assert_eq!(2, r.time_tone);
    }
}
