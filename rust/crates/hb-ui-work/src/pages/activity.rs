//! 工作台「專案動態」頁（對應 C# `UI/Work/ActivityPage.cs`）。
//! 誰開始做了什麼、新增了哪些任務、完成了哪些（跟網頁「專案動態」視圖同一套），
//! 上方是這段期間的進展摘要；可以選專案、期間、事件類型，只看自己的。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use chrono::{Datelike, Duration, Local, NaiveDate};
use hb_api::{ActivityFeed, ActivityItem, KeyFinder, ProjectInfo};
use hb_ui_common::ProjectIcons;
use hb_ui_common::inputs::StatusMessage;
use slint::{ComponentHandle, Model};

use super::meetings::w3_common::{model, spawn, status_parts, strings};
use crate::{ActCard, ActDay, ActRow, ActSpan, ActivityPageData, PageArg, WorkCtx};

/// 事件類型（C# Types）與期間（C# Periods）。
const TYPES: [(&str, &str); 4] = [("progress", "進展"), ("assign", "指派"), ("talk", "討論"), ("other", "其他")];
const PERIODS: [(&str, &str); 3] = [("week", "本週"), ("lastWeek", "上週"), ("month", "最近 30 天")];

struct Page {
    ctx: WorkCtx,
    st: RefCell<State>,
}

#[derive(Default)]
struct State {
    /// 下拉選單：None＝全部專案
    projects: Vec<Option<ProjectInfo>>,
    project: Option<String>,
    version: u64,
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

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page { ctx: ctx.clone(), st: RefCell::new(State::default()) });
    PAGE.with(|p| *p.borrow_mut() = Some(page.clone()));
    let Some(w) = ctx.window() else { return };
    let g = w.global::<ActivityPageData>();
    bind!(g.on_project_selected, page, |i| {
        let project = page.st.borrow().projects.get(i as usize).cloned().flatten();
        page.st.borrow_mut().project = project.map(|p| p.key);
        page.render_project_icon();
        spawn(async move { page.load().await });
    });
    bind!(g.on_period_selected, page, |_i| spawn(async move { page.load().await }));
    bind!(g.on_type_toggled, page, |i, checked| {
        page.with(|g| {
            let types = g.get_types();
            types.set_row_data(i as usize, checked);
            if !types.iter().any(|t| t) {
                types.set_row_data(i as usize, true); // 至少要一種
            }
        });
        spawn(async move { page.load().await });
    });
    bind!(g.on_only_me_toggled, page, |_c| spawn(async move { page.load().await }));
    bind!(g.on_reload, page, || spawn(async move { page.load().await }));
    bind!(g.on_open_task, page, |key| page.ctx.show_page("tasks", Some(PageArg::Text(key.to_string()))));
    g.set_types(model(TYPES.iter().map(|(k, _)| *k == "progress").collect()));
}

