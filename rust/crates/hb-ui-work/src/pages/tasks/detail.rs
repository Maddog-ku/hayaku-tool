//! 對應 C# `UI/Work/TasksPage.xaml.cs` 的「詳細」：彈窗裡直接改名稱、狀態、優先級、日期、預估工時；計時、完成、釘選、移動；
//! 子任務的工時、整棵子任務樹（展開才讀）、聊天。
//! 跟上 Hayaku 1.5.3／1.5.4（C# 版沒有）：這個任務的工時記錄（合計、本週、本月、每個人；按天分組，較早的按週、按月收起來，
//! 展開才用 list_time_entries 的 task 讀）；〔＋ 子任務〕旁的〔套用範本…〕。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use chrono::{Datelike, Local, NaiveDate, TimeDelta};
use hb_api::{KeyFinder, ProjectDetail, TaskDetail, TimeEntry, WorkMath};
use hb_ui_common::date_range::{from_int, to_int};
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use serde_json::{Map, Value, json};
use slint::SharedString;

use slint::ComponentHandle;

use super::Page;
use super::vm;
use crate::shared::{self, string_model, vec_model};
use crate::{PageArg, TasksPageData, TkChat, TkHourRow, TkLine, TkStat};

const PRIORITIES: [&str; 5] = ["緊急", "高", "中", "低", "無"];

/// 展開區塊的一行。
pub(super) fn line(kind: i32, text: impl Into<String>) -> TkLine {
    TkLine { kind, text: text.into().into(), ..Default::default() }
}

/// 子任務的計時段多長（分鐘）：有 duration 用它，否則開始到結束（還在計時就到現在）。
pub(super) fn entry_minutes(t: &TimeEntry, now: &hb_api::DateTimeOffset) -> f64 {
    if let Some(m) = t.duration_minutes {
        return m as f64;
    }
    match t.started_at {
        Some(s) => (t.ended_at.unwrap_or(*now) - s).num_milliseconds() as f64 / 60000.0,
        None => 0.0,
    }
}

/// 子任務工時的一段：「11/12 09:00–10:30　1 小時 30 分　amy　備註」。
pub(super) fn entry_text(t: &TimeEntry, now: &hb_api::DateTimeOffset) -> String {
    let start = t.started_at.map(|s| s.with_timezone(&Local).format("%-m/%-d %H:%M").to_string()).unwrap_or_default();
    let end = if t.running {
        "進行中".to_owned()
    } else {
        t.ended_at.map(|e| e.with_timezone(&Local).format("%H:%M").to_string()).unwrap_or_default()
    };
    let mut s = format!("{start}–{end}　{}", WorkMath::hours_minutes(entry_minutes(t, now)));
    if let Some(u) = t.user.as_deref().filter(|u| !u.is_empty()) {
        s += &format!("　{u}");
    }
    if let Some(n) = t.note.as_deref().filter(|n| !n.is_empty()) {
        s += &format!("　{n}");
    }
    s
}

/// 工時記錄的一段：日期（with_date）、起訖、時長、誰、備註。
pub(super) fn entry_line(t: &TimeEntry, now: &hb_api::DateTimeOffset, with_date: bool) -> (String, String) {
    let fmt = if with_date { "%-m/%-d %H:%M" } else { "%H:%M" };
    let start = t.started_at.map(|s| s.with_timezone(&Local).format(fmt).to_string()).unwrap_or_default();
    let end = if t.running {
        "進行中".to_owned()
    } else {
        t.ended_at.map(|e| e.with_timezone(&Local).format("%H:%M").to_string()).unwrap_or_default()
    };
    let mut text = format!("{start}–{end}");
    if let Some(u) = t.user.as_deref().filter(|u| !u.is_empty()) {
        text += &format!("　{u}");
    }
    if let Some(n) = t.note.as_deref().filter(|n| !n.is_empty()) {
        text += &format!("　{n}");
    }
    (text, WorkMath::hours_minutes(entry_minutes(t, now)))
}

/// 工時記錄的摘要：合計、本週（週一起）、本月，以及每個人（時數多的在前；沒有名字的只算進合計）。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HoursSummary {
    pub total: f64,
    pub week: f64,
    pub month: f64,
    pub people: Vec<(String, f64)>,
}

fn local_date(t: &TimeEntry) -> Option<NaiveDate> {
    t.started_at.map(|s| s.with_timezone(&Local).date_naive())
}

fn week_start(d: NaiveDate) -> NaiveDate {
    d - TimeDelta::days(i64::from(d.weekday().num_days_from_monday()))
}

pub(super) fn hours_summary(entries: &[TimeEntry], now: &hb_api::DateTimeOffset) -> HoursSummary {
    let today = now.with_timezone(&Local).date_naive();
    let (week, month) = (week_start(today), today.with_day(1).unwrap_or(today));
    let sum = |from: Option<NaiveDate>| -> f64 {
        entries
            .iter()
            .filter(|t| from.is_none_or(|f| local_date(t).is_some_and(|d| d >= f)))
            .map(|t| entry_minutes(t, now))
            .sum()
    };
    let mut people: Vec<(String, f64)> = Vec::new();
    for t in entries {
        let Some(user) = t.user.as_deref().filter(|u| !u.is_empty()) else { continue };
        match people.iter_mut().find(|p| p.0 == user) {
            Some(p) => p.1 += entry_minutes(t, now),
            None => people.push((user.to_owned(), entry_minutes(t, now))),
        }
    }
    people.sort_by(|a, b| b.1.total_cmp(&a.1));
    HoursSummary { total: sum(None), week: sum(Some(week)), month: sum(Some(month)), people }
}

