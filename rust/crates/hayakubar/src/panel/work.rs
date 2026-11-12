//! 對應 C# `UI/PanelWindow.Work.cs`：面板的任務分頁、任務選單（右鍵或 ⋯）、通知篩選與稍後／封存、今天分頁的摘要。
//! （開始任務的遠端搜尋在 mod.rs 的 filter_start／search_remote。）

use chrono::{Local, NaiveDate};
use hb_api::{KeyFinder, NotificationItem, TaskSummary};
use hb_core::TimerEngine;
use hb_ui_common::view_models::{ColorKey, NotificationRowVm};
use hb_ui_common::{KeyCopy, TaskText};
use hb_ui_work::PageArg;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::{PanelInner, badge_text, spawn, with_panel};
use crate::menu::{self, MenuEntry, MenuPlace};
use crate::ui::{FilterChip, MyTaskRow, NotifRow};

/// 面板「任務」分頁的一列（C# MyTaskRowVm）。
#[derive(Debug, Clone, PartialEq)]
pub struct MyTaskRowVm {
    pub item: TaskSummary,
    pub running: bool,
    pub busy: bool,
}

impl MyTaskRowVm {
    pub fn title(&self) -> String {
        TaskText::title_of(&self.item)
    }

    pub fn due_text_on(&self, today: NaiveDate) -> String {
        let Some(end) = self.item.planned_end else { return String::new() };
        let days = (end - today).num_days();
        if days < 0 {
            format!("· 逾期 {} 天", -days)
        } else if days == 0 {
            "· 今天到期".to_owned()
        } else if days == 1 {
            "· 明天到期".to_owned()
        } else {
            format!("· {} 到期", end.format("%-m/%-d"))
        }
    }

    pub fn due_color_on(&self, today: NaiveDate) -> ColorKey {
        let days = self.item.planned_end.map_or(99, |end| (end - today).num_days());
        if days < 0 {
            ColorKey::Error
        } else if days <= 1 {
            ColorKey::Warn
        } else {
            ColorKey::Muted
        }
    }

    pub fn action_glyph(&self) -> &'static str {
        if self.busy {
            "…"
        } else if self.running {
            "❚❚"
        } else {
            "▶"
        }
    }

    pub fn action_tip(&self) -> &'static str {
        if self.running { "暫停計時" } else { "開始計時" }
    }

    pub fn action_color(&self) -> ColorKey {
        if self.running { ColorKey::Run } else { ColorKey::Muted }
    }

    fn row(&self, today: NaiveDate) -> MyTaskRow {
        MyTaskRow {
            key: self.item.key.as_str().into(),
            title: self.title().into(),
            status: self.item.status.as_str().into(),
            due_text: self.due_text_on(today).into(),
            due_color: self.due_color_on(today).code(),
            action_glyph: self.action_glyph().into(),
            action_tip: self.action_tip().into(),
            action_color: self.action_color().code(),
            not_busy: !self.busy,
        }
    }
}

/// 通知的分類標籤（key、文字）。
pub const NOTIFY_FILTERS: [(&str, &str); 6] = [
    ("all", "全部"),
    ("mention", "@提及"),
    ("discussions", "討論"),
    ("assigned", "指派給我"),
    ("reminder", "提醒"),
    ("system", "系統"),
];

const MENTION_TYPES: [&str; 6] =
    ["MENTIONED", "DOC_MENTIONED", "THREAD_REPLY", "CHANNEL_MESSAGE", "COMMENT_ON_MY_TASK", "CHAT_REACTION"];

const ASSIGNED_TYPES: [&str; 7] = [
    "TASK_ASSIGNED",
    "DOC_TODO_ASSIGNED",
    "WATCHER_ADDED",
    "ADDED_TO_PROJECT",
    "MEETING_INVITED",
    "CHANGE_REQUESTED",
    "CHANGE_REVIEWED",
];

const REMINDER_TYPES: [&str; 10] = [
    "TASK_REMINDER",
    "TASK_DUE_SOON",
    "TASK_OVERDUE",
    "MEETING_STARTING",
    "TIMER_RESUME_PROMPT",
    "HOURS_UNFILLED",
    "HOURS_UNFILLED_WEEKLY",
    "MILESTONE_DUE_SOON",
    "POLL_CLOSING",
    "DAILY_DIGEST",
];

