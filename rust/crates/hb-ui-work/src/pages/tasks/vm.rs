//! 對應 C# `UI/Work/TasksPage.xaml.cs` 檔尾的 view model（TaskListRowVm、MyTreeProjectVm、MyTreeTaskVm、KanbanCardVm、
//! TreeNodeVm、ChatRowVm）與幾個顯示用的純函式。畫面無關，可以單獨測試。

use chrono::NaiveDate;
use hb_api::{ChatMessage, KeyFinder, ProjectInfo, ProjectTask, TaskSummary, WorkMath};
use hb_core::{CiSet, MyTaskNode, MyTaskProject, TaskTreeNode};
use hb_ui_common::inputs;

/// 清單的一列（C# TaskListRowVm）。
#[derive(Debug, Clone, PartialEq)]
pub struct RowVm {
    pub key: String,
    pub title: String,
    pub context: String,
    pub status: String,
    pub planned_start: Option<NaiveDate>,
    pub planned_end: Option<NaiveDate>,
    pub timing: bool,
    pub done: bool,
    pub priority: String,
    /// 專案欄的文字（名稱，或設定成只顯示代碼）；project 是用來找圖示的專案。
    pub project_text: String,
    pub project: Option<ProjectInfo>,
    /// 今天計時（計時中的每秒更新）。
    pub elapsed: String,
}

impl RowVm {
    pub fn new(key: &str, title: &str) -> Self {
        Self {
            key: key.to_owned(),
            title: title.to_owned(),
            context: String::new(),
            status: String::new(),
            planned_start: None,
            planned_end: None,
            timing: false,
            done: false,
            priority: String::new(),
            project_text: String::new(),
            project: None,
            elapsed: String::new(),
        }
    }

    pub fn priority_text(&self) -> &str {
        if self.priority.is_empty() || self.priority == "無" { "" } else { &self.priority }
    }

    pub fn start_text(&self, today: NaiveDate) -> String {
        start_text(self.planned_start, self.planned_end, today)
    }

    pub fn due_text(&self, today: NaiveDate) -> String {
        due_text(self.planned_start, self.planned_end, self.done, today)
    }

    pub fn due_tone(&self, today: NaiveDate) -> i32 {
        due_tone(self.planned_end, self.done, today)
    }
}

/// 預計開始欄：有開始就是日期；只有結束時寫「未指定」；都沒有是空的。
pub fn start_text(start: Option<NaiveDate>, end: Option<NaiveDate>, today: NaiveDate) -> String {
    match (start, end) {
        (Some(s), _) => inputs::short_date_at(s, today),
        (None, Some(_)) => "未指定".to_owned(),
        (None, None) => String::new(),
    }
}

/// 到期文字（C# TaskListRowVm.DueText）：已完成的只顯示日期；逾期 N 天、今天到期、明天到期、M/d 到期。
pub fn due_text(start: Option<NaiveDate>, end: Option<NaiveDate>, done: bool, today: NaiveDate) -> String {
    let Some(end) = end else {
        return if start.is_some() { "結束未指定".to_owned() } else { String::new() };
    };
    let days = (end - today).num_days();
    if done {
        inputs::short_date_at(end, today)
    } else if days < 0 {
        format!("逾期 {} 天", -days)
    } else if days == 0 {
        "今天到期".to_owned()
    } else if days == 1 {
        "明天到期".to_owned()
    } else {
        format!("{} 到期", inputs::short_date_at(end, today))
    }
}

/// 到期文字的顏色（C# DueBrush）：0 Muted、1 Error（逾期）、2 Warn（今天、明天）。
pub fn due_tone(end: Option<NaiveDate>, done: bool, today: NaiveDate) -> i32 {
    let Some(end) = end.filter(|_| !done) else { return 0 };
    let days = (end - today).num_days();
    if days < 0 {
        1
    } else if days <= 1 {
        2
    } else {
        0
    }
}

/// 專案欄的文字：名稱，或設定成只顯示代碼。
pub fn project_text(code_only: bool, project_name: &str, key: &str) -> String {
    if code_only { KeyFinder::project_of(key).to_owned() } else { project_name.to_owned() }
}