/// 工時記錄的一組：本週的每一天各一組（展開）；再往前四週按週（收起）；更早的按月（收起）。新的在前，組內也是新的在前。
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HoursGroup {
    pub label: String,
    pub minutes: f64,
    pub expanded: bool,
    /// true：組內每段要寫日期（按週、按月的組）
    pub with_date: bool,
    pub entries: Vec<TimeEntry>,
}

pub(super) fn hours_groups(entries: &[TimeEntry], now: &hb_api::DateTimeOffset) -> Vec<HoursGroup> {
    let today = now.with_timezone(&Local).date_naive();
    let this_week = week_start(today);
    let weeks_from = this_week - TimeDelta::days(28);
    // 排序鍵：(0 天 1 週 2 月 反過來比較用的日期)；同一組的鍵一樣
    let mut groups: BTreeMap<std::cmp::Reverse<NaiveDate>, HoursGroup> = BTreeMap::new();
    for t in entries {
        let Some(d) = local_date(t) else { continue };
        let (key, label, expanded, with_date) = if d >= this_week {
            let label = if d == today {
                "今天".to_owned()
            } else if Some(d) == today.pred_opt() {
                "昨天".to_owned()
            } else {
                super::super::hours::w2_common::md_weekday(d)
            };
            (d, label, true, false)
        } else if d >= weeks_from {
            let w = week_start(d);
            let end = w + TimeDelta::days(6);
            (w, format!("{}/{}–{}/{} 那週", w.month(), w.day(), end.month(), end.day()), false, true)
        } else {
            let m = d.with_day(1).unwrap_or(d);
            (m, format!("{} 年 {} 月", m.year(), m.month()), false, true)
        };
        let g = groups.entry(std::cmp::Reverse(key)).or_insert_with(|| HoursGroup {
            label,
            minutes: 0.0,
            expanded,
            with_date,
            entries: Vec::new(),
        });
        g.minutes += entry_minutes(t, now);
        g.entries.push(t.clone());
    }
    groups
        .into_values()
        .map(|mut g| {
            g.entries.sort_by_key(|t| std::cmp::Reverse(t.started_at));
            g
        })
        .collect()
}

/// 工時記錄攤平成畫面上的列：組的標題＋展開的組裡每一段。
pub(super) fn hours_rows(groups: &[HoursGroup], now: &hb_api::DateTimeOffset) -> Vec<TkHourRow> {
    let mut rows = Vec::new();
    for (i, g) in groups.iter().enumerate() {
        rows.push(TkHourRow {
            header: true,
            group: i as i32,
            label: format!("{}（{} 段）", g.label, g.entries.len()).into(),
            meta: WorkMath::hours_minutes(g.minutes).into(),
            expanded: g.expanded,
        });
        if g.expanded {
            for t in &g.entries {
                let (text, minutes) = entry_line(t, now, g.with_date);
                rows.push(TkHourRow {
                    header: false,
                    group: i as i32,
                    label: text.into(),
                    meta: minutes.into(),
                    expanded: false,
                });
            }
        }
    }
    rows
}

/// 工時記錄（展開才讀）：讀的是哪個任務、讀好的分組（點標題展開／收起不重讀）。
#[derive(Default)]
struct HoursState {
    key: Option<String>,
    groups: Vec<HoursGroup>,
    version: u64,
}

thread_local! {
    static HOURS: RefCell<HoursState> = RefCell::new(HoursState::default());
}

/// 實際日期：「2026-11-01 → 進行中」；都沒有是「—」。
pub(super) fn actual_text(d: &TaskDetail) -> String {
    if d.actual_start.is_none() && d.actual_end.is_none() {
        return "—".to_owned();
    }
    let end = match d.actual_end {
        Some(e) => inputs::date_text(Some(e)),
        None => "進行中".to_owned(),
    };
    format!("{} → {end}", inputs::date_text(d.actual_start))
}

/// 預估工時欄填的字：空白＝清掉（Ok(None)）；不是 0～1666 的數字回 Err。
pub(super) fn parse_estimate(text: &str) -> Result<Option<f64>, ()> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    match t.parse::<f64>() {
        Ok(h) if h.is_finite() && (0.0..=1666.0).contains(&h) => Ok(Some(h)),
        _ => Err(()),
    }
}