/// 通知屬於哪一類：mention、assigned、reminder，其他是 system（C# CategoryOf）。
pub fn category_of(n: &NotificationItem) -> &'static str {
    let t = n.r#type.as_str();
    if MENTION_TYPES.contains(&t) {
        "mention"
    } else if ASSIGNED_TYPES.contains(&t) {
        "assigned"
    } else if REMINDER_TYPES.contains(&t) {
        "reminder"
    } else {
        "system"
    }
}

/// 任務分頁空的時候顯示的文字。
pub fn empty_tasks_text(loaded: bool, tab: &str) -> &'static str {
    if !loaded {
        return "讀取中…";
    }
    match tab {
        "overdue" => "沒有逾期的任務 🎉",
        "should_start" => "沒有該開始還沒開始的任務",
        "test" => "沒有指派給你的測試任務或測試結果",
        _ => "這裡沒有任務",
    }
}

/// 通知分頁空的時候顯示的文字。
pub fn empty_notify_text(discussions: bool, error: Option<&str>, loading: bool, rows: usize) -> String {
    if !discussions {
        return "沒有通知".to_owned();
    }
    match error {
        Some(e) => e.to_owned(),
        None if loading && rows == 0 => "讀取中…".to_owned(),
        None => "沒有討論".to_owned(),
    }
}