/// 完成類（完成、已取消）。
pub fn is_done_category(category: &str) -> bool {
    matches!(category, "DONE" | "CANCELLED")
}

/// 清單第二行：清單檢視有專案欄時只放上層路徑，其他放「專案 › 路徑」。
pub fn context_text(t: &TaskSummary, list_with_project_column: bool) -> String {
    if list_with_project_column {
        t.path.clone()
    } else if !t.path.is_empty() {
        format!("{} › {}", t.project, t.path)
    } else {
        t.project.clone()
    }
}

/// 分頁標題：有任務時後面寫數量。
pub fn tab_label(label: &str, count: usize) -> String {
    if count > 0 { format!("{label} {count}") } else { label.to_owned() }
}

/// 搜尋分頁的標題：「搜尋「…」N」，太長的字截成 8 個字＋…。
pub fn search_tab_label(query: &str, count: usize) -> String {
    let chars: Vec<char> = query.chars().collect();
    let q = if chars.len() > 8 { format!("{}…", chars[..8].iter().collect::<String>()) } else { query.to_owned() };
    format!("搜尋「{q}」{count}")
}

/// 編號的數字部分（排序用）；不是數字就排最後。
pub fn key_number(key: &str) -> i64 {
    key.rsplit_once('-').and_then(|(_, n)| n.parse::<i64>().ok()).unwrap_or(i32::MAX as i64)
}

/// 狀態篩選按鈕的文字。
pub fn status_filter_text(count: usize) -> String {
    if count == 0 { "狀態：全部 ▾".to_owned() } else { format!("狀態 · 選了 {count} 個 ▾") }
}

/// 小時數「0.##」（C# `ToString("0.##")`）。
pub fn hours_0_00(h: f64) -> String {
    let s = format!("{:.2}", round_away(h, 2));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".to_owned() } else { s.to_owned() }
}

fn round_away(v: f64, digits: i32) -> f64 {
    let p = 10f64.powi(digits);
    (v * p).abs().round().copysign(v) / p
}

/// 伺服器的顯示文字（例「3.58 小時」）取出小時數；舊版回應沒有分鐘數時用。
pub fn parse_hours(text: &str) -> Option<f64> {
    let number: String = text.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    number.parse::<f64>().ok()
}

/// 已記錄／預估（C# RenderLogged）：例「2.1 / 4 小時」；沒設預估就只顯示已記錄。回傳 (文字, 超過預估, 提示)。
pub fn logged_text(
    logged_minutes: Option<f64>,
    logged_text: &str,
    estimated: Option<f64>,
    is_category: bool,
) -> (String, bool, String) {
    let logged = logged_minutes.map(|m| m / 60.0).or_else(|| parse_hours(logged_text));
    let logged_str = logged.map_or_else(|| "0".to_owned(), WorkMath::hours_number);
    let mut text = match estimated {
        Some(est) => format!("{logged_str} / {} 小時", WorkMath::hours_number(est)),
        None => format!("{logged_str} 小時（未設預估）"),
    };
    let over = matches!((estimated, logged), (Some(e), Some(l)) if l > e);
    let tip = logged.map(|exact| format!("已記錄 {}", WorkMath::hours_minutes(exact * 60.0))).unwrap_or_default();
    if is_category {
        text += "（加總子任務）";
    }
    (text, over, tip)
}

/// 附件大小（C# FileSize）。
pub fn file_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", one_decimal(bytes as f64 / 1024.0))
    } else {
        format!("{} MB", one_decimal(bytes as f64 / 1024.0 / 1024.0))
    }
}

/// 「0.#」。
fn one_decimal(v: f64) -> String {
    let s = format!("{:.1}", round_away(v, 1));
    s.trim_end_matches(".0").to_owned()
}

/// 聊天的一則（C# ChatRowVm）：author、time、body、poll_id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRowVm {
    pub author: String,
    pub time: String,
    pub body: String,
    pub poll_id: Option<String>,
}

