//! 會議、待辦、提醒、隨手記、通知、工時修改。對應 C# `Mcp/HayakuApi.Extras.cs`。

use chrono::NaiveDate;
use serde_json::{Map, Value, json};
use std::sync::atomic::Ordering;

use super::HayakuApi;
use crate::json::*;
use crate::models::*;
use crate::transport::McpError;

fn notification_label(t: &str) -> Option<&'static str> {
    Some(match t {
        "TASK_ASSIGNED" => "被指派任務",
        "MENTIONED" => "被提及",
        "COMMENT_ON_MY_TASK" => "任務有新聊天",
        "TASK_DUE_SOON" => "即將到期",
        "TASK_OVERDUE" => "已逾期",
        "STATUS_CHANGED" => "狀態被變更",
        "ADDED_TO_PROJECT" => "被加入專案",
        "TIMER_AUTO_STOPPED" => "計時被自動停止",
        "TIMER_RESUME_PROMPT" => "要繼續計時嗎",
        "TASK_REMINDER" => "提醒",
        "CHANGE_REQUESTED" => "有新的變更申請",
        "CHANGE_REVIEWED" => "申請已審核",
        "DEPENDENCY_UNBLOCKED" => "可以開始了",
        "DEPENDENCY_TRIGGER_BLOCKED" => "自動變更被擋住",
        "HOURS_UNFILLED" => "補工時提醒",
        "HOURS_UNFILLED_WEEKLY" => "上週工時沒填滿彙總",
        "MEETING_STARTING" => "會議即將開始",
        "MEETING_INVITED" => "被邀請參加會議",
        "MEETING_CHANGED" => "會議有變更",
        "TASK_MOVED" => "任務被移動",
        "FEEDBACK_NEW" => "新的問題回報",
        "FEEDBACK_CREATED" => "新的問題回報",
        "WATCHED_ASSIGNEE_CHANGED" => "關注的任務換了執行者",
        "WATCHED_DUE_CHANGED" => "關注的任務改了預計結束",
        "WATCHER_ADDED" => "被加為關注者",
        "WATCHED_TASK" => "我關注的任務",
        "AUTOMATION" => "自動化通知",
        "DAILY_DIGEST" => "每日摘要",
        "MILESTONE_DUE_SOON" => "里程碑快到了",
        "BUDGET_THRESHOLD" => "工時預算快用完",
        "THREAD_REPLY" => "討論串回覆",
        "CHANNEL_MESSAGE" => "頻道訊息",
        "CHAT_REACTION" => "訊息的表情回應",
        "POLL_CLOSING" => "投票快截止",
        "POLL_CLOSED" => "我的投票已結束",
        "FORUM_POST" => "論壇新貼文",
        "SPACE_PAGE_COMMENT" => "空間頁面有新評論",
        "SECURITY_ALERT" => "帳號安全",
        "DOC_MENTIONED" => "在文件被提及",
        "DOC_TODO_ASSIGNED" => "被指派待辦",
        "DOC_TODO_DONE" => "我指派的待辦完成了",
        _ => return None,
    })
}

/// `Shorten`：樣本紀錄只留前 600 字。
fn shorten(s: &str) -> String {
    if utf16_len(s) > 600 { format!("{}…", truncate_utf16(s, 600)) } else { s.to_owned() }
}

impl HayakuApi {
    // ───────────── 會議 ─────────────

    /// 期間內的會議；mine=false 連我看得到的專案裡、我不是參加者的會議也列出來（C# 預設 true）。
    pub async fn list_meetings(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        mine: bool,
    ) -> Result<Vec<MeetingItem>, McpError> {
        let r = self.call("list_meetings", json!({ "from": iso(from), "to": iso(to), "mine": mine })).await?;
        Ok(Self::parse_meetings(&r))
    }

    pub async fn join_meeting(&self, meeting: &str) -> Result<(), McpError> {
        self.call("join_meeting", json!({ "meeting": meeting })).await.map(drop)
    }