impl Page {
    pub(super) fn wire_detail(self: &Rc<Self>, d: &TasksPageData) {
        use super::on;
        on!(d, self, on_d_title_commit, |p, t| p.commit_title(t.to_string()));
        on!(d, self, on_d_status_selected, |p, i| p.status_selected(i as usize));
        on!(d, self, on_d_priority_selected, |p, i| p.priority_selected(i as usize));
        on!(d, self, on_d_dates_applied, |p| p.dates_applied());
        on!(d, self, on_d_estimate_commit, |p, t| p.commit_estimate(t.to_string()));
        on!(d, self, on_d_timer, |p| shared::spawn("計時", async move { p.timer_clicked().await }));
        on!(d, self, on_d_complete, |p| shared::spawn("停止並完成", async move { p.complete_clicked().await }));
        on!(d, self, on_d_remind, |p| p.remind());
        on!(d, self, on_d_new_child, |p| {
            let key = p.st.borrow().detail.as_ref().map(|d| d.key.clone());
            p.show_create(key);
        });
        on!(d, self, on_d_pin, |p| shared::spawn("釘選", async move { p.pin_clicked().await }));
        on!(d, self, on_d_move, |p| shared::spawn("移動任務", async move { p.move_clicked().await }));
        on!(d, self, on_d_delete, |p| shared::spawn("刪除任務", async move { p.delete_clicked().await }));
        on!(d, self, on_d_open_web, |p| {
            if let Some(key) = p.st.borrow().detail.as_ref().map(|d| d.key.clone()) {
                hb_win::open(&p.engine().task_url(&key));
            }
        });
        on!(d, self, on_d_category_toggled, |p, v| {
            let what = if v { "成分類任務" } else { "成一般任務" };
            p.spawn_update(json!({ "isCategory": v }), what.to_owned());
        });
        on!(d, self, on_d_open, |p, key| p.open_key(&key));
        on!(d, self, on_d_ch_toggled, |p, open| {
            if open {
                shared::spawn("讀取子任務的工時", async move { p.load_child_hours().await });
            }
        });
        on!(d, self, on_d_apply_template, |p| p.apply_template());
        // 工時記錄：每次展開都重讀（使用者按了才讀，不預先讀）；點組的標題只在本機展開／收起
        on!(d, self, on_d_hours_toggled, |p, open| {
            if open {
                shared::spawn("讀取任務的工時", async move { p.load_hours().await });
            }
        });
        on!(d, self, on_d_hours_group_toggled, |p, i| p.toggle_hours_group(i));
        on!(d, self, on_d_tree_toggled, |p, open| {
            if open {
                shared::spawn("讀取子任務樹", async move { p.load_subtree().await });
            }
        });
        on!(d, self, on_d_chat_send, |p| shared::spawn("傳訊息", async move { p.send_chat().await }));
        on!(d, self, on_d_poll, |p, id| {
            let owner = p.ctx.window();
            hb_ui_common::poll::show(p.engine(), Some(&id), owner.as_ref().map(|w| w.window()));
        });
        self.wire_testing(d);
        self.wire_extras(d);
        self.wire_note(d);
    }

    /// 詳細的訊息列（C# Inputs.Show(DetailStatus, …)）。
    pub(super) fn show_msg(&self, msg: Option<StatusMessage>) {
        let (text, tone) = shared::status_parts(msg.as_ref());
        self.with_data(|d| {
            d.set_d_msg(text);
            d.set_d_msg_tone(tone);
        });
    }

    /// 打開任務的詳細（C# OpenTaskAsync）。
    pub(super) async fn open_task(self: Rc<Self>, key: String) {
        let version = {
            let mut st = self.st.borrow_mut();
            st.detail_version += 1;
            st.detail_version
        };
        self.with_data(|d| {
            d.set_dialog(1);
            d.set_detail_ready(false);
            d.set_detail_empty("讀取中…".into());
            d.set_detail_serial(d.get_detail_serial() + 1);
        });
        let engine = self.engine().clone();
        let result: Result<(TaskDetail, Rc<ProjectDetail>), String> = async {
            let detail = engine.api().get_task(&key).await.map_err(|e| e.0)?;
            let project =
                engine.get_project(KeyFinder::project_of(&detail.key)).await.map_err(|e| e.message().to_owned())?;
            Ok((detail, project))
        }
        .await;
        if version != self.st.borrow().detail_version {
            return;
        }
        match result {
            Ok((detail, project)) => {
                let key = detail.key.clone();
                self.show_detail(detail, &project);
                self.load_chat(&key, version).await;
            }
            Err(message) => {
                log::error!("讀取任務 {key} 失敗: {message}");
                self.with_data(|d| d.set_detail_empty(format!("讀不到 {key}：{message}").into()));
            }
        }
    }

    pub(super) fn show_detail(&self, d: TaskDetail, project: &ProjectDetail) {
        self.st.borrow_mut().loading_detail = true;
        let reset_child_hours = self.st.borrow().child_hours_for.as_deref() != Some(d.key.as_str());
        let reset_tree = self.st.borrow().tree_for.as_deref() != Some(d.key.as_str());
        let reset_hours = HOURS.with(|h| {
            let mut h = h.borrow_mut();
            let reset = h.key.as_deref() != Some(d.key.as_str());
            if reset {
                *h = HoursState { version: h.version + 1, ..HoursState::default() };
            }
            reset
        });
        let mut statuses: Vec<String> = project.statuses.iter().map(|s| s.name.clone()).collect();
        if !statuses.contains(&d.status) {
            statuses.insert(0, d.status.clone());
        }
        let status_index = statuses.iter().position(|s| *s == d.status).map_or(-1, |i| i as i32);
        let (logged, over, tip) = vm::logged_text(d.logged_minutes, &d.logged_text, d.estimated_hours, d.is_category);
        let path = if d.path.is_empty() {
            project.name.clone()
        } else {
            d.path.iter().map(|(k, t)| format!("{k} {t}")).collect::<Vec<_>>().join(" › ")
        };
        let key_text = if d.path.is_empty() { d.key.clone() } else { format!("{} · {}", d.key, project.name) };
        let approval = if d.approval_state == "PENDING" {
            "這個任務還在等管理者核准（可以先計時）。".to_owned()
        } else if d.pending_change_requests > 0 {
            format!("有 {} 個變更申請在等審核。", d.pending_change_requests)
        } else {
            String::new()
        };
        let assignees = if d.assignees.is_empty() {
            "—".to_owned()
        } else {
            d.assignees.iter().map(|a| a.display_name.as_str()).collect::<Vec<_>>().join("、")
        };
        self.with_data(|x| {
            x.set_detail_ready(true);
            x.set_d_msg(SharedString::new());
            x.set_d_path(path.into());
            x.set_d_key_text(key_text.into());
            x.set_d_title(d.title.as_str().into());
            x.set_d_approval(approval.into());
            x.set_d_status_items(string_model(&statuses));
            x.set_d_status_index(status_index);
            x.set_d_priority_index(PRIORITIES.iter().position(|p| *p == d.priority).map_or(-1, |i| i as i32));
            x.set_d_plan_start(to_int(d.planned_start));
            x.set_d_plan_end(to_int(d.planned_end));
            x.set_d_estimate(d.estimated_hours.map(vm::hours_0_00).unwrap_or_default().into());
            x.set_d_logged(logged.into());
            x.set_d_logged_over(over);
            x.set_d_logged_tip(tip.into());
            x.set_d_actual(actual_text(&d).into());
            x.set_d_assignees(assignees.into());
            x.set_d_category(d.is_category);
            x.set_d_note(d.note.clone().unwrap_or_default().into());
            x.set_d_note_visible(d.note.as_deref().is_some_and(|n| !n.trim().is_empty()));
            if reset_child_hours {
                x.set_d_ch_expanded(false);
                x.set_d_ch_lines(vec_model(Vec::new()));
                x.set_d_ch_header("子任務的工時".into());
            }
            if reset_tree {
                x.set_d_tree_expanded(false);
                x.set_d_tree_lines(vec_model(Vec::new()));
                x.set_d_tree_header("整棵子任務樹".into());
            }
            if reset_hours {
                x.set_d_hours_expanded(false);
                x.set_d_hours_header("工時記錄".into());
                x.set_d_hours_stats(vec_model(Vec::new()));
                x.set_d_hours_people(vec_model(Vec::new()));
                x.set_d_hours_rows(vec_model(Vec::new()));
                x.set_d_hours_message(SharedString::new());
                x.set_d_hours_error(false);
            }
        });
        {
            let mut st = self.st.borrow_mut();
            if reset_child_hours {
                st.child_hours_for = None;
            }
            if reset_tree {
                st.tree_for = None;
            }
            st.detail = Some(d.clone());
        }
        self.render_testing(&d);
        self.reset_note_append(&d);
        self.reset_extras(&d);
        self.render_timer_button();
        self.st.borrow_mut().loading_detail = false;
    }