/// C# `ChatRows`：照時間排，刪掉的寫「（訊息已刪除）」，投票寫「📊 投票：…」。
pub fn chat_rows(messages: &[ChatMessage], now: &hb_api::DateTimeOffset) -> Vec<ChatRowVm> {
    let mut list: Vec<&ChatMessage> = messages.iter().collect();
    list.sort_by_key(|m| m.created_at);
    list.into_iter()
        .map(|m| ChatRowVm {
            author: m.author.display_name.clone(),
            time: inputs::ago_at(&m.created_at, now),
            body: if m.deleted {
                "（訊息已刪除）".to_owned()
            } else if let Some(q) = &m.poll_question {
                format!("📊 投票：{q}")
            } else {
                m.body.clone()
            },
            poll_id: if m.deleted { None } else { m.poll_id.clone() },
        })
        .collect()
}

// ───────────── 樹狀檢視 ─────────────

/// 樹狀清單的一列（攤平後）：專案列或任務列。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum TreeItem {
    Project {
        name: String,
        count: usize,
        expanded: bool,
    },
    Task {
        key: String,
        title: String,
        /// 這次清單裡的那一列；None 表示只是上層脈絡（不在這個分頁裡），淡色、沒有計時按鈕。
        row: Option<RowVm>,
        /// 層級：專案底下第一層是 1。
        depth: i32,
        children: usize,
        expanded: bool,
    },
}

impl TreeItem {
    pub fn sub_text(&self) -> String {
        match self {
            TreeItem::Task { row: None, .. } => "上層任務（不在這個分頁）".to_owned(),
            TreeItem::Task { children, .. } if *children > 0 => format!("{children} 個子任務"),
            _ => String::new(),
        }
    }

    pub fn tip(&self) -> String {
        match self {
            TreeItem::Task { key, title, row: None, .. } => {
                format!("{key} {title}\n上層任務（不在這個分頁裡），點一下查看")
            }
            TreeItem::Task { key, title, .. } => format!("{key} {title}"),
            TreeItem::Project { .. } => String::new(),
        }
    }

    pub fn text_opacity(&self) -> f32 {
        match self {
            TreeItem::Task { row: None, .. } => 0.55,
            TreeItem::Task { row: Some(r), .. } if r.done => 0.6,
            _ => 1.0,
        }
    }
}

/// 專案收合用的鍵（C#「p:名稱」）。
pub fn project_collapse_key(name: &str) -> String {
    format!("p:{name}")
}

/// 依專案分組的樹攤平成清單，收合的節點底下不列（C# RenderTree＋AddVisible）。
pub fn flatten_tree(projects: &[MyTaskProject], rows: &[RowVm], collapsed: &CiSet) -> Vec<TreeItem> {
    let mut out = Vec::new();
    for p in projects {
        let expanded = !collapsed.contains(&project_collapse_key(&p.project));
        out.push(TreeItem::Project { name: p.project.clone(), count: p.count, expanded });
        if expanded {
            add_visible(&p.roots, rows, collapsed, 1, &mut out);
        }
    }
    out
}

fn add_visible(nodes: &[MyTaskNode], rows: &[RowVm], collapsed: &CiSet, depth: i32, out: &mut Vec<TreeItem>) {
    for n in nodes {
        let row = n.task.as_ref().and_then(|_| rows.iter().find(|r| r.key.eq_ignore_ascii_case(&n.key))).cloned();
        let title = row.as_ref().map_or_else(|| n.title.clone(), |r| r.title.clone());
        let expanded = !collapsed.contains(&n.key);
        out.push(TreeItem::Task { key: n.key.clone(), title, row, depth, children: n.children.len(), expanded });
        if expanded {
            add_visible(&n.children, rows, collapsed, depth + 1, out);
        }
    }
}

/// 全部收合（有展開的就全部收起來）或全部展開（C# CollapseAll_Click）；收合狀態記著。
pub fn collapse_all(projects: &[MyTaskProject], collapsed: &mut CiSet) {
    let any_expanded = projects.iter().any(|p| !collapsed.contains(&project_collapse_key(&p.project)));
    if any_expanded {
        for p in projects {
            collapsed.insert(project_collapse_key(&p.project));
            collapse_tasks(&p.roots, collapsed);
        }
    } else {
        collapsed.clear();
    }
}

fn collapse_tasks(nodes: &[MyTaskNode], collapsed: &mut CiSet) {
    for n in nodes.iter().filter(|n| !n.children.is_empty()) {
        collapsed.insert(n.key.clone());
        collapse_tasks(&n.children, collapsed);
    }
}