impl PanelInner {
    /// C# InitWork：任務分頁、通知分類的篩選標籤。
    pub(super) fn init_work(&self) {
        let chips: Vec<FilterChip> = TimerEngine::TASK_VIEW_TABS
            .iter()
            .map(|(_, label)| FilterChip { label: (*label).into(), shown: true, tip: SharedString::new() })
            .collect();
        self.ui.set_task_filters(ModelRc::new(VecModel::from(chips)));
        let chips: Vec<FilterChip> = NOTIFY_FILTERS
            .iter()
            .map(|(key, label)| FilterChip {
                label: (*label).into(),
                shown: true,
                tip: if *key == "discussions" {
                    "團隊空間的評論和聊天裡 @ 你的、你負責的頁、你參與的討論串，同一段討論一列".into()
                } else {
                    SharedString::new()
                },
            })
            .collect();
        self.ui.set_notify_filters(ModelRc::new(VecModel::from(chips)));
        self.ui.set_task_filter(
            TimerEngine::TASK_VIEW_TABS.iter().position(|(t, _)| *t == TimerEngine::ALL_TAB).unwrap_or(0) as i32,
        );
        self.ui.set_notify_filter(0);

        let ui = &self.ui;
        ui.on_task_filter_selected(with_panel!(self.me, |p, i| {
            if let Some((tab, _)) = TimerEngine::TASK_VIEW_TABS.get(i as usize) {
                p.s.borrow_mut().task_tab = (*tab).to_owned();
                p.ui.set_task_filter(i);
                p.render_tasks();
            }
        }));
        ui.on_leaf_only_toggled(with_panel!(self.me, |p, v| {
            p.engine.config_mut().task_leaf_only = v;
            p.engine.config().save();
            p.render_tasks();
        }));
        ui.on_my_task_open(with_panel!(self.me, |p, i| {
            if let Some(key) = p.my_task_key(i) {
                p.request_workspace("tasks", Some(PageArg::Text(key)));
            }
        }));
        // 點編號是複製編號，點其他地方才打開任務（跟網頁一樣）
        ui.on_my_task_copy(with_panel!(self.me, |p, i| {
            if let Some(key) = p.my_task_key(i) {
                KeyCopy::copy(&p.engine, &key);
            }
        }));
        ui.on_my_task_act(with_panel!(self.me, |p, i| p.my_task_action(i as usize)));
        ui.on_my_task_more(with_panel!(self.me, |p, i, x, y| {
            if let Some(key) = p.my_task_key(i) {
                let anchor = menu::to_screen(p.ui.window(), x, y, 1.0, 1.0);
                p.show_task_menu(key, anchor);
            }
        }));
        ui.on_my_task_status(with_panel!(self.me, |p, i, x, y| {
            if let Some(key) = p.my_task_key(i) {
                let anchor = menu::to_screen(p.ui.window(), x, y, 1.0, 1.0);
                p.show_status_menu(key, anchor);
            }
        }));
        ui.on_all_tasks(with_panel!(self.me, |p| {
            let tab = p.s.borrow().task_tab.clone();
            p.request_workspace("tasks", Some(PageArg::Text(tab)));
        }));
        ui.on_new_task(with_panel!(self.me, |p| p.request_workspace("tasks", Some(PageArg::Text("new".into())))));
        // 今天分頁的摘要
        ui.on_due_clicked(with_panel!(self.me, |p| {
            let target = if p.engine.overdue_count() > 0 { "overdue" } else { "in_progress" };
            p.request_workspace("tasks", Some(PageArg::Text(target.into())));
        }));
        ui.on_new_meeting(with_panel!(self.me, |p| p.request_workspace("meetings", Some(PageArg::Text("new".into())))));
        ui.on_new_reminder(with_panel!(self.me, |p| p.request_workspace("reminders", None)));
        ui.on_work_log(with_panel!(self.me, |p| {
            let yesterday = Local::now().date_naive().pred_opt().unwrap_or_else(|| Local::now().date_naive());
            p.request_workspace("worklog", Some(PageArg::Date(yesterday)));
        }));
        ui.on_workspace(with_panel!(self.me, |p| p.request_workspace("tasks", None)));
        // 通知
        ui.on_notify_filter_selected(with_panel!(self.me, |p, i| {
            let Some((key, _)) = NOTIFY_FILTERS.get(i as usize) else { return };
            p.s.borrow_mut().notify_filter = (*key).to_owned();
            p.ui.set_notify_filter(i);
            p.render_notifications();
            if *key == "discussions" {
                p.load_discussions(false);
            }
        }));
        ui.on_discussion_source_selected(with_panel!(self.me, |p, i| {
            p.s.borrow_mut().discussion_source = match i {
                1 => Some("space".to_owned()),
                2 => Some("chat".to_owned()),
                _ => None,
            };
            p.ui.set_discussion_source(i);
            if p.s.borrow().notify_filter == "discussions" {
                p.load_discussions(false);
            }
        }));
        ui.on_discussion_filter(with_panel!(self.me, |p, unresolved| {
            if unresolved {
                p.load_discussions(false); // 未解決是伺服器篩的，要重讀
            } else {
                p.render_notifications();
            }
        }));
        ui.on_notif_snooze(with_panel!(self.me, |p, i, x, y| p.notification_snooze(i as usize, x, y)));
        ui.on_notif_archive(with_panel!(self.me, |p, i| p.notification_archive(i as usize)));
    }

    fn my_task_key(&self, index: i32) -> Option<String> {
        self.s.borrow().my_tasks.get(index as usize).map(|t| t.item.key.clone())
    }

    // ───────────── 任務分頁 ─────────────

