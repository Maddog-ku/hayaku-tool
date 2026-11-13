//! 對應 C# `Slack/SlackMessages.cs`：要傳到 Slack 的訊息怎麼排（Block Kit）、按鈕代表什麼、哪些通知要傳。
//! 按鈕的 action_id 是動作，value 是訊息代號（`SlackPost::key`），重開程式後也找得回對應的通知或申請。
//!
//! C# 用匿名物件組 blocks，送出時 `WhenWritingNull` 不寫 null；這裡直接組 `serde_json::Value`，null 的欄位不放（例按鈕沒有 style）。

use std::collections::HashSet;

use chrono::NaiveDateTime;
use hb_api::{ChangeRequest, DateTimeOffset, NotificationItem, WorkMath};
use serde_json::{Value, json};

use crate::slack_state::SlackPost;

/// Slack 上按了按鈕或送出表單（Socket Mode 收到的 interactive）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SlackInteraction {
    pub r#type: String,
    pub user_id: Option<String>,
    pub action_id: Option<String>,
    pub value: Option<String>,
    pub trigger_id: Option<String>,
    pub channel: Option<String>,
    pub ts: Option<String>,
    pub callback_id: Option<String>,
    pub metadata: Option<String>,
    pub reason: Option<String>,
}

pub struct SlackMessages;

impl SlackMessages {
    pub const READ: &'static str = "read";
    pub const ARCHIVE: &'static str = "archive";
    pub const APPROVE: &'static str = "approve";
    pub const APPROVE_WATCH: &'static str = "approve_watch";
    pub const REJECT: &'static str = "reject";
    pub const PASS: &'static str = "pass";
    pub const REOPEN: &'static str = "reopen";
    pub const FIX: &'static str = "fix";
    pub const OPEN: &'static str = "open";

    /// 表單的 callback_id；private_metadata 是「動作|訊息代號」。
    pub const REASON_CALLBACK: &'static str = "reason";

    /// 一次超過這麼多則一般通知就合成一則（跟本機提示一樣）。
    pub const MAX_SINGLE: usize = 3;

    // ───────────── 哪些要傳 ─────────────

    /// 要動手處理的通知：測試結果被退回（要修）、進待復驗（要驗）。
    pub fn is_actionable(n: &NotificationItem) -> bool {
        n.is_test_result() && n.task_key.is_some() && (n.is_reopened() || n.is_ready_to_retest())
    }