/// 全部收合／全部展開按鈕的文字。
pub fn collapse_button_text(projects: &[MyTaskProject], collapsed: &CiSet) -> &'static str {
    if projects.iter().any(|p| !collapsed.contains(&project_collapse_key(&p.project))) {
        "⊟ 全部收合"
    } else {
        "⊞ 全部展開"
    }
}

// ───────────── 看板 ─────────────

/// 看板的欄：0 待處理（不是進行中、完成、已取消的）、1 進行中、2 已完成（完成＋已取消）。
pub fn kanban_column(category: &str) -> usize {
    match category {
        "IN_PROGRESS" => 1,
        "DONE" | "CANCELLED" => 2,
        _ => 0,
    }
}

/// 看板卡片的提示：編號名稱＋專案（有上層時「專案 › 路徑」）。
pub fn card_tip(t: &TaskSummary) -> String {
    if t.path.is_empty() {
        format!("{} {}\n{}", t.key, t.title, t.project)
    } else {
        format!("{} {}\n{} › {}", t.key, t.title, t.project, t.path)
    }
}

// ───────────── 新增任務的專案樹 ─────────────

/// 專案樹的一列（C# TreeNodeVm，攤平成清單）。
#[derive(Debug, Clone, PartialEq)]
pub struct PickRowVm {
    pub task: ProjectTask,
    pub depth: i32,
    pub has_children: bool,
    pub expanded: bool,
    pub picked: bool,
}

impl PickRowVm {
    pub fn status_suffix(&self) -> String {
        if self.task.is_category {
            format!("  · 分類 · {}", self.task.status)
        } else {
            format!("  · {}", self.task.status)
        }
    }

    pub fn opacity(&self) -> f32 {
        if is_done_category(&self.task.status_category) { 0.55 } else { 1.0 }
    }

    pub fn tip(&self) -> String {
        let t = &self.task;
        let mut s = format!("{} {}", t.key, t.title);
        if t.planned_start.is_some() || t.planned_end.is_some() {
            s += &format!("\n預計 {} → {}", date_or_unset(t.planned_start), date_or_unset(t.planned_end));
        }
        s + "\n點一下或按 ＋：在這個任務底下新增"
    }
}

fn date_or_unset(d: Option<NaiveDate>) -> String {
    match d {
        Some(day) => inputs::date_text(Some(day)),
        None => "未指定".to_owned(),
    }
}

/// 某任務的所有上層（含自己），由下往上（C# AncestorsOf）。
pub fn ancestors_of(tasks: &[ProjectTask], key: &str) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut k = Some(key.to_owned());
    while let Some(cur) = k {
        if chain.iter().any(|c| c.eq_ignore_ascii_case(&cur)) {
            break;
        }
        chain.push(cur.clone());
        k = tasks.iter().find(|t| t.key.eq_ignore_ascii_case(&cur)).and_then(|t| t.parent.clone());
        if !tasks.iter().any(|t| t.key.eq_ignore_ascii_case(chain.last().map_or("", String::as_str))) {
            break;
        }
    }
    chain
}

/// 「建立在」的文字：最上層，或上層鏈「F2P-1 名稱 › F2P-5 名稱」。
pub fn parent_text(tasks: &[ProjectTask], picked: Option<&str>) -> String {
    let Some(picked) = picked else { return "最上層（不屬於任何任務）".to_owned() };
    let mut chain = ancestors_of(tasks, picked);
    chain.reverse();
    chain
        .iter()
        .map(|k| match tasks.iter().find(|t| t.key.eq_ignore_ascii_case(k)) {
            Some(t) => format!("{} {}", t.key, t.title),
            None => k.clone(),
        })
        .collect::<Vec<_>>()
        .join(" › ")
}