    pub(super) fn render_tasks(&self) {
        let engine = &self.engine;
        let running = engine.running_keys();
        let leaf_only = engine.config().task_leaf_only;
        self.ui.set_leaf_only(leaf_only);
        let task_tab = self.s.borrow().task_tab.clone();
        let chips: Vec<FilterChip> = TimerEngine::TASK_VIEW_TABS
            .iter()
            .map(|(tab, label)| {
                let count = engine.task_list(tab).iter().filter(|t| !leaf_only || t.is_leaf()).count();
                FilterChip {
                    label: if count > 0 { format!("{label} {count}") } else { (*label).to_owned() }.into(),
                    // 測試分頁沒有東西時收起來（面板很窄）；正選著的照樣顯示
                    shown: *tab != "test" || count > 0 || task_tab == *tab,
                    tip: SharedString::new(),
                }
            })
            .collect();
        self.ui.set_task_filters(ModelRc::new(VecModel::from(chips)));
        let all: Vec<TaskSummary> =
            engine.task_list(&task_tab).iter().filter(|t| !leaf_only || t.is_leaf()).cloned().collect();
        let rows: Vec<MyTaskRowVm> = all
            .iter()
            .take(30)
            .map(|t| MyTaskRowVm { running: running.contains(&t.key) || t.timing, item: t.clone(), busy: false })
            .collect();
        let today = Local::now().date_naive();
        self.ui.set_my_tasks(ModelRc::new(VecModel::from(rows.iter().map(|r| r.row(today)).collect::<Vec<_>>())));
        self.ui.set_my_task_empty(empty_tasks_text(!engine.task_lists().is_empty(), &task_tab).into());
        self.ui.set_more_tasks_text(
            if all.len() > rows.len() {
                format!("在工作台看全部 {} 個 ›", all.len())
            } else {
                "在工作台看全部 ›".to_owned()
            }
            .into(),
        );
        self.set_tab_badge(1, badge_text(engine.overdue_count()));
        self.s.borrow_mut().my_tasks = rows;
    }

    fn my_task_action(&self, index: usize) {
        let vm = {
            let mut s = self.s.borrow_mut();
            let Some(vm) = s.my_tasks.get_mut(index) else { return };
            vm.busy = true;
            vm.clone()
        };
        let model = self.ui.get_my_tasks();
        if index < model.row_count() {
            model.set_row_data(index, vm.row(Local::now().date_naive()));
        }
        let engine = self.engine.clone();
        spawn(async move {
            if vm.running {
                engine.pause_task(&vm.item.key).await;
            } else {
                engine.start_task(&vm.item.key).await;
            }
        });
    }

    // ───────────── 任務選單（右鍵或 ⋯） ─────────────

    /// 選單擺在按鈕右側（螢幕右邊放不下改放左側），不會蓋住其他列的 ⋯ 與狀態。
    pub(super) fn show_task_menu(&self, key: String, anchor: hb_win::Rect) {
        let engine = &self.engine;
        let running = engine.running_keys().contains(&key);
        let status = engine.status_of(&key);
        let pinned = engine.is_pinned(&key);
        let entries = vec![
            MenuEntry::item(if running { "❚❚  暫停計時" } else { "▶  開始計時" }),
            MenuEntry::item("✓  停止並完成"),
            MenuEntry::item_if(
                if pinned { "📌  取消釘選" } else { "📌  釘到托盤" },
                pinned || engine.pins().len() < TimerEngine::MAX_PINS,
            ),
            MenuEntry::item(match &status {
                None => "⇄  改狀態…".to_owned(),
                Some(s) => format!("⇄  改狀態…（{s}）"),
            }),
            MenuEntry::Separator,
            MenuEntry::item("⏰  新增提醒…"),
            MenuEntry::item("💬  任務聊天"),
            MenuEntry::item("☰  查看詳細"),
            MenuEntry::item("🔗  複製連結"),
            MenuEntry::item("↗  在網頁開啟"),
            MenuEntry::Separator,
            MenuEntry::item("🗑  刪除任務…"),
        ];
        let me = self.weak();
        menu::show_later(self.ui.as_weak(), entries, MenuPlace::RightOf(anchor), move |i| {
            let Some(p) = me.upgrade() else { return };
            let engine = p.engine.clone();
            let k = key.clone();
            match i {
                0 => spawn(async move {
                    if running {
                        engine.pause_task(&k).await;
                    } else {
                        engine.start_task(&k).await;
                    }
                }),
                1 => spawn(async move { engine.complete_task(&k, None).await }),
                2 => spawn(async move {
                    if pinned {
                        engine.unpin_task(&k).await;
                    } else {
                        engine.pin_task(&k).await;
                    }
                }),
                3 => p.show_status_menu(key, anchor),
                5 => p.request_workspace("reminders", Some(PageArg::Text(key))),
                6 => p.request_workspace("chat", Some(PageArg::Text(key))),
                7 => p.request_workspace("tasks", Some(PageArg::Text(key))),
                8 => KeyCopy::copy_link(&p.engine, &key),
                9 => hb_win::open(&p.engine.task_url(&key)),
                11 => p.delete_task(key),
                _ => {}
            }
        });
    }