    /// 這次同步新出現（開程式後才收到、還沒看過）的未讀通知裡，要傳到 Slack 的：要動手處理的一律傳，其他的只在螢幕鎖定時傳。
    /// 看過的記在 seen，所以沒鎖定時收到的一般通知，之後鎖定也不會補傳。
    /// 能審核時「有新的變更申請」不傳，由待我審核的訊息（可以直接核准、駁回）取代。
    pub fn select_notifications(
        notifications: &[NotificationItem],
        seen: &mut HashSet<String>,
        started_at: DateTimeOffset,
        locked: bool,
        can_review: bool,
    ) -> Vec<NotificationItem> {
        let mut send = Vec::new();
        for n in notifications {
            if !n.unread || n.created_at <= started_at || !seen.insert(format!("n:{}", n.id)) {
                continue;
            }
            if n.r#type == "CHANGE_REQUESTED" && can_review {
                continue;
            }
            if Self::is_actionable(n) || locked {
                send.push(n.clone());
            }
        }
        send
    }

    /// 開程式後才送出、還沒傳過的待審申請（一律傳）。
    pub fn select_reviews(
        pending: &[ChangeRequest],
        seen: &mut HashSet<String>,
        started_at: DateTimeOffset,
    ) -> Vec<ChangeRequest> {
        pending.iter().filter(|c| c.created_at > started_at && seen.insert(format!("cr:{}", c.id))).cloned().collect()
    }

    // ───────────── 訊息內容 ─────────────

    pub fn from_notification(n: &NotificationItem, url: &str, now: NaiveDateTime) -> SlackPost {
        let who = n.actor.as_ref().map(|a| format!("{a}：")).unwrap_or_default();
        let kind = if !Self::is_actionable(n) {
            "notice"
        } else if n.is_reopened() {
            "fix"
        } else {
            "retest"
        };
        let title = match &n.task_key {
            None if kind == "notice" => n.label.clone(),
            task => format!("{}・{}", n.label, task.as_deref().unwrap_or("")),
        };
        SlackPost {
            key: format!("n:{}", n.id),
            kind: kind.to_owned(),
            title,
            body: who + &n.text,
            url: Some(url.to_owned()),
            notification_ids: vec![n.id.clone()],
            task_key: n.task_key.clone(),
            toast_tag: Some(format!("n:{}", n.id)),
            posted_at: now,
            ..Default::default()
        }
    }

    /// 很多則一般通知合成一則（list 至少要有一則）。
    pub fn from_notices(list: &[NotificationItem], url: &str, now: NaiveDateTime) -> SlackPost {
        let first = list.first().map(|n| n.id.as_str()).unwrap_or("");
        let mut body =
            list.iter().take(10).map(|n| format!("• {}：{}", n.label, n.text)).collect::<Vec<_>>().join("\n");
        if list.len() > 10 {
            body.push_str("\n…");
        }
        SlackPost {
            key: format!("ns:{}:{first}", now.format("%Y%m%d%H%M%S")),
            kind: "notices".to_owned(),
            title: format!("有 {} 則新通知", list.len()),
            body,
            url: Some(url.to_owned()),
            notification_ids: list.iter().map(|n| n.id.clone()).collect(),
            posted_at: now,
            ..Default::default()
        }
    }

    pub fn from_review(c: &ChangeRequest, url: &str, now: NaiveDateTime) -> SlackPost {
        let task_key = c.task_key.as_deref().unwrap_or("");
        let requester = c.requester.as_ref().map(|r| r.display_name.as_str()).unwrap_or("有人");
        let mut lines = vec![format!("{requester}：{task_key} {}", c.task_title).trim_end().to_owned()];
        lines.extend(c.changes.iter().take(5).map(|change| format!("• {change}")));
        if c.changes.len() > 5 {
            lines.push("…".to_owned());
        }
        if let Some(reason) = c.reason.as_deref().filter(|r| !r.trim().is_empty()) {
            lines.push(format!("理由：{reason}"));
        }
        let logged = if c.logged_minutes > 0 {
            format!("（已經記錄 {} 工時）", WorkMath::hours_minutes(f64::from(c.logged_minutes)))
        } else {
            String::new()
        };
        SlackPost {
            key: format!("cr:{}", c.id),
            kind: "review".to_owned(),
            title: format!("待審核・{}", c.type_text()),
            body: lines.join("\n"),
            url: Some(url.to_owned()),
            change_request_id: Some(c.id.clone()),
            can_watch: c.r#type != "DELETE",
            reject_warning: (c.r#type == "CREATE")
                .then(|| format!("駁回會刪除 {task_key} {}{logged}", c.task_title).trim_end().to_owned()),
            task_key: c.task_key.clone(),
            toast_tag: Some(format!("cr:{}", c.id)),
            posted_at: now,
            ..Default::default()
        }
    }

    /// 通知列與不能顯示 blocks 時看到的文字。
    pub fn fallback(p: &SlackPost) -> String {
        if p.body.is_empty() { p.title.clone() } else { format!("{}：{}", p.title, first_line(&p.body)) }
    }

    /// 訊息的 blocks：標題與內容；還沒處理放按鈕（error：上次按了失敗的原因），處理了換成結果。
    pub fn blocks(p: &SlackPost, error: Option<&str>) -> Vec<Value> {
        let body = if p.body.is_empty() { String::new() } else { format!("\n{}", Self::escape(&p.body)) };
        let mut blocks = vec![section(&format!("*{}*{body}", Self::escape(&p.title)))];
        if let Some(done) = &p.done {
            blocks.push(context(done));
            return blocks;
        }
        blocks.push(json!({ "type": "actions", "block_id": "act", "elements": buttons(p) }));
        if let Some(error) = error {
            blocks.push(context(&format!("⚠ {error}")));
        }
        blocks
    }

    /// 駁回申請、退回測試結果要填原因的表單；駁回的說明可以不填，退回一定要填。
    pub fn reason_modal(p: &SlackPost, action: &str) -> Value {
        let reject = action == Self::REJECT;
        let mut blocks = vec![section(&format!("*{}*\n{}", Self::escape(&p.title), Self::escape(first_line(&p.body))))];
        if reject && let Some(warning) = &p.reject_warning {
            blocks.push(context(&format!("⚠ {warning}")));
        }
        blocks.push(json!({
            "type": "input",
            "block_id": "reason",
            "optional": reject,
            "label": plain(if reject { "給申請人的說明" } else { "退回的原因" }),
            "element": { "type": "plain_text_input", "action_id": "text", "multiline": true },
        }));
        json!({
            "type": "modal",
            "callback_id": Self::REASON_CALLBACK,
            "private_metadata": format!("{action}|{}", p.key),
            "title": plain(if reject { "駁回申請" } else { "退回測試結果" }),
            "submit": plain(if reject { "駁回" } else { "退回" }),
            "close": plain("取消"),
            "blocks": blocks,
        })
    }

    /// Slack 文字裡 & < > 要跳脫。
    pub fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    // ───────────── 收到的互動 ─────────────

    pub fn parse_interaction(p: &Value) -> SlackInteraction {
        let (action_id, value) = match p.get("actions").and_then(Value::as_array).and_then(|a| a.first()) {
            Some(first) => (s(first, &["action_id"]), s(first, &["value"])),
            None => (None, None),
        };
        let reason = p.get("view").and_then(|view| s(view, &["state", "values", "reason", "text", "value"]));
        SlackInteraction {
            r#type: s(p, &["type"]).unwrap_or_default(),
            user_id: s(p, &["user", "id"]),
            action_id,
            value,
            trigger_id: s(p, &["trigger_id"]),
            channel: s(p, &["container", "channel_id"]).or_else(|| s(p, &["channel", "id"])),
            ts: s(p, &["container", "message_ts"]).or_else(|| s(p, &["message", "ts"])),
            callback_id: s(p, &["view", "callback_id"]),
            metadata: s(p, &["view", "private_metadata"]),
            reason,
        }
    }
}

