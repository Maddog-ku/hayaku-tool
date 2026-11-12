//! 對應 C# `UI/ViewModels.cs`：面板清單各列的顯示資料。
//!
//! C# 的 Brush 換成 [`ColorKey`]（對應 hb-ui-kit `Theme` 的色票名稱），Visibility 換成 bool；
//! INotifyPropertyChanged 的 Busy、Elapsed 是一般欄位，呼叫端改了再重新放進 Slint 的 model。

use chrono::{Local, NaiveDateTime};
use hb_api::{DateTimeOffset, MeetingItem, NotificationItem, ReminderItem, TodoItem};
use hb_claude::RepoActivity;

/// C# `Brushes2.Get(key)` 的色票；對應 hb-ui-kit `Theme` 的屬性。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorKey {
    /// TextBrush → `Theme.ink`
    Text,
    /// MutedBrush → `Theme.muted`
    Muted,
    /// RunBrush → `Theme.run`
    Run,
    /// ClaudeBrush → `Theme.claude`
    Claude,
    /// AccentBrush → `Theme.accent`
    Accent,
    /// ErrorBrush → `Theme.error`
    Error,
    /// WarnBrush → `Theme.warn`
    Warn,
}

impl ColorKey {
    /// 給 Slint 用的整數代碼（`@hbc` 的 `Palette.color(code)` 依這個取 Theme 色）：0 Text、1 Muted、2 Run、3 Claude、4 Accent、5 Error、6 Warn。
    pub fn code(self) -> i32 {
        match self {
            ColorKey::Text => 0,
            ColorKey::Muted => 1,
            ColorKey::Run => 2,
            ColorKey::Claude => 3,
            ColorKey::Accent => 4,
            ColorKey::Error => 5,
            ColorKey::Warn => 6,
        }
    }
}

/// 面板計時清單的一列。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskRowVm {
    pub key: String,
    pub title: String,
    pub running: bool,
    pub is_claude: bool,
    /// 上層任務名稱，同名的子任務才分得出來（跟網頁右上角的計時清單一樣）；最上層或查不到時是 None。
    pub parent_title: Option<String>,
    /// 任務目前的狀態；不在我的任務清單裡（查不到）時不顯示。
    pub status: Option<String>,
    /// 按了開始／暫停、還在等 Hayaku 回應。
    pub busy: bool,
    pub elapsed: String,
}

impl TaskRowVm {
    pub fn parent_prefix(&self) -> String {
        match &self.parent_title {
            Some(p) if !p.is_empty() => format!("{p} › "),
            _ => String::new(),
        }
    }

    pub fn status_visible(&self) -> bool {
        self.status.as_deref().is_some_and(|s| !s.is_empty())
    }

    pub fn icon(&self) -> &'static str {
        if self.is_claude {
            "✦"
        } else if self.running {
            "●"
        } else {
            "○"
        }
    }

    pub fn icon_color(&self) -> ColorKey {
        if self.is_claude {
            ColorKey::Claude
        } else if self.running {
            ColorKey::Run
        } else {
            ColorKey::Muted
        }
    }

    pub fn icon_tip(&self) -> &'static str {
        if self.is_claude {
            "Claude 自動計時"
        } else if self.running {
            "計時中"
        } else {
            "已暫停"
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
        if self.busy {
            "處理中"
        } else if self.running {
            "暫停"
        } else {
            "開始計時"
        }
    }

    pub fn not_busy(&self) -> bool {
        !self.busy
    }
}

/// 設定頁 Claude 資料夾的一列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRowVm {
    pub name: String,
    pub path: String,
    pub status_text: String,
    pub status_color: ColorKey,
    pub task_text: String,
    pub task_tip: String,
    pub pick_visible: bool,
}

impl RepoRowVm {
    pub fn from(a: &RepoActivity, now: NaiveDateTime, _idle_minutes: i32) -> Self {
        let (status, color) = if !a.enabled {
            ("已停用".to_owned(), ColorKey::Muted)
        } else if a.last_activity == NaiveDateTime::MIN {
            ("找不到 session".to_owned(), ColorKey::Muted)
        } else if a.active {
            let source = a.source_text();
            (if source.is_empty() { "活動中".to_owned() } else { format!("活動中（{source}）") }, ColorKey::Claude)
        } else {
            (format!("閒置 {}", repo_ago(now - a.last_activity)), ColorKey::Muted)
        };
        RepoRowVm {
            name: a.name().to_owned(),
            path: a.path.clone(),
            status_text: status,
            status_color: color,
            task_text: a.resolved_task.clone().unwrap_or_else(|| "未對應任務".to_owned()),
            task_tip: match &a.resolved_task {
                None => format!("偵測不到任務編號（分支：{}），點「選擇」指定", a.branch.as_deref().unwrap_or("無")),
                Some(_) => {
                    format!("來源：{}；Claude 有動作就開始計時，之後持續計時", a.task_source.as_deref().unwrap_or(""))
                }
            },
            pick_visible: true,
        }
    }
}