    fn delete_task(&self, key: String) {
        let title = self
            .engine
            .my_tasks()
            .iter()
            .find(|t| hb_core::ci_set::eq_ic(&t.key, &key))
            .map(|t| t.title.clone())
            .unwrap_or_default();
        let me = self.weak();
        spawn(async move {
            let Some(p) = me.upgrade() else { return };
            p.s.borrow_mut().dialog_open = true;
            // 刪了就 AfterExternalChange（C# DeleteTaskAsync）
            let engine = p.engine.clone();
            let window = p.ui.as_weak();
            let owner = window.upgrade();
            hb_ui_common::delete_task::confirm(&engine, &key, &title, None, owner.as_ref().map(|w| w.window())).await;
            p.s.borrow_mut().dialog_open = false;
            if p.s.borrow().shown {
                hb_ui_common::window_util::activate(p.ui.window());
            }
        });
    }

    pub(super) fn show_status_menu(&self, key: String, anchor: hb_win::Rect) {
        let me = self.weak();
        let engine = self.engine.clone();
        spawn(async move {
            let project = match engine.get_project(KeyFinder::project_of(&key)).await {
                Ok(p) => p,
                Err(e) => {
                    log::error!("讀取狀態清單失敗: {e}");
                    return;
                }
            };
            let Some(p) = me.upgrade() else { return };
            let current = engine.status_of(&key);
            let mut entries = vec![MenuEntry::item_if(format!("{key} 改成…"), false)];
            let names: Vec<String> = project.statuses.iter().map(|s| s.name.clone()).collect();
            for name in &names {
                let mark = if current.as_deref() == Some(name.as_str()) { "● " } else { "    " };
                entries.push(MenuEntry::item(format!("{mark}{name}")));
            }
            menu::show_later(p.ui.as_weak(), entries, MenuPlace::RightOf(anchor), move |i| {
                if let Some(name) = i.checked_sub(1).and_then(|j| names.get(j)).cloned() {
                    let k = key.clone();
                    spawn(async move { engine.set_task_status(&k, &name).await });
                }
            });
        });
    }

    // ───────────── 今天分頁的摘要 ─────────────

    pub(super) fn render_today_extras(&self) {
        let engine = &self.engine;
        let due = engine.due_today_count();
        let overdue = engine.overdue_count();
        self.ui.set_due_none(due == 0 && overdue == 0);
        self.ui.set_due_count(due as i32);
        self.ui.set_overdue_count(overdue as i32);
        let leaves = engine.on_leave_today();
        self.ui.set_leave_text(leave_text(&leaves).into());
    }

    // ───────────── 通知篩選、稍後、封存 ─────────────

    pub(super) fn render_notifications(&self) {
        let engine = &self.engine;
        let notifications = engine.notifications();
        let discussions_list = engine.discussions();
        let (filter, loading, error) = {
            let s = self.s.borrow();
            (s.notify_filter.clone(), s.discussions_loading, s.discussions_error.clone())
        };
        let chips_model = self.ui.get_notify_filters();
        for (i, (key, label)) in NOTIFY_FILTERS.iter().enumerate() {
            if *key == "all" {
                continue;
            }
            let unread = if *key == "discussions" {
                discussions_list.iter().filter(|n| n.unread).count()
            } else {
                notifications.iter().filter(|n| n.unread && category_of(n) == *key).count()
            };
            if let Some(mut chip) = chips_model.row_data(i) {
                let text = if unread > 0 { format!("{label} {unread}") } else { (*label).to_owned() };
                if chip.label.as_str() != text {
                    chip.label = text.into();
                    chips_model.set_row_data(i, chip);
                }
            }
        }
        let discussions = filter == "discussions";
        let unread_only = self.ui.get_discussion_unread();
        let filtered: Vec<NotificationItem> = match filter.as_str() {
            "all" => notifications.iter().cloned().collect(),
            "discussions" => discussions_list.iter().filter(|n| !unread_only || n.unread).cloned().collect(),
            f => notifications.iter().filter(|n| category_of(n) == f).cloned().collect(),
        };
        let rows: Vec<NotificationRowVm> =
            filtered.into_iter().take(30).map(|n| NotificationRowVm::new(n, discussions)).collect();
        self.ui.set_notifications(ModelRc::new(VecModel::from(
            rows.iter()
                .map(|r| NotifRow {
                    label: r.label().into(),
                    count_text: r.count_text().into(),
                    text: r.text().into(),
                    text_color: r.text_color().code(),
                    meta: r.meta().into(),
                    unread: r.unread_visible(),
                    archiving: false,
                })
                .collect::<Vec<_>>(),
        )));
        self.ui.set_discussions_visible(discussions);
        self.ui.set_notify_empty(empty_notify_text(discussions, error.as_deref(), loading, rows.len()).into());
        self.ui.set_notify_empty_visible(rows.is_empty() || discussions && error.is_some());
        let unread = engine.unread_count();
        self.ui.set_unread_text(if unread > 0 { format!("{unread} 則未讀") } else { "都已讀".to_owned() }.into());
        self.ui.set_mark_all_enabled(unread > 0);
        self.set_tab_badge(3, badge_text(unread));
        self.s.borrow_mut().notifications = rows;
    }