    pub fn parse_meetings(r: &Value) -> Vec<MeetingItem> {
        let mut list = Vec::new();
        let Some(Value::Array(arr)) = prop(r, "meetings") else {
            return list;
        };
        for m in arr {
            let (Some(start), Some(end)) = (date(m, "startAt"), date(m, "endAt")) else { continue };
            list.push(MeetingItem {
                key: string(m, "meeting").or_else(|| string(m, "id")).unwrap_or_default(),
                id: string(m, "id").unwrap_or_default(),
                title: string(m, "title").unwrap_or_default(),
                project: string(m, "project").unwrap_or_default(),
                start_at: start,
                end_at: end,
                location: string(m, "location"),
                my_status: string(m, "me"),
            });
        }
        list.sort_by_key(|a| a.start_at);
        list
    }

    // ───────────── 待辦、提醒、隨手記 ─────────────

    pub async fn list_todos(&self) -> Result<Vec<TodoItem>, McpError> {
        let r = self.call("list_my_todos", json!({})).await?;
        let mut list = Vec::new();
        for t in items(&r, &["items", "todos"]) {
            if !self.logged_todo_sample.swap(true, Ordering::Relaxed) {
                log::info!("待辦格式樣本：{}", shorten(&raw_text(t)));
            }
            let (Some(id), Some(text)) = (string(t, "id"), first_str(t, &["text", "title", "content", "body"])) else {
                continue;
            };
            list.push(TodoItem {
                id,
                text: text.trim().to_owned(),
                task_code: first_key(t, &["taskCode", "task"]),
                remind_at: first_date(t, &["remindAt", "reminderAt", "dueAt"]),
                source: first_string(t, &["source", "page", "doc"]),
                assigned_by: first_name(t, "assignedBy"),
            });
        }
        Ok(list)
    }

    pub async fn complete_todo(&self, todo_id: &str) -> Result<(), McpError> {
        self.call("complete_my_todo", json!({ "todoId": todo_id })).await.map(drop)
    }

    pub async fn list_reminders(&self) -> Result<Vec<ReminderItem>, McpError> {
        let r = self.call("list_reminders", json!({ "tab": "upcoming" })).await?;
        let mut list = Vec::new();
        for t in items(&r, &["reminders", "items"]) {
            if !self.logged_reminder_sample.swap(true, Ordering::Relaxed) {
                log::info!("提醒格式樣本：{}", shorten(&raw_text(t)));
            }
            let Some(id) = string(t, "id") else { continue };
            list.push(ReminderItem {
                id,
                text: first_string(t, &["text", "title", "message", "note", "target"])
                    .unwrap_or_else(|| "提醒".to_owned()),
                at: first_date(t, &["nextAt", "remindAt", "at", "fireAt", "time", "dueAt"]),
                task: first_key(t, &["task", "taskCode"]),
            });
        }
        let max = max_value();
        list.sort_by_key(|a| a.at.unwrap_or(max));
        Ok(list)
    }

    pub async fn snooze_reminder(&self, id: &str, minutes: i32) -> Result<(), McpError> {
        self.call("snooze_reminder", json!({ "id": id, "minutes": minutes })).await.map(drop)
    }

    pub async fn quick_capture(&self, text: &str, task_code: Option<&str>, todo: bool) -> Result<(), McpError> {
        let args = match task_code {
            None => json!({ "text": text, "todo": todo }),
            Some(code) => json!({ "text": text, "todo": todo, "taskCode": code }),
        };
        self.call("quick_capture", args).await.map(drop)
    }

    // ───────────── 通知 ─────────────

    pub async fn list_notifications(&self) -> Result<Vec<NotificationItem>, McpError> {
        Ok(Self::parse_notifications(&self.call("list_notifications", json!({})).await?))
    }

    /// 我的討論（tab=discussions）：團隊空間的評論、聊天討論串與 @ 我，同一段討論一組（ids 是那段的全部通知、users 是說過話的人）。
    /// unresolved_only：只留沒解決的評論串（聊天照樣列）；source：space＝空間與文件的評論、chat＝聊天，None 是全部。
    pub async fn list_discussions(
        &self,
        unresolved_only: bool,
        source: Option<&str>,
    ) -> Result<Vec<NotificationItem>, McpError> {
        let mut args = Map::new();
        args.insert("tab".into(), json!("discussions"));
        if unresolved_only {
            args.insert("unresolvedOnly".into(), json!(true));
        }
        if let Some(s) = source {
            args.insert("source".into(), json!(s));
        }
        Ok(Self::parse_notifications(&self.call("list_notifications", Value::Object(args)).await?))
    }