/// C# `s.Split('\n')[0]`。
fn first_line(s: &str) -> &str {
    s.split('\n').next().unwrap_or("")
}

/// 沿著路徑往下找，最後是字串才回傳。
fn s(e: &Value, path: &[&str]) -> Option<String> {
    let mut e = e;
    for name in path {
        e = e.as_object()?.get(*name)?;
    }
    e.as_str().map(str::to_owned)
}

fn buttons(p: &SlackPost) -> Vec<Value> {
    use SlackMessages as M;
    let mut list = Vec::new();
    match p.kind.as_str() {
        "review" => {
            list.push(button("✓ 核准", M::APPROVE, &p.key, Some("primary")));
            if p.can_watch {
                list.push(button("核准並關注", M::APPROVE_WATCH, &p.key, None));
            }
            list.push(button("✕ 駁回", M::REJECT, &p.key, Some("danger")));
        }
        "retest" => {
            list.push(button("✓ 通過", M::PASS, &p.key, Some("primary")));
            list.push(button("↩ 退回", M::REOPEN, &p.key, Some("danger")));
        }
        "fix" => {
            list.push(button("▶ 開始修並計時", M::FIX, &p.key, Some("primary")));
            list.push(button("標為已讀", M::READ, &p.key, None));
        }
        "notices" => {
            list.push(button("全部標為已讀", M::READ, &p.key, Some("primary")));
        }
        _ => {
            list.push(button("標為已讀", M::READ, &p.key, Some("primary")));
            list.push(button("封存", M::ARCHIVE, &p.key, None));
        }
    }
    if let Some(url) = p.url.as_deref().filter(|u| !u.is_empty()) {
        list.push(json!({ "type": "button", "text": plain("開啟"), "action_id": M::OPEN, "url": url }));
    }
    list
}

/// mrkdwn 段落；超過 2900 字（C# 的 UTF-16 長度）截斷加「…」。
fn section(mrkdwn: &str) -> Value {
    const LIMIT: usize = 2900;
    let text = if mrkdwn.encode_utf16().count() > LIMIT {
        let mut units = 0;
        let mut end = 0;
        for (i, c) in mrkdwn.char_indices() {
            units += c.len_utf16();
            if units > LIMIT {
                break;
            }
            end = i + c.len_utf8();
        }
        format!("{}…", &mrkdwn[..end])
    } else {
        mrkdwn.to_owned()
    };
    json!({ "type": "section", "text": { "type": "mrkdwn", "text": text } })
}

fn context(text: &str) -> Value {
    json!({ "type": "context", "elements": [ { "type": "mrkdwn", "text": SlackMessages::escape(text) } ] })
}