    /// 討論分頁：點了才讀（一分鐘內不重讀），不加輪詢。
    pub(super) fn load_discussions(&self, force: bool) {
        {
            let mut s = self.s.borrow_mut();
            s.discussions_loading = true;
            s.discussions_error = None;
        }
        self.render_notifications();
        let unresolved = self.ui.get_discussion_unresolved();
        let source = self.s.borrow().discussion_source.clone();
        let engine = self.engine.clone();
        let me = self.weak();
        spawn(async move {
            let result = engine.load_discussions(unresolved, source.as_deref(), force).await;
            let Some(p) = me.upgrade() else { return };
            if let Err(e) = result {
                log::error!("讀取我的討論失敗: {e}");
                p.s.borrow_mut().discussions_error = Some(format!("讀不到討論：{}", e.message()));
            }
            p.s.borrow_mut().discussions_loading = false;
            p.render_notifications();
        });
    }

    fn notification_snooze(&self, index: usize, x: f32, y: f32) {
        let Some(item) = self.s.borrow().notifications.get(index).map(|n| n.item.clone()) else { return };
        const PRESETS: [(&str, &str); 3] =
            [("1h", "1 小時後"), ("next_workday", "下個工作日上班時"), ("next_week", "下週第一個上班日")];
        let entries: Vec<MenuEntry> = PRESETS.iter().map(|(_, label)| MenuEntry::item(*label)).collect();
        let anchor = menu::to_screen(self.ui.window(), x, y, 1.0, 1.0);
        let engine = self.engine.clone();
        menu::show_later(self.ui.as_weak(), entries, MenuPlace::Below(anchor), move |i| {
            if let Some((preset, _)) = PRESETS.get(i) {
                spawn(async move { engine.snooze_notification(&item, preset).await });
            }
        });
    }

    fn notification_archive(&self, index: usize) {
        let Some(item) = self.s.borrow().notifications.get(index).map(|n| n.item.clone()) else { return };
        let model = self.ui.get_notifications();
        if let Some(mut row) = model.row_data(index) {
            row.archiving = true;
            model.set_row_data(index, row);
        }
        let engine = self.engine.clone();
        spawn(async move { engine.archive_notification(&item).await });
    }
}