pub async fn activate(_ctx: WorkCtx, _arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    page.render_projects();
    page.load().await;
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&ActivityPageData<'_>) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let g = w.global::<ActivityPageData>();
        Some(f(&g))
    }

    fn show(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_status(text);
            g.set_status_tone(tone);
        });
    }

    fn render_projects(&self) {
        let (names, index) = {
            let mut st = self.st.borrow_mut();
            let mut items: Vec<Option<ProjectInfo>> = vec![None];
            items.extend(self.ctx.engine.projects().iter().filter(|p| !p.is_archived).cloned().map(Some));
            let index = items.iter().position(|p| p.as_ref().map(|p| &p.key) == st.project.as_ref()).unwrap_or(0);
            st.project = items[index].as_ref().map(|p| p.key.clone());
            let names: Vec<String> =
                items.iter().map(|p| p.as_ref().map_or("全部專案".to_owned(), |p| p.name.clone())).collect();
            st.projects = items;
            (names, index)
        };
        self.with(|g| {
            g.set_projects(strings(names));
            g.set_project_index(index as i32);
        });
        self.render_project_icon();
    }

    fn render_project_icon(&self) {
        let icon = {
            let st = self.st.borrow();
            st.projects
                .iter()
                .flatten()
                .find(|p| Some(&p.key) == st.project.as_ref())
                .map(|p| ProjectIcons::icon(Some(p), &p.name))
        };
        self.with(|g| match icon {
            Some(icon) => {
                g.set_project_glyph(icon.glyph.into());
                g.set_project_tint(icon.color.to_slint());
            }
            None => g.set_project_glyph("".into()),
        });
    }

    async fn load(self: Rc<Self>) {
        let version = {
            let mut st = self.st.borrow_mut();
            st.version += 1;
            st.version
        };
        self.show(None);
        let Some((period, types, only_me)) = self.with(|g| {
            let types: Vec<String> = g
                .get_types()
                .iter()
                .zip(TYPES.iter())
                .filter(|(on, _)| *on)
                .map(|(_, (k, _))| (*k).to_owned())
                .collect();
            (g.get_period(), types, g.get_only_me())
        }) else {
            return;
        };
        self.with(|g| g.set_top("讀取中…".into()));
        let period = period.clamp(0, PERIODS.len() as i32 - 1) as usize;
        let (from, to) = range(PERIODS[period].0, Local::now().date_naive());
        let project = self.st.borrow().project.clone();
        let me = ["me".to_owned()];
        let result = self
            .ctx
            .engine
            .api()
            .project_activity(project.as_deref(), from, to, &types, only_me.then_some(&me[..]))
            .await;
        if version != self.st.borrow().version {
            return;
        }
        match result {
            Ok(feed) => self.render(&feed, PERIODS[period].1),
            Err(e) => {
                log::error!("讀取專案動態失敗: {e}");
                self.with(|g| g.set_top("".into()));
                self.show(Some(StatusMessage::error(format!("讀取失敗：{e}"))));
            }
        }
    }

    fn render(&self, feed: &ActivityFeed, period: &str) {
        let engine = &self.ctx.engine;
        let cards: Vec<ActCard> = [
            ("新增", feed.created, 4),
            ("開始", feed.started, 2),
            ("完成", feed.completed, 0),
            ("逾期", feed.overdue, 5),
        ]
        .into_iter()
        .map(|(label, value, color)| ActCard {
            label: label.into(),
            value: value.to_string().into(),
            color: if value > 0 { color } else { 1 },
        })
        .collect();
        let has_projects = !engine.projects().is_empty();
        let days: Vec<ActDay> = group_by_day(&feed.items)
            .into_iter()
            .map(|(label, items)| ActDay {
                label: label.into(),
                rows: model(
                    items
                        .into_iter()
                        .map(|item| {
                            let project = engine.project_of(&item.project);
                            let icon = ProjectIcons::icon(project.as_ref(), &item.project);
                            let spans = link_spans(&item.text, |key| {
                                !has_projects || engine.project_of(KeyFinder::project_of(key)).is_some()
                            });
                            ActRow {
                                time: item.at.split(' ').nth(1).unwrap_or(&item.at).into(),
                                glyph: icon.glyph.into(),
                                tint: icon.color.to_slint(),
                                project: project.map_or_else(|| item.project.clone(), |p| p.name).into(),
                                text: item.text.as_str().into(),
                                spans: model(
                                    spans
                                        .into_iter()
                                        .map(|(text, key)| ActSpan { text: text.into(), key: key.into() })
                                        .collect(),
                                ),
                            }
                        })
                        .collect(),
                ),
            })
            .collect();
        let empty = feed.items.is_empty();
        self.with(|g| {
            g.set_cards(model(cards));
            g.set_top(top_text(feed, period).into());
            g.set_days(model(days));
            g.set_empty_text(if empty { "這段期間沒有動態" } else { "" }.into());
        });
    }
}

/// 期間（本週一到今天、上週一到上週日、最近 30 天）。
pub(crate) fn range(period: &str, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    match period {
        "lastWeek" => (monday - Duration::days(7), monday - Duration::days(1)),
        "month" => (today - Duration::days(29), today),
        _ => (monday, today),
    }
}