    pub async fn mark_notifications_read(&self, ids: Option<&[String]>) -> Result<(), McpError> {
        let args = match ids {
            Some(ids) if !ids.is_empty() => json!({ "ids": ids }),
            _ => json!({}),
        };
        self.call("mark_notifications_read", args).await.map(drop)
    }

    pub fn parse_notifications(r: &Value) -> Vec<NotificationItem> {
        let mut list = Vec::new();
        for group in items(r, &["items"]) {
            let n = if obj(group, "latest").is_object() { obj(group, "latest") } else { group };
            let (Some(id), Some(type_)) = (string(n, "id"), str_(n, "type")) else { continue };
            let unread_count = prop(group, "unreadCount").and_then(as_i32).unwrap_or(0);
            let count = prop(group, "count").and_then(as_i32).unwrap_or(1);
            let project = obj(n, "project");
            let task = obj(n, "task");
            let payload = obj(n, "payload");
            let project_key = if project.is_object() { string(project, "key") } else { None };
            let mut task_key = None;
            let mut meeting_id = None;
            if task.is_object() {
                if let Some(pk) = &project_key
                    && let Some(number) = prop(task, "number").and_then(as_i32)
                {
                    task_key = Some(format!("{pk}-{number}"));
                }
                if str_(task, "kind") == Some("MEETING") {
                    meeting_id = string(task, "id");
                }
            }
            let mut text = if payload.is_object() { string(payload, "task") } else { None };
            if text.is_none() && task.is_object() {
                text = string(task, "title");
            }
            if type_ == "DAILY_DIGEST" {
                text = Some("今天到期、逾期、等你審核的事情與會議".to_owned());
            }
            if text.is_none() && project.is_object() {
                text = string(project, "name");
            }
            let actor = match prop(n, "actor") {
                Some(a @ Value::Object(_)) => string(a, "displayName"),
                _ => None,
            };
            let mut feedback: Option<i32> = None;
            if type_.starts_with("FEEDBACK") {
                for source in [payload, prop(n, "feedback").unwrap_or(&NULL)] {
                    match source {
                        Value::Number(_) => {
                            if let Some(direct) = as_i32(source) {
                                feedback = feedback.or(Some(direct));
                            }
                        }
                        Value::Object(_) => {
                            feedback = feedback.or_else(|| {
                                ["number", "feedbackNumber", "feedback"].iter().find_map(|p| match prop(source, p) {
                                    Some(v @ Value::Number(_)) => as_i32(v),
                                    _ => None,
                                })
                            });
                        }
                        _ => {}
                    }
                }
                if text.is_none() && payload.is_object() {
                    text = first_string(payload, &["title", "summary"]);
                }
            }
            let label = match notification_label(type_) {
                Some(l) => l.to_owned(),
                None if type_.starts_with("FEEDBACK") => "問題回報".to_owned(),
                None if type_.to_uppercase().contains("TEST") => Self::test_notification_label(type_).to_owned(),
                None => type_.to_owned(),
            };
            let unread = unread_count > 0 || matches!(prop(n, "readAt"), Some(Value::Null));
            // 討論分頁才有：ids、users、resolved、quote（評論串引用的原文）或 rootExcerpt
            let ids = items(group, &["ids"]).iter().filter_map(|x| x.as_str().map(str::to_owned)).collect();
            let users = items(group, &["users"])
                .iter()
                .filter_map(|x| match x.as_str() {
                    Some(s) => Some(s),
                    None => first_str(x, &["displayName", "username"]),
                })
                .filter(|x| !is_blank(x))
                .map(str::to_owned)
                .collect();
            let resolved = match prop(group, "resolved") {
                Some(Value::Bool(b)) => Some(*b),
                _ => None,
            };
            let quote = first_string(group, &["quote", "rootExcerpt"])
                .or_else(|| if payload.is_object() { string(payload, "excerpt") } else { None });
            list.push(NotificationItem {
                id,
                r#type: type_.to_owned(),
                label,
                text: text.unwrap_or_default(),
                actor,
                task_key,
                project_key,
                meeting_id,
                created_at: date(n, "createdAt").unwrap_or_else(now),
                unread,
                count,
                feedback,
                ids: Some(ids),
                users: Some(users),
                resolved,
                quote,
            });
        }
        list
    }

