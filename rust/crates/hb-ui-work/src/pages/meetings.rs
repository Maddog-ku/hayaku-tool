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
    /// 進行中的會議的參加者狀態（會議 key → 會議中、還沒加入、請假的名字；跟上 Hayaku 1.5.5）
    people: std::collections::HashMap<String, MeetingPeople>,
    /// 「今天已結束」那一區展開了
    ended_open: bool,
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
    bind!(g.on_ended_toggled, page, || {
        let open = !page.st.borrow().ended_open;
        page.st.borrow_mut().ended_open = open;
        page.render_rows();
    });
    bind!(g.on_attendee_toggled, page, |i, c| {
        page.with(|g| set_check(&g.get_attendees(), i, c));
    });
    bind!(g.on_pick_attendees, page, || {
        let Some(items) = page.with(|g| g.get_attendees()) else { return };
        let weak = Rc::downgrade(&page);
        pick_checks(items, "選擇參加者", "people:meeting-new", move |next| {
            if let Some(page) = weak.upgrade() {
                page.with(|g| {
                    g.set_attendees_text(checks_text(&next).into());
                    g.set_attendees(next);
                });
            }
        });
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

/// 已選的人（顯示在選人欄位）：四個以內全列，多了寫「甲、乙、丙 等 5 人」。
pub(crate) fn checks_text(items: &slint::ModelRc<W3Check>) -> String {
    let names: Vec<String> = items.iter().filter(|c| c.checked).map(|c| c.text.to_string()).collect();
    match names.len() {
        0 => String::new(),
        n if n <= 4 => names.join("、"),
        n => format!("{} 等 {n} 人", names[..3].join("、")),
    }
}

/// 用選人視窗改勾選（跟上 Hayaku 1.5.5）：可以選的是模型裡的人，選好換掉勾選（順序不變）後交給 done。
pub(crate) fn pick_checks(
    items: slint::ModelRc<W3Check>,
    heading: &str,
    tag: &str,
    done: impl Fn(slint::ModelRc<W3Check>) + 'static,
) {
    let members = items.iter().map(|c| hb_api::PersonRef::new(c.id.to_string(), c.text.to_string())).collect();
    let selected = items.iter().filter(|c| c.checked).map(|c| c.id.to_string()).collect();
    let pick = hb_ui_common::people_picker::PeoplePick {
        heading: heading.to_owned(),
        hint: String::new(),
        members,
        selected,
        multi: true,
    };
    hb_ui_common::people_picker::open(pick, Some(tag.to_owned()), move |chosen| {
        let next: Vec<W3Check> = items
            .iter()
            .map(|mut c| {
                c.checked = chosen.iter().any(|u| u.eq_ignore_ascii_case(&c.id));
                c
            })
            .collect();
        done(model(next));
    });
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
        let now = Local::now().fixed_offset();
        let result = if past {
            api.past_meetings().await
        } else {
            // 接下來的＋今天已經開過的（跟上 Hayaku 1.5.5：開過的灰掉、收成一區）
            let today = now.with_timezone(&Local).date_naive();
            match api.upcoming_meetings().await {
                Ok(mut items) => {
                    match api.list_meetings(today, today, true).await {
                        Ok(todays) => {
                            for m in todays {
                                if !items.iter().any(|x| x.key == m.key) {
                                    items.push(m);
                                }
                            }
                        }
                        Err(e) => log::error!("讀取今天的會議失敗: {e}"),
                    }
                    Ok(arrange(items, &now))
                }
                Err(e) => Err(e),
            }
        };
        if version != self.st.borrow().version {
            return;
        }
        self.with(|g| g.set_loading(false));
        match result {
            Ok(items) => {
                // 進行中的會議讀參加者狀態（最多 3 場；沒有進行中的會議時不多打請求）
                let ongoing: Vec<String> = if past {
                    Vec::new()
                } else {
                    items.iter().filter(|m| now >= m.start_at && now < m.end_at).take(3).map(|m| m.key.clone()).collect()
                };
                let mut people = std::collections::HashMap::new();
                for key in ongoing {
                    match api.get_meeting(&key).await {
                        Ok(d) => {
                            people.insert(key, MeetingPeople::from(&d));
                        }
                        Err(e) => log::error!("讀取 {key} 的參加者失敗: {e}"),
                    }
                }
                if version != self.st.borrow().version {
                    return;
                }
                // 整個清單是空的時由空白畫面說明，不另外顯示「今天沒有會議」
                let today_empty = !past && !items.is_empty() && !items.iter().any(|m| is_today(m, &now));
                {
                    let mut st = self.st.borrow_mut();
                    st.items = items;
                    st.people = people;
                }
                self.with(|g| g.set_today_empty(today_empty));
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
        let mut rows: Vec<MeetingRow> = st
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
        if !st.past {
            decorate(&mut rows, &st.items, &st.people, st.ended_open, &now);
        }
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
                let checks = model(checks);
                self.with(|g| {
                    g.set_attendees_text(checks_text(&checks).into());
                    g.set_attendees(checks);
                });
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

/// 進行中的會議的參加者：會議中、還沒加入、請假的名字（用頓號接起來）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MeetingPeople {
    pub inside: String,
    pub waiting: String,
    pub leave: String,
}

impl MeetingPeople {
    fn from(d: &MeetingDetail) -> Self {
        let names = |f: &dyn Fn(&str) -> bool| {
            d.attendees.iter().filter(|a| f(&a.state)).map(|a| a.name.clone()).collect::<Vec<_>>().join("、")
        };
        Self {
            inside: names(&|s| s == "會議中"),
            waiting: names(&|s| s.contains("還沒") || s.contains("未加入")),
            leave: names(&|s| s.contains("請假")),
        }
    }
}

fn is_today(m: &MeetingItem, now: &DateTimeOffset) -> bool {
    let today = now.with_timezone(&Local).date_naive();
    m.start_at.with_timezone(&Local).date_naive() <= today && m.end_at.with_timezone(&Local).date_naive() >= today
}

/// 「接下來」的排法（跟上 Hayaku 1.5.5）：今天還沒結束的（照開始時間）→ 今天已結束的 → 之後幾天的。
pub(crate) fn arrange(items: Vec<MeetingItem>, now: &DateTimeOffset) -> Vec<MeetingItem> {
    let (mut today, mut later): (Vec<MeetingItem>, Vec<MeetingItem>) = items.into_iter().partition(|m| is_today(m, now));
    today.sort_by_key(|m| (m.end_at <= *now, m.start_at));
    later.sort_by_key(|m| m.start_at);
    today.extend(later);
    today
}

/// 加上分組標題、「現在」線、放大進行中的、灰掉並收起已結束的（rows 與 items 同順序，已經用 arrange 排過）。
pub(crate) fn decorate(
    rows: &mut [MeetingRow],
    items: &[MeetingItem],
    people: &std::collections::HashMap<String, MeetingPeople>,
    ended_open: bool,
    now: &DateTimeOffset,
) {
    let today = now.with_timezone(&Local).date_naive();
    let now_text = format!("現在 {}", now.with_timezone(&Local).format("%H:%M"));
    let ended_count = items.iter().filter(|m| is_today(m, now) && m.end_at <= *now).count();
    let mut last_day = None;
    let mut ended_seen = false;
    let mut now_placed = false;
    for (i, (row, m)) in rows.iter_mut().zip(items).enumerate() {
        let todays = is_today(m, now);
        let ended = todays && m.end_at <= *now;
        let ongoing = *now >= m.start_at && *now < m.end_at;
        if todays && !ended {
            if last_day != Some(today) {
                row.head = format!("今天 {}", crate::pages::hours::w2_common::md_weekday(today)).into();
                last_day = Some(today);
            }
            if !now_placed && m.start_at > *now {
                row.now_before = true;
                row.now_text = now_text.as_str().into();
                now_placed = true;
            }
            let next_active = items.get(i + 1).is_some_and(|n| is_today(n, now) && n.end_at > *now);
            if !now_placed && !next_active {
                row.now_after = true;
                row.now_text = now_text.as_str().into();
                now_placed = true;
            }
        } else if ended {
            if !ended_seen {
                row.head = format!("今天已結束（{ended_count}）").into();
                row.head_toggle = true;
                ended_seen = true;
                last_day = Some(today);
            }
            row.dim = true;
            row.hidden = !ended_open;
        } else {
            let day = m.start_at.with_timezone(&Local).date_naive();
            if last_day != Some(day) {
                row.head = crate::pages::hours::w2_common::md_weekday(day).into();
                last_day = Some(day);
            }
        }
        if ongoing {
            row.big = true;
            if let Some(p) = people.get(&m.key) {
                row.people_in = p.inside.as_str().into();
                row.people_wait = p.waiting.as_str().into();
                row.people_leave = p.leave.as_str().into();
            }
        }
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
        ..Default::default()
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
    fn 會議列表_今天排在現在線上下_已結束收起_之後的照天分組() {
        // 本地時區不一定是 +8：用本地時間建 now，會議也用同一天的本地時間
        let local = |d: u32, h: u32, m: u32| Local.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap().fixed_offset();
        let now = local(12, 14, 30);
        let mk = |key: &str, d: u32, s: (u32, u32), e: (u32, u32)| MeetingItem {
            key: key.into(),
            start_at: local(d, s.0, s.1),
            end_at: local(d, e.0, e.1),
            ..meeting(None)
        };
        let items = arrange(
            vec![
                mk("later", 13, (9, 0), (10, 0)),
                mk("future", 12, (16, 0), (17, 0)),
                mk("ended", 12, (9, 0), (10, 0)),
                mk("ongoing", 12, (14, 0), (15, 0)),
            ],
            &now,
        );
        assert_eq!(items.iter().map(|m| m.key.as_str()).collect::<Vec<_>>(), vec!["ongoing", "future", "ended", "later"]);
        let mut rows: Vec<MeetingRow> = items.iter().map(|m| meeting_row(m, false, false, false, &now)).collect();
        let mut people = std::collections::HashMap::new();
        people.insert("ongoing".to_owned(), MeetingPeople { inside: "小明".into(), waiting: "小華".into(), leave: String::new() });
        decorate(&mut rows, &items, &people, false, &now);
        assert!(rows[0].head.starts_with("今天 11/12"));
        assert!(rows[0].big && rows[0].people_in == "小明" && rows[0].people_wait == "小華");
        assert!(rows[1].now_before && rows[1].now_text == "現在 14:30" && !rows[0].now_before);
        assert_eq!((rows[2].head.as_str(), rows[2].head_toggle, rows[2].dim, rows[2].hidden), ("今天已結束（1）", true, true, true));
        assert!(rows[3].head.starts_with("11/13") && !rows[3].dim);
        // 今天的都已經開始了：現在線畫在最後一場進行中的下面
        let items = arrange(vec![mk("ongoing", 12, (14, 0), (15, 0))], &now);
        let mut rows: Vec<MeetingRow> = items.iter().map(|m| meeting_row(m, false, false, false, &now)).collect();
        decorate(&mut rows, &items, &people, true, &now);
        assert!(rows[0].now_after && !rows[0].now_before);
    }

    #[test]
    fn 已選的人_多了寫等幾人() {
        let c = |n: &str, on: bool| W3Check { id: n.into(), text: n.into(), checked: on };
        let few = model(vec![c("甲", true), c("乙", false), c("丙", true)]);
        assert_eq!(checks_text(&few), "甲、丙");
        let many = model(["甲", "乙", "丙", "丁", "戊"].iter().map(|n| c(n, true)).collect());
        assert_eq!(checks_text(&many), "甲、乙、丙 等 5 人");
        assert_eq!(checks_text(&model(vec![c("甲", false)])), "");
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