pub(crate) fn top_text(feed: &ActivityFeed, period: &str) -> String {
    let mut text = format!("{period}（{}～{}）的進展", feed.from.format("%-m/%-d"), feed.to.format("%-m/%-d"));
    if !feed.top_members.is_empty() {
        text += "　最活躍：";
        text += &feed.top_members.iter().take(5).map(|(n, c)| format!("{n} {c}")).collect::<Vec<_>>().join("、");
    }
    if feed.items.len() >= 50 {
        text += "　（只列最新 50 則）";
    }
    text
}

/// C# `GroupBy(i => i.At.Split(' ')[0])`：照第一次出現的順序分組。
pub(crate) fn group_by_day(items: &[ActivityItem]) -> Vec<(String, Vec<&ActivityItem>)> {
    let mut groups: Vec<(String, Vec<&ActivityItem>)> = Vec::new();
    for item in items {
        let day = item.at.split(' ').next().unwrap_or("").to_owned();
        match groups.iter_mut().find(|(d, _)| *d == day) {
            Some((_, list)) => list.push(item),
            None => groups.push((day, vec![item])),
        }
    }
    groups
}

/// 不分大小寫找 key（ASCII）在 text 裡的位置（C# IndexOf OrdinalIgnoreCase）。
fn find_ci(text: &str, key: &str) -> Option<usize> {
    if key.is_empty() {
        return Some(0);
    }
    text.char_indices()
        .map(|(i, _)| i)
        .find(|&i| text.get(i..i + key.len()).is_some_and(|s| s.eq_ignore_ascii_case(key)))
}

/// 文字裡的任務編號做成連結：回傳 (文字, 任務編號；空字串是一般文字)。is_project：代碼是不是專案（例「ISMS-P-13」不是）。
pub(crate) fn link_spans(text: &str, is_project: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while let Some(key) = KeyFinder::parse(Some(rest))
        && let Some(i) = find_ci(rest, &key)
    {
        let end = i + key.len();
        if !is_project(&key) {
            // 不是專案代碼：當一般文字
            plain.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        plain.push_str(&rest[..i]);
        if !plain.is_empty() {
            spans.push((std::mem::take(&mut plain), String::new()));
        }
        spans.push((rest[i..end].to_owned(), key));
        rest = &rest[end..];
    }
    plain.push_str(rest);
    if !plain.is_empty() {
        spans.push((plain, String::new()));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn range_本週上週最近30天() {
        let thu = ymd(2026, 11, 12);
        assert_eq!((ymd(2026, 11, 9), thu), range("week", thu));
        assert_eq!((ymd(2026, 11, 2), ymd(2026, 11, 8)), range("lastWeek", thu));
        assert_eq!((ymd(2026, 10, 14), thu), range("month", thu));
        let sun = ymd(2026, 11, 15);
        assert_eq!((ymd(2026, 11, 9), sun), range("week", sun));
    }

    #[test]
    fn link_spans_任務編號變連結() {
        let spans = link_spans("Amy 開始做 f2p-97 報表、完成 ISMS-P-13", |k| k.starts_with("F2P"));
        let expected: Vec<(String, String)> = vec![
            ("Amy 開始做 ".into(), "".into()),
            ("f2p-97".into(), "F2P-97".into()),
            (" 報表、完成 ISMS-P-13".into(), "".into()),
        ];
        assert_eq!(expected, spans);
        assert_eq!(vec![("沒有編號".to_owned(), String::new())], link_spans("沒有編號", |_| true));
    }

    #[test]
    fn group_by_day_與摘要() {
        let item = |at: &str| ActivityItem { at: at.into(), project: "F2P".into(), text: "x".into() };
        let items = vec![item("11/12 09:00"), item("11/11 18:00"), item("11/12 08:00")];
        let groups = group_by_day(&items);
        assert_eq!(2, groups.len());
        assert_eq!("11/12", groups[0].0);
        assert_eq!(2, groups[0].1.len());
        let feed = ActivityFeed {
            from: ymd(2026, 11, 9),
            to: ymd(2026, 11, 12),
            created: 1,
            started: 0,
            completed: 0,
            overdue: 0,
            top_members: vec![("Amy".into(), 3), ("Bob".into(), 1)],
            items,
        };
        assert_eq!("本週（11/9～11/12）的進展　最活躍：Amy 3、Bob 1", top_text(&feed, "本週"));
    }
}