/// 專案樹攤平：expand_all 全部展開，否則只展開第一層與選中任務的上層；使用者點過的照 toggled（C# TreeNodeVm.From＋TreeView 的 IsExpanded）。
pub fn flatten_pick_tree(
    roots: &[TaskTreeNode],
    picked: Option<&str>,
    expand_all: bool,
    picked_ancestors: &CiSet,
    toggled: &std::collections::HashMap<String, bool>,
) -> Vec<PickRowVm> {
    let mut out = Vec::new();
    fn walk(
        nodes: &[TaskTreeNode],
        depth: i32,
        picked: Option<&str>,
        expand_all: bool,
        anc: &CiSet,
        toggled: &std::collections::HashMap<String, bool>,
        out: &mut Vec<PickRowVm>,
    ) {
        for n in nodes {
            let default = expand_all || depth == 0 || anc.contains(&n.task.key);
            let expanded = toggled.get(&n.task.key.to_uppercase()).copied().unwrap_or(default);
            out.push(PickRowVm {
                task: n.task.clone(),
                depth,
                has_children: !n.children.is_empty(),
                expanded,
                picked: picked.is_some_and(|p| p.eq_ignore_ascii_case(&n.task.key)),
            });
            if expanded {
                walk(&n.children, depth + 1, picked, expand_all, anc, toggled, out);
            }
        }
    }
    walk(roots, 0, picked, expand_all, picked_ancestors, toggled, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::PersonRef;
    use hb_core::MyTaskTree;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    fn summary(key: &str, ancestors: Option<Vec<(&str, &str)>>, child_count: Option<i32>) -> TaskSummary {
        let mut t = TaskSummary::new(
            key,
            format!("任務 {key}"),
            "F2P 專案",
            "進行中",
            "IN_PROGRESS",
            "",
            None,
            None,
            false,
            "",
        );
        t.ancestors = ancestors.map(|a| a.into_iter().map(|(k, n)| (k.to_owned(), n.to_owned())).collect());
        t.child_count = child_count;
        t
    }

    #[test]
    fn 到期文字與顏色() {
        let today = d(11, 12);
        assert_eq!("", due_text(None, None, false, today));
        assert_eq!("結束未指定", due_text(Some(d(11, 1)), None, false, today));
        assert_eq!("逾期 3 天", due_text(None, Some(d(11, 9)), false, today));
        assert_eq!("今天到期", due_text(None, Some(today), false, today));
        assert_eq!("明天到期", due_text(None, Some(d(11, 13)), false, today));
        assert_eq!("11/16（週一） 到期", due_text(None, Some(d(11, 16)), false, today));
        // 已完成的只顯示日期
        assert_eq!("11/16（週一）", due_text(None, Some(d(11, 16)), true, today));
        assert_eq!(1, due_tone(Some(d(11, 9)), false, today));
        assert_eq!(2, due_tone(Some(today), false, today));
        assert_eq!(2, due_tone(Some(d(11, 13)), false, today));
        assert_eq!(0, due_tone(Some(d(11, 20)), false, today));
        assert_eq!(0, due_tone(Some(d(11, 9)), true, today));
        assert_eq!(0, due_tone(None, false, today));
    }

    #[test]
    fn 預計開始欄() {
        let today = d(11, 12);
        assert_eq!("今天", start_text(Some(today), None, today));
        assert_eq!("未指定", start_text(None, Some(d(11, 20)), today));
        assert_eq!("", start_text(None, None, today));
    }

    #[test]
    fn 優先級無不顯示() {
        let mut r = RowVm::new("F2P-1", "x");
        assert_eq!("", r.priority_text());
        r.priority = "無".into();
        assert_eq!("", r.priority_text());
        r.priority = "高".into();
        assert_eq!("高", r.priority_text());
    }

    #[test]
    fn 清單第二行() {
        let mut t = summary("F2P-1", None, None);
        assert_eq!("F2P 專案", context_text(&t, false));
        assert_eq!("", context_text(&t, true));
        t.path = "上層 › 中層".into();
        assert_eq!("F2P 專案 › 上層 › 中層", context_text(&t, false));
        assert_eq!("上層 › 中層", context_text(&t, true));
    }

    #[test]
    fn 分頁與搜尋標題() {
        assert_eq!("逾期", tab_label("逾期", 0));
        assert_eq!("逾期 3", tab_label("逾期", 3));
        assert_eq!("搜尋「登入」2", search_tab_label("登入", 2));
        assert_eq!("搜尋「一二三四五六七八…」0", search_tab_label("一二三四五六七八九", 0));
        assert_eq!("狀態：全部 ▾", status_filter_text(0));
        assert_eq!("狀態 · 選了 2 個 ▾", status_filter_text(2));
        assert_eq!("F2P", project_text(true, "F2P 專案", "F2P-12"));
        assert_eq!("F2P 專案", project_text(false, "F2P 專案", "F2P-12"));
    }

    #[test]
    fn 編號數字排序() {
        assert_eq!(12, key_number("F2P-12"));
        assert_eq!(i32::MAX as i64, key_number("F2P"));
    }

    #[test]
    fn 已記錄與預估() {
        let (text, over, tip) = logged_text(Some(126.0), "", Some(4.0), false);
        assert_eq!("2.1 / 4 小時", text);
        assert!(!over);
        assert_eq!("已記錄 2 小時 6 分", tip);
        let (text, over, _) = logged_text(Some(300.0), "", Some(4.0), false);
        assert_eq!("5 / 4 小時", text);
        assert!(over);
        let (text, _, _) = logged_text(None, "3.58 小時", None, true);
        assert_eq!("3.6 小時（未設預估）（加總子任務）", text);
        let (text, over, tip) = logged_text(None, "", None, false);
        assert_eq!("0 小時（未設預估）", text);
        assert!(!over);
        assert_eq!("", tip);
    }

    #[test]
    fn 小時數與檔案大小() {
        assert_eq!("4", hours_0_00(4.0));
        assert_eq!("1.5", hours_0_00(1.5));
        assert_eq!("1.33", hours_0_00(1.3333));
        assert_eq!(Some(3.58), parse_hours("3.58 小時"));
        assert_eq!(None, parse_hours("小時"));
        assert_eq!("512 B", file_size(512));
        assert_eq!("1.5 KB", file_size(1536));
        assert_eq!("2 KB", file_size(2048));
        assert_eq!("3.2 MB", file_size(3_355_443));
    }

    #[test]
    fn 聊天照時間排_刪除與投票() {
        let at = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap();
        let now = at("2026-11-12T10:00:00+08:00");
        let msg = |id: &str, t: &str, deleted: bool, poll: Option<&str>| ChatMessage {
            id: id.into(),
            author: PersonRef::new("amy", "Amy"),
            body: format!("內容 {id}"),
            created_at: at(t),
            deleted,
            poll_question: poll.map(str::to_owned),
            poll_id: poll.map(|_| "p1".to_owned()),
        };
        let rows = chat_rows(
            &[
                msg("2", "2026-11-12T09:55:00+08:00", false, None),
                msg("1", "2026-11-12T09:00:00+08:00", true, Some("午餐？")),
                msg("3", "2026-11-12T09:59:30+08:00", false, Some("午餐？")),
            ],
            &now,
        );
        assert_eq!(
            vec!["（訊息已刪除）", "內容 2", "📊 投票：午餐？"],
            rows.iter().map(|r| r.body.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(None, rows[0].poll_id);
        assert_eq!(Some("p1".to_owned()), rows[2].poll_id);
        assert_eq!("5 分鐘前", rows[1].time);
        assert_eq!("Amy", rows[1].author);
    }

    #[test]
    fn 樹狀攤平_收合的不列_上層脈絡沒有列() {
        let tasks = [
            summary("F2P-2", Some(vec![("F2P-1", "上層")]), Some(0)),
            summary("F2P-3", Some(vec![("F2P-1", "上層")]), Some(0)),
            summary("F2P-9", None, Some(0)),
        ];
        let projects = MyTaskTree::build(tasks.iter());
        let rows: Vec<RowVm> = tasks.iter().map(|t| RowVm::new(&t.key, &t.title)).collect();
        let mut collapsed = CiSet::new();
        let items = flatten_tree(&projects, &rows, &collapsed);
        // 專案列＋上層 F2P-1（脈絡）＋兩個子任務＋F2P-9
        assert_eq!(5, items.len());
        let parent = items.iter().find(|i| matches!(i, TreeItem::Task { key, .. } if key == "F2P-1")).unwrap();
        assert!(matches!(parent, TreeItem::Task { row: None, children: 2, depth: 1, .. }));
        assert_eq!("上層任務（不在這個分頁）", parent.sub_text());
        assert_eq!(0.55, parent.text_opacity());
        let child = items.iter().find(|i| matches!(i, TreeItem::Task { key, .. } if key == "F2P-2")).unwrap();
        assert!(matches!(child, TreeItem::Task { depth: 2, .. }));
        assert_eq!("", child.sub_text());

        collapsed.insert("f2p-1");
        assert_eq!(3, flatten_tree(&projects, &rows, &collapsed).len());

        // 全部收合：專案與有子任務的都收起來；再按一次全部展開
        collapse_all(&projects, &mut collapsed);
        assert_eq!(1, flatten_tree(&projects, &rows, &collapsed).len());
        assert_eq!("⊞ 全部展開", collapse_button_text(&projects, &collapsed));
        collapse_all(&projects, &mut collapsed);
        assert!(collapsed.is_empty());
        assert_eq!("⊟ 全部收合", collapse_button_text(&projects, &collapsed));
    }

    #[test]
    fn 看板分欄() {
        assert_eq!(0, kanban_column("TODO"));
        assert_eq!(0, kanban_column(""));
        assert_eq!(1, kanban_column("IN_PROGRESS"));
        assert_eq!(2, kanban_column("DONE"));
        assert_eq!(2, kanban_column("CANCELLED"));
        let mut t = summary("F2P-1", None, None);
        assert_eq!("F2P-1 任務 F2P-1\nF2P 專案", card_tip(&t));
        t.path = "上層".into();
        assert_eq!("F2P-1 任務 F2P-1\nF2P 專案 › 上層", card_tip(&t));
    }

    fn ptask(key: &str, parent: Option<&str>) -> ProjectTask {
        ProjectTask::new(key, format!("名稱{key}"), "待處理", "TODO", parent.map(str::to_owned), None, None)
    }

    #[test]
    fn 建立在_上層鏈() {
        let tasks = vec![ptask("A-1", None), ptask("A-2", Some("A-1")), ptask("A-3", Some("A-2"))];
        assert_eq!("最上層（不屬於任何任務）", parent_text(&tasks, None));
        assert_eq!("A-1 名稱A-1 › A-2 名稱A-2 › A-3 名稱A-3", parent_text(&tasks, Some("a-3")));
        assert_eq!(vec!["A-3", "A-2", "A-1"], ancestors_of(&tasks, "A-3"));
        // 不在清單裡的編號只列自己
        assert_eq!(vec!["X-9"], ancestors_of(&tasks, "X-9"));
    }

    #[test]
    fn 專案樹攤平_只展開第一層與選中的上層() {
        let tasks = vec![ptask("A-1", None), ptask("A-2", Some("A-1")), ptask("A-3", Some("A-2")), ptask("A-4", None)];
        let roots = hb_core::TaskTree::build(tasks.clone(), true, None);
        let none = CiSet::new();
        let toggled = std::collections::HashMap::new();
        let rows = flatten_pick_tree(&roots, None, false, &none, &toggled);
        // 第一層展開（看得到 A-2），A-2 沒展開（看不到 A-3）
        assert_eq!(vec!["A-1", "A-2", "A-4"], rows.iter().map(|r| r.task.key.as_str()).collect::<Vec<_>>());
        let anc: CiSet = ancestors_of(&tasks, "A-3").into_iter().collect();
        let rows = flatten_pick_tree(&roots, Some("A-3"), false, &anc, &toggled);
        assert_eq!(4, rows.len());
        assert!(rows.iter().find(|r| r.task.key == "A-3").unwrap().picked);
        let all = flatten_pick_tree(&roots, None, true, &none, &toggled);
        assert_eq!(4, all.len());
        let mut toggled = std::collections::HashMap::new();
        toggled.insert("A-1".to_owned(), false);
        assert_eq!(2, flatten_pick_tree(&roots, None, true, &none, &toggled).len());
        let r = &all[0];
        assert_eq!("  · 待處理", r.status_suffix());
        assert_eq!("A-1 名稱A-1\n點一下或按 ＋：在這個任務底下新增", r.tip());
    }
}