    pub(super) fn render_timer_button(&self) {
        let Some(d) = self.st.borrow().detail.clone() else { return };
        let engine = self.engine();
        let running = engine.running_keys().contains(&d.key);
        let pinned = engine.is_pinned(&d.key);
        self.with_data(|x| {
            x.set_d_timer_text(if running { "❚❚ 暫停計時" } else { "▶ 開始計時" }.into());
            x.set_d_timer_primary(!running);
            x.set_d_timer_enabled(running || !d.is_category);
            x.set_d_timer_tip(if d.is_category && !running { "分類任務只用來分組，不能計時" } else { "" }.into());
            x.set_d_complete_enabled(!matches!(d.status_category.as_str(), "DONE" | "CANCELLED"));
            x.set_d_pin_text(if pinned { "📌 取消釘選" } else { "📌 釘選" }.into());
            x.set_d_pin_tip(
                if pinned {
                    "從面板的托盤拿掉"
                } else {
                    "釘到面板的托盤，一鍵切換正在做的任務"
                }
                .into(),
            );
            x.set_d_pin_enabled(pinned || !d.is_category);
        });
    }

    async fn load_chat(&self, key: &str, version: u64) {
        self.with_data(|d| {
            d.set_d_chat(vec_model(Vec::new()));
            d.set_d_chat_empty("讀取中…".into());
        });
        match self.engine().api().list_chat(Some(key), None, None).await {
            Ok(messages) => {
                if version != self.st.borrow().detail_version {
                    return;
                }
                let now = Local::now().fixed_offset();
                let rows: Vec<TkChat> = vm::chat_rows(&messages, &now)
                    .into_iter()
                    .map(|r| TkChat {
                        author: r.author.into(),
                        time: r.time.into(),
                        body: r.body.into(),
                        poll_id: r.poll_id.unwrap_or_default().into(),
                    })
                    .collect();
                let empty = if messages.is_empty() { "還沒有訊息" } else { "" };
                self.with_data(|d| {
                    d.set_d_chat(vec_model(rows));
                    d.set_d_chat_empty(empty.into());
                });
            }
            Err(e) => {
                log::error!("讀取 {key} 聊天失敗: {}", e.0);
                self.with_data(|d| d.set_d_chat_empty("聊天讀取失敗".into()));
            }
        }
    }

    fn spawn_update(&self, fields: Value, what: String) {
        let Some(p) = self.rc() else { return };
        let fields = fields.as_object().cloned().unwrap_or_default();
        shared::spawn("更新任務", async move { p.update(fields, &what).await });
    }