fn repo_ago(span: chrono::TimeDelta) -> String {
    let minutes = span.num_milliseconds() as f64 / 60000.0;
    if minutes < 1.0 {
        "不到 1 分".to_owned()
    } else if minutes < 60.0 {
        format!("{} 分", minutes as i64)
    } else if minutes < 1440.0 {
        format!("{} 小時", (minutes / 60.0) as i64)
    } else {
        format!("{} 天", (minutes / 1440.0) as i64)
    }
}

/// 今天的會議一列。
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingRowVm {
    pub item: MeetingItem,
    pub time: String,
    pub title: String,
    pub detail: String,
    pub time_color: ColorKey,
    pub join_visible: bool,
}

impl MeetingRowVm {
    pub fn from(m: &MeetingItem, now: &DateTimeOffset) -> Self {
        let ongoing = *now >= m.start_at && *now < m.end_at;
        let can_join = *now >= m.start_at - chrono::TimeDelta::minutes(15) && *now < m.end_at;
        let mut parts: Vec<String> = Vec::new();
        if !m.project.is_empty() {
            parts.push(m.project.clone());
        }
        if let Some(l) = m.location.as_deref().filter(|l| !l.trim().is_empty()) {
            parts.push(l.to_owned());
        }
        if let Some(s) = m.my_status.as_deref().filter(|s| !s.trim().is_empty()) {
            parts.push(s.to_owned());
        } else if *now >= m.end_at {
            parts.push("已結束".to_owned());
        }
        let local = |t: &DateTimeOffset| t.with_timezone(&Local).format("%H:%M").to_string();
        MeetingRowVm {
            item: m.clone(),
            time: format!("{}–{}", local(&m.start_at), local(&m.end_at)),
            title: m.title.clone(),
            detail: parts.join(" · "),
            time_color: if ongoing {
                ColorKey::Accent
            } else if *now >= m.end_at {
                ColorKey::Muted
            } else {
                ColorKey::Text
            },
            join_visible: !m.in_meeting() && can_join,
        }
    }
}

/// 待辦一列。
#[derive(Debug, Clone, PartialEq)]
pub struct TodoRowVm {
    pub item: TodoItem,
    pub busy: bool,
}

impl TodoRowVm {
    pub fn new(item: TodoItem) -> Self {
        Self { item, busy: false }
    }

    pub fn text(&self) -> &str {
        &self.item.text
    }

    pub fn detail(&self) -> String {
        let i = &self.item;
        [
            i.task_code.clone(),
            i.remind_at.map(|at| format!("提醒 {}", at.with_timezone(&Local).format("%-m/%-d %H:%M"))),
            i.assigned_by.as_ref().map(|by| format!("{by} 指派")),
            i.source.clone(),
        ]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
    }

    pub fn detail_visible(&self) -> bool {
        !self.detail().is_empty()
    }

    pub fn not_busy(&self) -> bool {
        !self.busy
    }
}

/// 提醒一列。
#[derive(Debug, Clone, PartialEq)]
pub struct ReminderRowVm {
    pub item: ReminderItem,
}

impl ReminderRowVm {
    pub fn text(&self) -> String {
        match &self.item.task {
            Some(task) => format!("{}（{task}）", self.item.text),
            None => self.item.text.clone(),
        }
    }

    pub fn time(&self) -> String {
        let Some(at) = self.item.at else { return String::new() };
        let local = at.with_timezone(&Local);
        if local.date_naive() == Local::now().date_naive() {
            local.format("%H:%M").to_string()
        } else {
            local.format("%-m/%-d %H:%M").to_string()
        }
    }
}

/// 通知一列（收件匣、討論分頁）。
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationRowVm {
    pub item: NotificationItem,
    /// 討論分頁的列：多顯示引用的原文、說過話的人、解決了沒。
    pub discussion: bool,
}

impl NotificationRowVm {
    pub fn new(item: NotificationItem, discussion: bool) -> Self {
        Self { item, discussion }
    }