fn button(text: &str, action: &str, value: &str, style: Option<&str>) -> Value {
    let mut b = json!({ "type": "button", "text": plain(text), "action_id": action, "value": value });
    if let Some(style) = style {
        b["style"] = json!(style);
    }
    b
}

fn plain(text: &str) -> Value {
    json!({ "type": "plain_text", "text": text, "emoji": true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate, TimeDelta, TimeZone};
    use hb_api::PersonRef;

    // SlackMessagesTests

    fn started() -> DateTimeOffset {
        FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 11, 3, 9, 0, 0).unwrap()
    }

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 3).unwrap().and_hms_opt(10, 0, 0).unwrap()
    }

    fn n_full(id: &str, r#type: &str, unread: bool, minutes_after_start: i64, task: Option<&str>) -> NotificationItem {
        NotificationItem::new(
            id,
            r#type,
            "標籤",
            "內容",
            Some("小明".to_owned()),
            task.map(str::to_owned),
            Some("F2P".to_owned()),
            None,
            started() + TimeDelta::minutes(minutes_after_start),
            unread,
            1,
        )
    }

    fn n(id: &str) -> NotificationItem {
        n_full(id, "COMMENT", true, 10, Some("F2P-1"))
    }

    fn nt(id: &str, r#type: &str) -> NotificationItem {
        n_full(id, r#type, true, 10, Some("F2P-1"))
    }

    fn cr_at(id: &str, r#type: &str, minutes_after_start: i64) -> ChangeRequest {
        ChangeRequest {
            id: id.to_owned(),
            project: "F2P".to_owned(),
            r#type: r#type.to_owned(),
            status: "PENDING".to_owned(),
            task_key: Some("F2P-2".to_owned()),
            task_title: "做登入".to_owned(),
            requester: Some(PersonRef::new("amy", "Amy")),
            reviewer: None,
            created_at: started() + TimeDelta::minutes(minutes_after_start),
            changes: vec!["狀態：待處理 → 進行中".to_owned()],
            reason: Some("趕時程".to_owned()),
            review_note: None,
            logged_minutes: 90,
            descendant_count: 0,
        }
    }

    fn cr(id: &str, r#type: &str) -> ChangeRequest {
        cr_at(id, r#type, 10)
    }

    /// C# 測試用 `UnsafeRelaxedJsonEscaping` 序列化（不縮排、中文不跳脫）。
    fn json_of<T: serde::Serialize + ?Sized>(o: &T) -> String {
        hb_backend::infra::json_file::to_string_compact(o).unwrap()
    }

    fn ids(list: &[NotificationItem]) -> Vec<&str> {
        list.iter().map(|n| n.id.as_str()).collect()
    }

    #[test]
    fn 沒鎖定_只傳要動手處理的() {
        let mut seen = HashSet::new();
        let list = [n("1"), nt("2", "TEST_RESULT_REOPENED"), nt("3", "TEST_RESULT_READY_TO_RETEST")];
        let send = SlackMessages::select_notifications(&list, &mut seen, started(), false, false);
        assert_eq!(vec!["2", "3"], ids(&send));
    }

    #[test]
    fn 鎖定_一般通知也傳() {
        let send = SlackMessages::select_notifications(&[n("1")], &mut HashSet::new(), started(), true, false);
        assert_eq!(1, send.len());
    }

    #[test]
    fn 沒鎖定時看過的一般通知_之後鎖定也不補傳() {
        let mut seen = HashSet::new();
        SlackMessages::select_notifications(&[n("1")], &mut seen, started(), false, false);
        assert!(SlackMessages::select_notifications(&[n("1")], &mut seen, started(), true, false).is_empty());
    }

    #[test]
    fn 已讀_開程式前收到的_不傳() {
        let list = [n_full("1", "COMMENT", false, 10, Some("F2P-1")), n_full("2", "COMMENT", true, -5, Some("F2P-1"))];
        assert!(SlackMessages::select_notifications(&list, &mut HashSet::new(), started(), true, false).is_empty());
    }

    #[test]
    fn 能審核時_有新的變更申請通知不傳_由審核訊息取代() {
        let list = [nt("1", "CHANGE_REQUESTED")];
        assert!(SlackMessages::select_notifications(&list, &mut HashSet::new(), started(), true, true).is_empty());
        assert_eq!(1, SlackMessages::select_notifications(&list, &mut HashSet::new(), started(), true, false).len());
    }

    #[test]
    fn 待審申請_開程式後的才傳_同一筆只傳一次() {
        let mut seen = HashSet::new();
        let send = SlackMessages::select_reviews(&[cr("a", "UPDATE"), cr_at("b", "UPDATE", -1)], &mut seen, started());
        assert_eq!(vec!["a"], send.iter().map(|c| c.id.as_str()).collect::<Vec<_>>());
        assert!(SlackMessages::select_reviews(&[cr("a", "UPDATE")], &mut seen, started()).is_empty());
    }

    #[test]
    fn 審核訊息_有核准駁回_新增任務的駁回要警告() {
        let p = SlackMessages::from_review(&cr("a", "CREATE"), "https://x/t", now());
        assert_eq!("cr:a", p.key);
        assert!(p.reject_warning.as_deref().unwrap().contains("駁回會刪除 F2P-2 做登入"));
        let json = json_of(&SlackMessages::blocks(&p, None));
        assert!(json.contains("\"action_id\":\"approve\""));
        assert!(json.contains("\"action_id\":\"approve_watch\""));
        assert!(json.contains("\"action_id\":\"reject\""));
        assert!(json.contains("\"value\":\"cr:a\""));
        assert!(json.contains("\"url\":\"https://x/t\""));
    }

    #[test]
    fn 刪除任務的申請_不能核准並關注() {
        let p = SlackMessages::from_review(&cr("a", "DELETE"), "u", now());
        assert!(!json_of(&SlackMessages::blocks(&p, None)).contains("approve_watch"));
    }

    #[test]
    fn 測試結果_被退回是開始修_待復驗是通過退回() {
        let fix = SlackMessages::from_notification(&nt("1", "TEST_RESULT_REOPENED"), "u", now());
        let retest = SlackMessages::from_notification(&nt("2", "TEST_RESULT_READY_TO_RETEST"), "u", now());
        assert_eq!(("fix", "retest"), (fix.kind.as_str(), retest.kind.as_str()));
        assert!(json_of(&SlackMessages::blocks(&fix, None)).contains("\"action_id\":\"fix\""));
        let r = json_of(&SlackMessages::blocks(&retest, None));
        assert!(r.contains("\"action_id\":\"pass\""));
        assert!(r.contains("\"action_id\":\"reopen\""));
    }

    #[test]
    fn 處理過了_按鈕換成結果() {
        let mut p = SlackMessages::from_notification(&n("1"), "u", now());
        p.done = Some("✓ 已標為已讀".to_owned());
        let json = json_of(&SlackMessages::blocks(&p, None));
        assert!(!json.contains("actions"));
        assert!(json.contains("已標為已讀"));
    }

    #[test]
    fn 失敗_保留按鈕並顯示原因() {
        let json =
            json_of(&SlackMessages::blocks(&SlackMessages::from_notification(&n("1"), "u", now()), Some("連不上")));
        assert!(json.contains("\"action_id\":\"read\""));
        assert!(json.contains("⚠ 連不上"));
    }

    #[test]
    fn 很多則合成一則_全部標為已讀() {
        let p = SlackMessages::from_notices(&[n("1"), n("2"), n("3"), n("4")], "u", now());
        assert_eq!(vec!["1", "2", "3", "4"], p.notification_ids);
        assert_eq!("有 4 則新通知", p.title);
    }

    #[test]
    fn 文字要跳脫() {
        assert_eq!("a &lt;b&gt; &amp; c", SlackMessages::escape("a <b> & c"));
    }

    #[test]
    fn 表單_駁回說明可不填_退回一定要填() {
        let p = SlackMessages::from_review(&cr("a", "CREATE"), "u", now());
        let reject = json_of(&SlackMessages::reason_modal(&p, SlackMessages::REJECT));
        assert!(reject.contains("\"optional\":true"));
        assert!(reject.contains("\"private_metadata\":\"reject|cr:a\""));
        assert!(reject.contains("駁回會刪除"));
        assert!(json_of(&SlackMessages::reason_modal(&p, SlackMessages::REOPEN)).contains("\"optional\":false"));
    }

    #[test]
    fn 解析按鈕() {
        let payload: Value = serde_json::from_str(
            r#"{"type":"block_actions","user":{"id":"U1"},"trigger_id":"t1",
             "container":{"type":"message","message_ts":"111.2","channel_id":"D1"},
             "actions":[{"action_id":"approve","value":"cr:a","type":"button"}]}"#,
        )
        .unwrap();
        let i = SlackMessages::parse_interaction(&payload);
        assert_eq!(
            ("block_actions", Some("U1"), Some("approve"), Some("cr:a"), Some("t1"), Some("D1"), Some("111.2")),
            (
                i.r#type.as_str(),
                i.user_id.as_deref(),
                i.action_id.as_deref(),
                i.value.as_deref(),
                i.trigger_id.as_deref(),
                i.channel.as_deref(),
                i.ts.as_deref()
            )
        );
    }

    #[test]
    fn 解析表單送出() {
        let payload: Value = serde_json::from_str(
            r#"{"type":"view_submission","user":{"id":"U1"},
             "view":{"callback_id":"reason","private_metadata":"reopen|n:9",
                     "state":{"values":{"reason":{"text":{"type":"plain_text_input","value":"還是會當"}}}}}}"#,
        )
        .unwrap();
        let i = SlackMessages::parse_interaction(&payload);
        assert_eq!(
            (Some("reason"), Some("reopen|n:9"), Some("還是會當")),
            (i.callback_id.as_deref(), i.metadata.as_deref(), i.reason.as_deref())
        );
    }

    // ───────────── 補充：文字一字不改 ─────────────

    #[test]
    fn 審核訊息的內容() {
        let p = SlackMessages::from_review(&cr("a", "CREATE"), "u", now());
        assert_eq!("待審核・新增任務", p.title);
        assert_eq!("Amy：F2P-2 做登入\n• 狀態：待處理 → 進行中\n理由：趕時程", p.body);
        assert_eq!(Some("駁回會刪除 F2P-2 做登入（已經記錄 1 小時 30 分 工時）"), p.reject_warning.as_deref());
        assert_eq!("待審核・新增任務：Amy：F2P-2 做登入", SlackMessages::fallback(&p));
        assert_eq!(
            (Some("a"), Some("cr:a"), true),
            (p.change_request_id.as_deref(), p.toast_tag.as_deref(), p.can_watch)
        );
    }

    #[test]
    fn 一般通知的內容_沒有任務只放標籤() {
        let p = SlackMessages::from_notification(&n_full("7", "COMMENT", true, 10, None), "u", now());
        assert_eq!(
            ("n:7", "notice", "標籤", "小明：內容"),
            (p.key.as_str(), p.kind.as_str(), p.title.as_str(), p.body.as_str())
        );
        let p = SlackMessages::from_notification(&n("7"), "u", now());
        assert_eq!("標籤・F2P-1", p.title);
        let p = SlackMessages::from_notices(&(1..=12).map(|i| n(&i.to_string())).collect::<Vec<_>>(), "u", now());
        assert_eq!("ns:20261103100000:1", p.key);
        assert!(p.body.starts_with("• 標籤：內容\n"));
        assert!(p.body.ends_with("\n…"));
        assert_eq!(11, p.body.split('\n').count());
    }

    #[test]
    fn blocks的格式() {
        let mut p = SlackMessages::from_notification(&n("1"), "https://x", now());
        p.body = "<b>".to_owned();
        assert_eq!(
            r#"[{"type":"section","text":{"type":"mrkdwn","text":"*標籤・F2P-1*\n&lt;b&gt;"}},{"type":"actions","block_id":"act","elements":[{"type":"button","text":{"type":"plain_text","text":"標為已讀","emoji":true},"action_id":"read","value":"n:1","style":"primary"},{"type":"button","text":{"type":"plain_text","text":"封存","emoji":true},"action_id":"archive","value":"n:1"},{"type":"button","text":{"type":"plain_text","text":"開啟","emoji":true},"action_id":"open","url":"https://x"}]}]"#,
            json_of(&SlackMessages::blocks(&p, None))
        );
    }

    #[test]
    fn 太長的內容截斷() {
        let mut p = SlackMessages::from_notification(&n("1"), "u", now());
        p.body = "字".repeat(4000);
        let blocks = SlackMessages::blocks(&p, None);
        let text = blocks[0]["text"]["text"].as_str().unwrap();
        assert_eq!(2901, text.encode_utf16().count());
        assert!(text.ends_with('…'));
    }
}