    /// 更新任務欄位；伺服器回的說明（例如送出變更申請）顯示在下方（C# UpdateAsync）。
    pub(super) async fn update(self: Rc<Self>, fields: Map<String, Value>, what: &str) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        if self.st.borrow().loading_detail {
            return;
        }
        let message: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
        let api = self.engine().api();
        let (m, k) = (message.clone(), key.clone());
        let ok = inputs::run(
            |_| {},
            |msg| self.show_msg(msg),
            move || async move {
                *m.borrow_mut() = api.update_task(&k, fields).await.map_err(ActionError::from)?;
                Ok(())
            },
            None,
        )
        .await;
        if !ok {
            self.clone().open_task(key).await;
            return;
        }
        let text = message.borrow().clone().filter(|m| !m.is_empty()).unwrap_or_else(|| format!("已更新{what}"));
        self.show_msg(Some(StatusMessage::ok(text)));
        self.engine().after_external_change().await;
        self.refresh_detail_quietly(&key).await;
    }

    /// 重讀詳細，訊息列保留（C# RefreshDetailQuietlyAsync）。
    pub(super) async fn refresh_detail_quietly(&self, key: &str) {
        let engine = self.engine().clone();
        let result: Result<(TaskDetail, Rc<ProjectDetail>), String> = async {
            let detail = engine.api().get_task(key).await.map_err(|e| e.0)?;
            let project = engine.get_project(KeyFinder::project_of(key)).await.map_err(|e| e.message().to_owned())?;
            Ok((detail, project))
        }
        .await;
        match result {
            Ok((detail, project)) => {
                if self.st.borrow().detail.as_ref().is_none_or(|d| d.key != key) {
                    return;
                }
                let saved = self.with_data(|d| (d.get_d_msg(), d.get_d_msg_tone()));
                self.show_detail(detail, &project);
                if let Some((text, tone)) = saved {
                    self.with_data(|d| {
                        d.set_d_msg(text);
                        d.set_d_msg_tone(tone);
                    });
                }
            }
            Err(message) => log::error!("重新讀取 {key} 失敗: {message}"),
        }
    }

    /// 名稱：Enter 或離開時存；空白就換回原本的。
    pub(super) fn commit_title(&self, text: String) {
        let title = text.trim().to_owned();
        let Some(current) = self.st.borrow().detail.as_ref().map(|d| d.title.clone()) else { return };
        if title.is_empty() || title == current {
            if title.is_empty() {
                self.with_data(|d| d.set_d_title(current.into()));
            }
            return;
        }
        self.spawn_update(json!({ "title": title }), "名稱".to_owned());
    }

    fn status_selected(&self, index: usize) {
        let Some(status) = self.with_data(|d| slint::Model::row_data(&d.get_d_status_items(), index)).flatten() else {
            return;
        };
        if self.st.borrow().detail.as_ref().is_none_or(|d| d.status == status.as_str()) {
            return;
        }
        self.spawn_update(json!({ "status": status.as_str() }), "狀態".to_owned());
    }

    fn priority_selected(&self, index: usize) {
        let Some(priority) = PRIORITIES.get(index) else { return };
        if self.st.borrow().detail.as_ref().is_none_or(|d| d.priority == *priority) {
            return;
        }
        self.spawn_update(json!({ "priority": priority }), "優先級".to_owned());
    }

    /// 日曆按了「套用日期」或「清除」：只送有變的那一邊。
    fn dates_applied(&self) {
        let Some(d) = self.st.borrow().detail.clone() else { return };
        let Some((start, end)) = self.with_data(|x| (from_int(x.get_d_plan_start()), from_int(x.get_d_plan_end())))
        else {
            return;
        };
        let mut fields = Map::new();
        if start != d.planned_start {
            fields.insert(
                "plannedStart".into(),
                start.map_or(Value::Null, |s| Value::String(inputs::date_text(Some(s)))),
            );
        }
        if end != d.planned_end {
            fields.insert("plannedEnd".into(), end.map_or(Value::Null, |e| Value::String(inputs::date_text(Some(e)))));
        }
        if !fields.is_empty() {
            self.spawn_update(Value::Object(fields), "預計日期".to_owned());
        }
    }

    /// 預估工時：離開欄位或 Enter 時存；不是 0～1666 的數字就換回原本的。
    pub(super) fn commit_estimate(&self, text: String) {
        let Some(current) = self.st.borrow().detail.as_ref().map(|d| d.estimated_hours) else { return };
        let hours = match parse_estimate(&text) {
            Ok(h) => h,
            Err(()) => {
                self.show_msg(Some(StatusMessage::error("預估工時請填小時數，例 4 或 1.5")));
                self.with_data(|d| d.set_d_estimate(current.map(vm::hours_0_00).unwrap_or_default().into()));
                return;
            }
        };
        if hours == current {
            return;
        }
        self.spawn_update(json!({ "estimatedHours": hours }), "預估工時".to_owned());
    }

    async fn timer_clicked(self: Rc<Self>) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        self.with_data(|d| d.set_d_timer_enabled(false));
        if self.engine().running_keys().contains(&key) {
            self.engine().pause_task(&key).await;
        } else {
            self.engine().start_task(&key).await;
        }
        self.with_data(|d| d.set_d_timer_enabled(true));
        self.show_msg(self.engine().error().map(StatusMessage::error));
        self.render_timer_button();
        self.render_list(true);
    }

    /// 子任務的狀態名稱是不是完成類（從專案的狀態清單查；查不到時用常見名稱判斷）。
    fn is_done_status(&self, status: &str) -> bool {
        if let Some(d) = self.st.borrow().detail.as_ref()
            && let Some(project) = self.engine().try_get_cached_project(KeyFinder::project_of(&d.key))
            && let Some(option) = project.statuses.iter().find(|s| s.name == status)
        {
            return matches!(option.category.as_str(), "DONE" | "CANCELLED");
        }
        matches!(status, "完成" | "已完成" | "不處理" | "取消" | "已取消")
    }

    async fn complete_clicked(self: Rc<Self>) {
        let Some(d) = self.st.borrow().detail.clone() else { return };
        let key = d.key.clone();
        let mut cascade: Option<bool> = None;
        let open_children = d.children.iter().filter(|c| !self.is_done_status(&c.2)).count();
        if open_children > 0 {
            let text = format!(
                "{key} 還有 {open_children} 個子任務沒完成。\n\n是：照專案的「任務狀態連動」設定（有開的話子任務一起完成）\n否：只完成 {key}，不帶動子任務與上層"
            );
            match self.confirm("停止並完成", &text, &["是", "否", "取消"]).await {
                0 => cascade = None,
                1 => cascade = Some(false),
                _ => return,
            }
        }
        self.with_data(|x| x.set_d_complete_enabled(false));
        self.engine().complete_task(&key, cascade).await;
        let msg = match self.engine().error() {
            Some(e) => StatusMessage::error(e),
            None => StatusMessage::ok("已停止並完成"),
        };
        self.show_msg(Some(msg));
        self.refresh_detail_quietly(&key).await;
    }

    async fn pin_clicked(self: Rc<Self>) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        self.with_data(|x| x.set_d_pin_enabled(false));
        if self.engine().is_pinned(&key) {
            self.engine().unpin_task(&key).await;
        } else {
            self.engine().pin_task(&key).await;
        }
        let msg = match self.engine().error() {
            Some(e) => StatusMessage::error(e),
            None if self.engine().is_pinned(&key) => StatusMessage::ok("已釘到托盤（面板的計時分頁最上面）"),
            None => StatusMessage::ok("已取消釘選"),
        };
        self.show_msg(Some(msg));
        self.render_timer_button();
    }

    /// 移動任務：同一個專案裡換上層或移到最上層（MCP 不能移到別的專案）。
    async fn move_clicked(self: Rc<Self>) {
        let Some(d) = self.st.borrow().detail.clone() else { return };
        let key = d.key.clone();
        let current = d.path.last().map(|p| p.0.clone()).filter(|p| !p.is_empty());
        let owner = self.ctx.window();
        let picked = hb_ui_common::move_task::pick_parent(
            self.engine(),
            &key,
            current.as_deref(),
            owner.as_ref().map(|w| w.window()),
        )
        .await;
        let Some(parent) = picked else { return };
        self.engine().move_task(&key, parent.as_deref()).await;
        let msg = match (self.engine().error(), &parent) {
            (Some(e), _) => StatusMessage::error(e),
            (None, None) => StatusMessage::ok("已移到最上層"),
            (None, Some(p)) => StatusMessage::ok(format!("已移到 {p} 底下")),
        };
        self.show_msg(Some(msg));
        self.refresh_detail_quietly(&key).await;
    }

    async fn delete_clicked(self: Rc<Self>) {
        let Some(d) = self.st.borrow().detail.clone() else { return };
        let owner = self.ctx.window();
        let ok = hb_ui_common::delete_task::confirm(
            self.engine(),
            &d.key,
            &d.title,
            Some(d.children.len() as i32),
            owner.as_ref().map(|w| w.window()),
        )
        .await;
        if !ok {
            return;
        }
        self.close_dialog();
        self.render_list(false);
    }

    fn remind(&self) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        self.close_dialog();
        self.ctx.show_page("reminders", Some(PageArg::Text(key)));
    }

    /// 展開「子任務的工時」才讀：每個直接子任務的計時段，依子任務列出，標題寫合計。
    async fn load_child_hours(self: Rc<Self>) {
        let Some(detail) = self.st.borrow().detail.clone() else { return };
        if self.st.borrow().child_hours_for.as_deref() == Some(detail.key.as_str()) {
            return;
        }
        self.st.borrow_mut().child_hours_for = Some(detail.key.clone());
        self.with_data(|d| d.set_d_ch_lines(vec_model(vec![line(2, "讀取中…")])));
        let api = self.engine().api();
        let mut groups: Vec<(String, String, Vec<TimeEntry>)> = Vec::new();
        for (key, title, _) in detail.children.iter().take(30) {
            match api.list_task_entries(key).await {
                Ok(entries) => groups.push((key.clone(), title.clone(), entries)),
                Err(e) => {
                    log::error!("讀取 {} 子任務的工時失敗: {}", detail.key, e.0);
                    self.with_data(|d| d.set_d_ch_lines(vec_model(vec![line(3, format!("讀取失敗：{}", e.0))])));
                    self.st.borrow_mut().child_hours_for = None;
                    return;
                }
            }
        }
        if self.st.borrow().detail.as_ref().is_none_or(|d| d.key != detail.key) {
            return;
        }
        let now = Local::now().fixed_offset();
        let total: f64 = groups.iter().flat_map(|g| g.2.iter()).map(|t| entry_minutes(t, &now)).sum();
        let header = format!("子任務的工時　合計 {}", WorkMath::hours_minutes(total))
            + if detail.children.len() > 30 { "（前 30 個子任務）" } else { "" };
        let mut lines: Vec<TkLine> = Vec::new();
        for (key, title, entries) in groups.iter().filter(|g| !g.2.is_empty()) {
            let sum: f64 = entries.iter().map(|t| entry_minutes(t, &now)).sum();
            lines.push(TkLine {
                kind: 5,
                key: key.as_str().into(),
                text: format!(" {title}").into(),
                meta: format!("　{}", WorkMath::hours_minutes(sum)).into(),
                ..Default::default()
            });
            let mut sorted: Vec<&TimeEntry> = entries.iter().collect();
            sorted.sort_by_key(|t| std::cmp::Reverse(t.started_at));
            for t in sorted {
                lines.push(TkLine { kind: 2, text: entry_text(t, &now).into(), depth: 1, ..Default::default() });
            }
        }
        if groups.iter().all(|g| g.2.is_empty()) {
            lines.push(line(2, "子任務都還沒有工時"));
        }
        self.with_data(|d| {
            d.set_d_ch_header(header.into());
            d.set_d_ch_lines(vec_model(lines));
        });
    }

    /// 〔套用範本…〕（跟上 Hayaku 1.5.4，C# 版只有右鍵選單）：套到這個任務底下，套了就重讀詳細（同右鍵選單）。
    fn apply_template(&self) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        let owner = self.ctx.window();
        let weak = self.me.clone();
        let k = key.clone();
        hb_ui_common::apply_template::open(
            self.engine(),
            KeyFinder::project_of(&key),
            vec![key.clone()],
            owner.as_ref().map(|w| w.window()),
            Box::new(move |applied| {
                if applied
                    && let Some(p) = weak.upgrade()
                    && p.st.borrow().detail.as_ref().is_some_and(|d| d.key == k)
                {
                    p.open_key(&k);
                }
            }),
        );
    }

    /// 展開「工時記錄」才讀（跟上 Hayaku 1.5.3）：list_time_entries 帶 task，每個人的都有。
    async fn load_hours(self: Rc<Self>) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        let version = HOURS.with(|h| {
            let mut h = h.borrow_mut();
            h.version += 1;
            h.key = Some(key.clone());
            h.version
        });
        self.with_data(|d| {
            d.set_d_hours_message("讀取中…".into());
            d.set_d_hours_error(false);
        });
        let result = self.engine().api().list_task_entries(&key).await;
        if HOURS.with(|h| h.borrow().version) != version
            || self.st.borrow().detail.as_ref().is_none_or(|d| d.key != key)
        {
            return;
        }
        let entries = match result {
            Ok(e) => e,
            Err(e) => {
                log::error!("讀取 {key} 的工時失敗: {}", e.0);
                self.with_data(|d| {
                    d.set_d_hours_message(format!("讀取失敗：{}", e.0).into());
                    d.set_d_hours_error(true);
                });
                return;
            }
        };
        let now = Local::now().fixed_offset();
        let summary = hours_summary(&entries, &now);
        let groups = hours_groups(&entries, &now);
        let rows = hours_rows(&groups, &now);
        let stats = vec![
            TkStat { label: "合計".into(), value: WorkMath::hours_minutes(summary.total).into() },
            TkStat { label: "本週".into(), value: WorkMath::hours_minutes(summary.week).into() },
            TkStat { label: "本月".into(), value: WorkMath::hours_minutes(summary.month).into() },
        ];
        let people: Vec<TkStat> = summary
            .people
            .iter()
            .map(|(name, m)| TkStat { label: name.as_str().into(), value: WorkMath::hours_minutes(*m).into() })
            .collect();
        let empty = entries.is_empty();
        self.with_data(|d| {
            d.set_d_hours_header(format!("工時記錄　合計 {}", WorkMath::hours_minutes(summary.total)).into());
            d.set_d_hours_message(if empty { "還沒有工時" } else { "" }.into());
            d.set_d_hours_stats(vec_model(if empty { Vec::new() } else { stats }));
            d.set_d_hours_people(vec_model(people));
            d.set_d_hours_rows(vec_model(rows));
        });
        HOURS.with(|h| h.borrow_mut().groups = groups);
    }

    /// 點了一組的標題：展開或收起（不重讀）。
    fn toggle_hours_group(&self, index: i32) {
        let rows = HOURS.with(|h| {
            let mut h = h.borrow_mut();
            let g = usize::try_from(index).ok().and_then(|i| h.groups.get_mut(i))?;
            g.expanded = !g.expanded;
            Some(hours_rows(&h.groups, &Local::now().fixed_offset()))
        });
        if let Some(rows) = rows {
            self.with_data(|d| d.set_d_hours_rows(vec_model(rows)));
        }
    }

    /// 展開「整棵子任務樹」才讀（最多 5 層）：照樹狀縮排，點編號打開那個任務。
    async fn load_subtree(self: Rc<Self>) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        if self.st.borrow().tree_for.as_deref() == Some(key.as_str()) {
            return;
        }
        self.st.borrow_mut().tree_for = Some(key.clone());
        self.with_data(|d| d.set_d_tree_lines(vec_model(vec![line(2, "讀取中…")])));
        let (rows, truncated) = match self.engine().api().task_tree(&key, 5).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("讀取 {key} 的子任務樹失敗: {}", e.0);
                self.with_data(|d| d.set_d_tree_lines(vec_model(vec![line(3, format!("讀取失敗：{}", e.0))])));
                self.st.borrow_mut().tree_for = None;
                return;
            }
        };
        if self.st.borrow().detail.as_ref().is_none_or(|d| d.key != key) {
            return;
        }
        let header = format!("整棵子任務樹　{} 個", rows.len())
            + if truncated { "（太多了，只列一部分）" } else { "" };
        let mut lines: Vec<TkLine> = rows
            .iter()
            .map(|r| {
                let mut meta = format!("· {}", r.status);
                if !r.assignees.is_empty() {
                    meta += &format!(" · {}", r.assignees.join("、"));
                }
                TkLine {
                    kind: 4,
                    key: r.key.as_str().into(),
                    text: r.title.as_str().into(),
                    meta: meta.into(),
                    depth: (r.depth - 1).max(0),
                    dim: matches!(r.status_category.as_deref(), Some("DONE" | "CANCELLED")),
                }
            })
            .collect();
        if rows.is_empty() {
            lines.push(line(2, "沒有子任務"));
        }
        self.with_data(|d| {
            d.set_d_tree_header(header.into());
            d.set_d_tree_lines(vec_model(lines));
        });
    }

    async fn send_chat(self: Rc<Self>) {
        let Some(body) = self.with_data(|d| d.get_d_chat_text().trim().to_owned()) else { return };
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        if body.is_empty() {
            return;
        }
        let api = self.engine().api();
        let k = key.clone();
        let ok = inputs::run(
            |busy| {
                self.with_data(|d| d.set_d_send_busy(busy));
            },
            |msg| self.show_msg(msg),
            move || async move { api.send_chat(Some(&k), None, None, &body).await.map_err(ActionError::from) },
            None,
        )
        .await;
        if ok {
            self.with_data(|d| d.set_d_chat_text(SharedString::new()));
            let version = self.st.borrow().detail_version;
            self.load_chat(&key, version).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn 預估工時輸入() {
        assert_eq!(Ok(None), parse_estimate("  "));
        assert_eq!(Ok(Some(1.5)), parse_estimate("1.5"));
        assert_eq!(Ok(Some(0.0)), parse_estimate("0"));
        assert_eq!(Err(()), parse_estimate("-1"));
        assert_eq!(Err(()), parse_estimate("1667"));
        assert_eq!(Err(()), parse_estimate("四"));
    }

    #[test]
    fn 子任務工時_沒有時長就用起訖() {
        let at = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap();
        let now = at("2026-11-12T12:00:00+08:00");
        let mut t = TimeEntry::new(
            "1",
            "F2P-2",
            "x",
            Some(at("2026-11-12T09:00:00+08:00")),
            Some(at("2026-11-12T10:30:00+08:00")),
            None,
            false,
            None,
        );
        assert_eq!(90.0, entry_minutes(&t, &now));
        t.duration_minutes = Some(45);
        assert_eq!(45.0, entry_minutes(&t, &now));
        t.duration_minutes = None;
        t.ended_at = None;
        t.running = true;
        assert_eq!(180.0, entry_minutes(&t, &now));
        t.user = Some("amy".into());
        assert!(entry_text(&t, &now).ends_with("–進行中　3 小時　amy"));
    }

    fn local_at(y: i32, m: u32, d: u32, h: u32, min: u32) -> hb_api::DateTimeOffset {
        use chrono::TimeZone;
        Local
            .from_local_datetime(&NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap())
            .unwrap()
            .fixed_offset()
    }

    fn hour_entry(user: &str, start: hb_api::DateTimeOffset, minutes: i32) -> TimeEntry {
        let mut t = TimeEntry::new(
            "1",
            "F2P-2",
            "x",
            Some(start),
            Some(start + TimeDelta::minutes(minutes as i64)),
            None,
            false,
            None,
        );
        t.user = Some(user.into());
        t
    }

    #[test]
    fn 工時記錄_合計本週本月與每個人() {
        // 2026-11-12 週四
        let now = local_at(2026, 11, 12, 15, 0);
        let entries = vec![
            hour_entry("amy", local_at(2026, 11, 12, 9, 0), 60),
            hour_entry("bob", local_at(2026, 11, 9, 9, 0), 120), // 本週一
            hour_entry("amy", local_at(2026, 11, 3, 9, 0), 30),  // 本月、上週
            hour_entry("bob", local_at(2026, 10, 20, 9, 0), 240), // 上個月
            hour_entry("", local_at(2026, 10, 1, 9, 0), 15),     // 沒有名字：只算合計
        ];
        let s = hours_summary(&entries, &now);
        assert_eq!((465.0, 180.0, 210.0), (s.total, s.week, s.month));
        assert_eq!(vec![("bob".to_owned(), 360.0), ("amy".to_owned(), 90.0)], s.people);
    }

    #[test]
    fn 工時記錄_本週按天_較早按週按月收起來() {
        let now = local_at(2026, 11, 12, 15, 0);
        let entries = vec![
            hour_entry("amy", local_at(2026, 11, 12, 9, 0), 60),
            hour_entry("amy", local_at(2026, 11, 12, 13, 0), 30),
            hour_entry("amy", local_at(2026, 11, 11, 9, 0), 60),
            hour_entry("amy", local_at(2026, 11, 9, 9, 0), 60),
            hour_entry("amy", local_at(2026, 11, 3, 9, 0), 45),
            hour_entry("amy", local_at(2026, 11, 2, 9, 0), 15),
            hour_entry("amy", local_at(2026, 10, 12, 9, 0), 30), // 本週一往前 28 天內
            hour_entry("amy", local_at(2026, 10, 5, 9, 0), 30),  // 更早：按月
            hour_entry("amy", local_at(2026, 9, 1, 9, 0), 30),
        ];
        let groups = hours_groups(&entries, &now);
        let view: Vec<(&str, f64, bool, usize)> =
            groups.iter().map(|g| (g.label.as_str(), g.minutes, g.expanded, g.entries.len())).collect();
        assert_eq!(
            vec![
                ("今天", 90.0, true, 2),
                ("昨天", 60.0, true, 1),
                ("11/9（週一）", 60.0, true, 1),
                ("11/2–11/8 那週", 60.0, false, 2),
                ("10/12–10/18 那週", 30.0, false, 1),
                ("2026 年 10 月", 30.0, false, 1),
                ("2026 年 9 月", 30.0, false, 1),
            ],
            view
        );
        // 組內新的在前
        assert_eq!(Some(local_at(2026, 11, 12, 13, 0)), groups[0].entries[0].started_at);
        // 攤平：收起的組只有標題
        let rows = hours_rows(&groups, &now);
        assert_eq!(7 + 4, rows.len()); // 7 組標題＋展開的三天裡 4 段
        assert_eq!("今天（2 段）", rows[0].label.as_str());
        assert_eq!("1 小時 30 分", rows[0].meta.as_str());
        assert_eq!("13:00–13:30　amy", rows[1].label.as_str());
        assert!(rows.last().is_some_and(|r| r.header && !r.expanded));
        let mut groups = groups;
        groups[3].expanded = true;
        let rows = hours_rows(&groups, &now);
        assert!(rows.iter().any(|r| r.label.as_str() == "11/3 09:00–09:45　amy"));
    }

    #[test]
    fn 實際日期() {
        let mut d = hb_api::HayakuApi::parse_task_detail(&serde_json::json!({ "task": "F2P-1" }));
        assert_eq!("—", actual_text(&d));
        d.actual_start = NaiveDate::from_ymd_opt(2026, 11, 1);
        assert_eq!("2026-11-01 → 進行中", actual_text(&d));
        d.actual_end = NaiveDate::from_ymd_opt(2026, 11, 3);
        assert_eq!("2026-11-01 → 2026-11-03", actual_text(&d));
    }
}