/// 「今天請假：王小明（特休）、李大華」。
pub fn leave_text(leaves: &[hb_api::TeamMember]) -> String {
    if leaves.is_empty() {
        return String::new();
    }
    let names: Vec<String> = leaves
        .iter()
        .map(|m| {
            if m.leaves.is_empty() {
                m.user.display_name.clone()
            } else {
                format!("{}（{}）", m.user.display_name, m.leaves.join("、"))
            }
        })
        .collect();
    format!("今天請假：{}", names.join("、"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use hb_api::{PersonRef, TeamMember};

    fn task(end: Option<NaiveDate>) -> MyTaskRowVm {
        MyTaskRowVm {
            item: TaskSummary::new("F2P-1", "名稱", "F2P", "進行中", "IN_PROGRESS", "", None, end, false, ""),
            running: false,
            busy: false,
        }
    }

    #[test]
    fn 到期文字與顏色() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let d = |m, day| Some(NaiveDate::from_ymd_opt(2026, m, day).unwrap());
        assert_eq!("", task(None).due_text_on(today));
        assert_eq!(ColorKey::Muted, task(None).due_color_on(today));
        assert_eq!("· 逾期 2 天", task(d(11, 10)).due_text_on(today));
        assert_eq!(ColorKey::Error, task(d(11, 10)).due_color_on(today));
        assert_eq!("· 今天到期", task(d(11, 12)).due_text_on(today));
        assert_eq!(ColorKey::Warn, task(d(11, 12)).due_color_on(today));
        assert_eq!("· 明天到期", task(d(11, 13)).due_text_on(today));
        assert_eq!(ColorKey::Warn, task(d(11, 13)).due_color_on(today));
        assert_eq!("· 11/20 到期", task(d(11, 20)).due_text_on(today));
        assert_eq!(ColorKey::Muted, task(d(11, 20)).due_color_on(today));
    }

    #[test]
    fn 任務列的按鈕() {
        let mut t = task(None);
        assert_eq!(("▶", "開始計時", ColorKey::Muted), (t.action_glyph(), t.action_tip(), t.action_color()));
        t.running = true;
        assert_eq!(("❚❚", "暫停計時", ColorKey::Run), (t.action_glyph(), t.action_tip(), t.action_color()));
        t.busy = true;
        assert_eq!("…", t.action_glyph());
    }

    fn notification(t: &str) -> NotificationItem {
        let at = chrono::FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 11, 12, 9, 0, 0).unwrap();
        NotificationItem::new("n1", t, "標籤", "內容", None, None, None, None, at, true, 1)
    }

    #[test]
    fn 通知分類() {
        assert_eq!("mention", category_of(&notification("MENTIONED")));
        assert_eq!("mention", category_of(&notification("CHAT_REACTION")));
        assert_eq!("assigned", category_of(&notification("TASK_ASSIGNED")));
        assert_eq!("assigned", category_of(&notification("CHANGE_REVIEWED")));
        assert_eq!("reminder", category_of(&notification("DAILY_DIGEST")));
        assert_eq!("reminder", category_of(&notification("HOURS_UNFILLED_WEEKLY")));
        assert_eq!("system", category_of(&notification("SOMETHING_ELSE")));
    }

    #[test]
    fn 空狀態文字() {
        assert_eq!("讀取中…", empty_tasks_text(false, "overdue"));
        assert_eq!("沒有逾期的任務 🎉", empty_tasks_text(true, "overdue"));
        assert_eq!("沒有該開始還沒開始的任務", empty_tasks_text(true, "should_start"));
        assert_eq!("沒有指派給你的測試任務或測試結果", empty_tasks_text(true, "test"));
        assert_eq!("這裡沒有任務", empty_tasks_text(true, "all"));
        assert_eq!("沒有通知", empty_notify_text(false, Some("x"), true, 0));
        assert_eq!("讀不到討論：x", empty_notify_text(true, Some("讀不到討論：x"), true, 0));
        assert_eq!("讀取中…", empty_notify_text(true, None, true, 0));
        assert_eq!("沒有討論", empty_notify_text(true, None, true, 3));
        assert_eq!("沒有討論", empty_notify_text(true, None, false, 0));
    }

    #[test]
    fn 今天請假文字() {
        let member = |name: &str, leaves: &[&str]| TeamMember {
            user: PersonRef::new("u", name),
            job_title: None,
            online: false,
            on_leave: true,
            leaves: leaves.iter().map(|s| s.to_string()).collect(),
            timing: vec![],
        };
        assert_eq!("", leave_text(&[]));
        assert_eq!(
            "今天請假：王小明（特休、病假）、李大華",
            leave_text(&[member("王小明", &["特休", "病假"]), member("李大華", &[])])
        );
    }
}