    /// 測試結果類通知的標題（代碼沒有固定清單，看關鍵字）。
    pub fn test_notification_label(type_: &str) -> &'static str {
        let t = type_.to_uppercase();
        let has = |w: &str| t.contains(w);
        if has("REOPEN") || has("REJECT") || has("RETURN") {
            "測試結果被退回"
        } else if has("RETEST") || has("VERIFY") || has("READY") {
            "測試結果待復驗"
        } else if has("ASSIGN") {
            "被指派修測試結果"
        } else if has("APPROV") {
            "改測試結果執行者的申請"
        } else if has("PASS") || has("CLOSE") || has("DONE") {
            "測試結果已通過"
        } else {
            "測試結果有更新"
        }
    }

    // ───────────── 工時（補登、修改、刪除） ─────────────

    pub async fn list_day_entries(&self, day_: NaiveDate) -> Result<Vec<TimeEntry>, McpError> {
        self.list_time_entries(day_, day_).await
    }

    pub async fn add_time_entry(
        &self,
        task: &str,
        start: DateTimeOffset,
        end: DateTimeOffset,
        note: Option<&str>,
    ) -> Result<(), McpError> {
        let args = match note.filter(|n| !is_blank(n)) {
            None => json!({ "task": task, "start": iso_time(&start), "end": iso_time(&end) }),
            Some(note) => json!({ "task": task, "start": iso_time(&start), "end": iso_time(&end), "note": note }),
        };
        self.call("add_time_entry", args).await.map(drop)
    }

    /// start、end 為 None 不改；note 為 None 不改、空字串是清空。
    pub async fn update_time_entry(
        &self,
        id: &str,
        start: Option<DateTimeOffset>,
        end: Option<DateTimeOffset>,
        note: Option<&str>,
    ) -> Result<(), McpError> {
        let mut args = Map::new();
        args.insert("id".into(), json!(id));
        if let Some(s) = start {
            args.insert("start".into(), json!(iso_time(&s)));
        }
        if let Some(e) = end {
            args.insert("end".into(), json!(iso_time(&e)));
        }
        if let Some(n) = note {
            args.insert("note".into(), if n.is_empty() { Value::Null } else { json!(n) });
        }
        self.call("update_time_entry", Value::Object(args)).await.map(drop)
    }

    pub async fn delete_time_entry(&self, id: &str) -> Result<(), McpError> {
        self.call("delete_time_entry", json!({ "id": id })).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 ExtrasTests（ExtrasParseTests）的 API 解析部分；樣本取自實際 Hayaku MCP 回應

    #[test]
    fn parse_meetings_讀取會議() {
        let list = HayakuApi::parse_meetings(&j(r#"
            {"meetings":[{"meeting":"REVIVAL-9","id":"4aa94b66","title":"HAYAKU專案管理系統介紹會議-2","project":"REVIVAL",
              "startAt":"2026-10-27T07:35:00.000Z","endAt":"2026-10-27T09:00:00.000Z","location":null,"recurring":false,
              "attendees":8,"inMeeting":0,"me":"已離開"}],"more":false}"#));
        assert_eq!(list.len(), 1);
        let m = &list[0];
        assert_eq!(m.key, "REVIVAL-9");
        assert_eq!(m.my_status.as_deref(), Some("已離開"));
        assert!(!m.in_meeting());
        assert_eq!((m.end_at - m.start_at).num_minutes(), 85);
    }

    #[test]
    fn parse_notifications_組出任務編號與文字() {
        let list = HayakuApi::parse_notifications(&j(r#"
            {"items":[
              {"count":1,"unreadCount":1,"latest":{"id":"n1","type":"TASK_ASSIGNED",
                "actor":{"username":"hayaku-admin","displayName":"系統管理員"},
                "project":{"key":"F2P","name":"F2P-臉部辨識結合印表機"},
                "task":{"id":"t1","number":96,"title":"優化上傳","deleted":false,"kind":"TASK"},
                "payload":{"task":"F2P-96 優化上傳","count":1},"readAt":null,"createdAt":"2026-10-27T07:34:51.828Z"}},
              {"count":1,"unreadCount":0,"latest":{"id":"n2","type":"MEETING_INVITED","actor":{"displayName":"羅瑞騰"},
                "project":{"key":"REVIVAL","name":"Revival"},
                "task":{"id":"m1","number":9,"title":"介紹會議","kind":"MEETING"},
                "payload":{"task":"REVIVAL-9 介紹會議"},"readAt":"2026-10-27T08:08:29.851Z","createdAt":"2026-10-27T07:38:01.138Z"}}
            ]}"#));
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].label, "被指派任務");
        assert_eq!(list[0].task_key.as_deref(), Some("F2P-96"));
        assert_eq!(list[0].text, "F2P-96 優化上傳");
        assert_eq!(list[0].actor.as_deref(), Some("系統管理員"));
        assert!(list[0].unread);
        assert_eq!(list[1].meeting_id.as_deref(), Some("m1"));
        assert!(!list[1].unread);
    }

    // 對應 TestingTests.ParseNotifications_測試結果與問題回報（TimerEngine.NotificationTarget 的斷言屬於 hb-core）
    #[test]
    fn parse_notifications_測試結果與問題回報() {
        let list = HayakuApi::parse_notifications(&j(r#"
            {"items":[
              {"latest":{"id":"n1","type":"TEST_RESULT_REOPENED","project":{"key":"F2P"},"task":{"number":140,"title":"登入頁跑版"},"readAt":null}},
              {"latest":{"id":"n2","type":"TEST_RESULT_RETEST","project":{"key":"F2P"},"task":{"number":141,"title":"按鈕沒反應"},"readAt":null}},
              {"latest":{"id":"n3","type":"FEEDBACK_NEW","payload":{"number":128,"title":"匯出失敗"},"readAt":null}}
            ]}"#));
        assert!(list[0].is_reopened());
        assert_eq!((list[0].task_key.as_deref(), list[0].label.as_str()), (Some("F2P-140"), "測試結果被退回"));
        assert!(list[1].is_ready_to_retest());
        assert_eq!(list[1].label, "測試結果待復驗");
        assert_eq!((list[2].feedback, list[2].text.as_str()), (Some(128), "匯出失敗"));
    }

    // 對應 Hayaku14Tests.通知的討論分頁_一段討論一組（NotificationRowVm 的斷言屬於 UI）
    #[test]
    fn 通知的討論分頁_一段討論一組() {
        let list = HayakuApi::parse_notifications(&j(r#"
            {"items":[{"latest":{"id":"n2","type":"MENTIONED","actor":{"displayName":"羅瑞騰"},
                         "project":{"key":"TEST"},"task":{"number":11,"title":"42","kind":"TASK"},
                         "payload":{"task":"TEST-11 42","excerpt":"@harry.gu 你做好了嗎？"},"readAt":null,"createdAt":"2026-10-27T07:51:11.294Z"},
                       "count":2,"unreadCount":1,"ids":["n1","n2"],"users":["羅瑞騰","吳景涵"],"resolved":false,"rootExcerpt":null}]}"#));
        assert_eq!(list.len(), 1);
        let n = &list[0];
        assert_eq!(n.all_ids(), vec!["n1", "n2"]);
        assert_eq!(n.users.as_deref(), Some(&["羅瑞騰".to_owned(), "吳景涵".to_owned()][..]));
        assert_eq!(n.resolved, Some(false));
        assert_eq!(n.quote.as_deref(), Some("@harry.gu 你做好了嗎？"));
        assert!(n.unread);
    }

    #[test]
    fn 收件匣通知沒有_ids_時_all_ids_是自己() {
        let n = NotificationItem::new("n", "TASK_ASSIGNED", "", "", None, None, None, None, min_value(), true, 1);
        assert_eq!(n.all_ids(), vec!["n"]);
    }

    #[test]
    fn 補登工時_時間格式() {
        let fake = std::sync::Arc::new(FakeTransport::default());
        let api = HayakuApi::new(fake.clone());
        let start = parse_date_time_offset("2026-11-02T09:00:00+08:00").expect("start");
        let end = parse_date_time_offset("2026-11-02T10:30:00+08:00").expect("end");
        block_on(api.add_time_entry("F2P-1", start, end, Some("  "))).expect("add");
        block_on(api.update_time_entry("x", None, None, Some(""))).expect("update");
        let calls = fake.calls.lock().expect("lock");
        assert_eq!(
            calls[0].1,
            json!({"task":"F2P-1","start":"2026-11-02T09:00:00+08:00","end":"2026-11-02T10:30:00+08:00"})
        );
        assert_eq!(calls[1].1, json!({"id":"x","note":null}));
    }
}