    pub fn label(&self) -> String {
        let resolved = self.discussion && self.item.resolved == Some(true);
        format!("{}{}", self.item.label, if resolved { "・已解決" } else { "" })
    }

    pub fn count_text(&self) -> String {
        if self.item.count > 1 { format!("（{}）", self.item.count) } else { String::new() }
    }

    pub fn text(&self) -> String {
        match &self.item.quote {
            Some(quote) if self.discussion && !quote.is_empty() && *quote != self.item.text => {
                format!("{}\n「{quote}」", self.item.text).trim_start().to_owned()
            }
            _ => self.item.text.clone(),
        }
    }

    pub fn unread_visible(&self) -> bool {
        self.item.unread
    }

    pub fn text_color(&self) -> ColorKey {
        if self.item.unread { ColorKey::Text } else { ColorKey::Muted }
    }

    pub fn meta(&self) -> String {
        self.meta_at(&Local::now().fixed_offset())
    }

    pub fn meta_at(&self, now: &DateTimeOffset) -> String {
        let minutes = (*now - self.item.created_at).num_milliseconds() as f64 / 60000.0;
        let ago = if minutes < 1.0 {
            "剛剛".to_owned()
        } else if minutes < 60.0 {
            format!("{} 分鐘前", minutes as i64)
        } else if minutes < 1440.0 {
            format!("{} 小時前", (minutes / 60.0) as i64)
        } else {
            format!("{} 天前", (minutes / 1440.0) as i64)
        };
        if self.discussion
            && let Some(users) = self.item.users.as_ref().filter(|u| u.len() > 1)
        {
            let shown: Vec<&str> = users.iter().take(4).map(String::as_str).collect();
            return format!("{}{} · {ago}", shown.join("、"), if users.len() > 4 { " 等" } else { "" });
        }
        match &self.item.actor {
            Some(actor) => format!("{actor} · {ago}"),
            None => ago,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::HayakuApi;

    // Hayaku14Tests.通知的討論分頁_一段討論一組（NotificationRowVm 的部分連同解析一起）
    #[test]
    fn 通知的討論分頁_一段討論一組() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"items":[{"latest":{"id":"n2","type":"MENTIONED","actor":{"displayName":"羅瑞騰"},
                 "project":{"key":"TEST"},"task":{"number":11,"title":"42","kind":"TASK"},
                 "payload":{"task":"TEST-11 42","excerpt":"@harry.gu 你做好了嗎？"},"readAt":null,"createdAt":"2026-10-27T07:51:11.294Z"},
               "count":2,"unreadCount":1,"ids":["n1","n2"],"users":["羅瑞騰","吳景涵"],"resolved":false,"rootExcerpt":null}]}"#,
        )
        .unwrap();
        let list = HayakuApi::parse_notifications(&json);
        assert_eq!(1, list.len());
        let n = list[0].clone();
        assert_eq!(vec!["n1", "n2"], n.all_ids());
        assert_eq!(Some(vec!["羅瑞騰".to_owned(), "吳景涵".to_owned()]), n.users);
        assert_eq!(Some(false), n.resolved);
        assert_eq!(Some("@harry.gu 你做好了嗎？"), n.quote.as_deref());
        assert!(n.unread);

        let vm = NotificationRowVm::new(n.clone(), true);
        assert!(vm.text().contains("「@harry.gu 你做好了嗎？」"));
        assert!(vm.meta().starts_with("羅瑞騰、吳景涵"));
        // 收件匣照舊只顯示任務
        assert_eq!("TEST-11 42", NotificationRowVm::new(n, false).text());
    }

    #[test]
    fn task_row_圖示與動作() {
        let mut vm = TaskRowVm { key: "A-1".into(), title: "t".into(), running: true, ..Default::default() };
        assert_eq!(
            ("●", ColorKey::Run, "計時中", "❚❚", "暫停"),
            (vm.icon(), vm.icon_color(), vm.icon_tip(), vm.action_glyph(), vm.action_tip())
        );
        vm.busy = true;
        assert_eq!(("…", "處理中"), (vm.action_glyph(), vm.action_tip()));
        vm.is_claude = true;
        assert_eq!(("✦", "Claude 自動計時"), (vm.icon(), vm.icon_tip()));
        assert_eq!("", vm.parent_prefix());
        vm.parent_title = Some("上層".into());
        assert_eq!("上層 › ", vm.parent_prefix());
    }
}
