//! 釘選托盤、切換計時、移動任務、變更申請、專案動態、工作負荷。對應 C# `Mcp/HayakuApi.Projects.cs`。

use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use super::HayakuApi;
use crate::json::*;
use crate::models::*;
use crate::transport::McpError;

/// changes 欄位名稱 → 中文（C# `ChangeLabels`，TaskExtras 的異動紀錄也用）。
pub(crate) fn change_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "title" => "名稱",
        "statusId" | "status" => "狀態",
        "priority" => "優先級",
        "plannedStart" => "預計開始",
        "plannedEnd" => "預計結束",
        "actualStart" => "實際開始",
        "actualEnd" => "實際結束",
        "estimatedHours" | "estimatedMinutes" | "estimate" => "預計工時",
        "assigneeIds" | "assignees" => "執行者",
        "note" => "備註",
        "parentId" => "上層",
        "milestoneIds" => "里程碑",
        "customFields" => "自訂欄位",
        "isCategory" => "當作分類",
        _ => return None,
    })
}

impl HayakuApi {
    // ───────────── 釘選托盤 ─────────────

    pub async fn list_pins(&self) -> Result<Vec<PinItem>, McpError> {
        Ok(Self::parse_pins(&self.call("list_pins", json!({})).await?))
    }

    /// 釘到托盤（加在最後，最多 20 個，Hayaku 1.5.0 前是 10 個）；回傳釘選後的整份清單。
    pub async fn pin_task(&self, task: &str) -> Result<Vec<PinItem>, McpError> {
        Ok(Self::parse_pins(&self.call("pin_task", json!({ "task": task })).await?))
    }

    pub async fn unpin_task(&self, task: &str) -> Result<Vec<PinItem>, McpError> {
        Ok(Self::parse_pins(&self.call("unpin_task", json!({ "task": task })).await?))
    }

    /// 托盤：任務是 { task }，會議可能是 { meeting }、{ task, kind: MEETING } 或帶 startAt／endAt；
    /// 已取消可能是 cancelled: true 或狀態「已取消」。
    pub fn parse_pins(r: &Value) -> Vec<PinItem> {
        items(r, &["pins"])
            .iter()
            .filter_map(|p| {
                let meeting_obj = obj(p, "meeting");
                let key = first_key(p, &["task", "meeting", "key", "number"]).or_else(|| {
                    if meeting_obj.is_object() { first_key(meeting_obj, &["meeting", "task", "key"]) } else { None }
                })?;
                let source = if meeting_obj.is_object() { meeting_obj } else { p };
                let kind = first_str(p, &["kind", "type", "pinType"]).unwrap_or("");
                let is_meeting = kind.to_uppercase().contains("MEETING")
                    || kind == "會議"
                    || prop(p, "meeting").is_some()
                    || date(source, "startAt").is_some();
                let status =
                    first_string(source, &["status", "state"]).or_else(|| string(p, "status")).unwrap_or_default();
                let cancelled = bool_(source, "cancelled")
                    || bool_(source, "canceled")
                    || date(source, "cancelledAt").is_some()
                    || matches!(status.as_str(), "已取消" | "CANCELLED" | "CANCELED");
                Some(PinItem {
                    key,
                    title: first_string(source, &["title", "name"]).or_else(|| string(p, "title")).unwrap_or_default(),
                    project: string(p, "project").or_else(|| string(source, "project")).unwrap_or_default(),
                    status,
                    today_minutes: minutes_of(p, &["today"]),
                    running: bool_(p, "running"),
                    can_time: !is_false(p, "canTime"),
                    is_meeting,
                    start_at: date(source, "startAt"),
                    end_at: date(source, "endAt"),
                    cancelled,
                })
            })
            .collect()
    }

    /// 切換計時（同網頁托盤的 ▶）：同一個交易裡暫停我所有正在計時的，開始這個；keep_others 是同時計時。
    pub async fn switch_timer(&self, task: &str, keep_others: bool) -> Result<(), McpError> {
        let args = if keep_others { json!({ "task": task, "keepOthers": true }) } else { json!({ "task": task }) };
        self.call("switch_timer", args).await.map(drop)
    }

    // ───────────── 移動任務 ─────────────

    /// 換上層（parent 為 None＝移到第一層）；只能在同一個專案裡。
    pub async fn move_task(&self, task: &str, parent: Option<&str>) -> Result<(), McpError> {
        self.call("move_task", json!({ "task": task, "parent": parent })).await.map(drop)
    }

    /// 某個任務的全部計時段（每個人的都有，user 是顯示名稱）。
    pub async fn list_task_entries(&self, task: &str) -> Result<Vec<TimeEntry>, McpError> {
        Ok(Self::parse_entries(&self.call("list_time_entries", json!({ "task": task })).await?, "entries"))
    }

    /// 補登一段工時，回傳新計時段的 id（回應裡沒有就是 None）。
    pub async fn add_time_entry_with_id(
        &self,
        task: &str,
        start: DateTimeOffset,
        end: DateTimeOffset,
        note: Option<&str>,
    ) -> Result<Option<String>, McpError> {
        let mut args = Map::new();
        args.insert("task".into(), json!(task));
        args.insert("start".into(), json!(iso_time(&start)));
        args.insert("end".into(), json!(iso_time(&end)));
        if let Some(n) = note.filter(|n| !is_blank(n)) {
            args.insert("note".into(), json!(n));
        }
        let r = self.call("add_time_entry", Value::Object(args)).await?;
        if !r.is_object() {
            return Ok(None);
        }
        Ok(string(&r, "id").or_else(|| match prop(&r, "entry") {
            Some(e @ Value::Object(_)) => string(e, "id"),
            _ => None,
        }))
    }

    // ───────────── 變更申請 ─────────────

    /// status：PENDING（預設）、APPROVED、REJECTED、WITHDRAWN；不給專案＝看得到的全部專案。
    pub async fn list_change_requests(&self, status: &str) -> Result<Vec<ChangeRequest>, McpError> {
        Ok(Self::parse_change_requests(&self.call("list_change_requests", json!({ "status": status })).await?))
    }

    pub fn parse_change_requests(r: &Value) -> Vec<ChangeRequest> {
        let mut list = Vec::new();
        for c in items(r, &["items"]) {
            let Some(id) = string(c, "id") else { continue };
            let task = obj(c, "task");
            let has_task = task.is_object();
            list.push(ChangeRequest {
                id,
                project: string(c, "project").unwrap_or_default(),
                r#type: string(c, "type").unwrap_or_default(),
                status: string(c, "status").unwrap_or_default(),
                task_key: if has_task { string(task, "key") } else { None },
                task_title: if has_task { string(task, "title").unwrap_or_default() } else { String::new() },
                requester: prop(c, "requester").and_then(person),
                reviewer: prop(c, "reviewer").and_then(person),
                created_at: date(c, "createdAt").unwrap_or_else(now),
                changes: Self::change_texts(c),
                reason: string(c, "reason"),
                review_note: string(c, "reviewNote"),
                logged_minutes: prop(c, "loggedSeconds").and_then(as_i32).map_or(0, |s| s / 60),
                descendant_count: prop(c, "descendantCount").and_then(as_i32).unwrap_or(0),
            });
        }
        list
    }

    /// changes 物件 → 「預計結束：2026-11-05」這類文字；值是 id 的只寫「有變更」。
    pub fn change_texts(c: &Value) -> Vec<String> {
        let Some(Value::Object(changes)) = prop(c, "changes") else {
            return Vec::new();
        };
        changes
            .iter()
            .map(|(name, v)| {
                let label = change_label(name).unwrap_or(name);
                let value = match v {
                    Value::Null => "清空".to_owned(),
                    Value::Bool(true) => "是".to_owned(),
                    Value::Bool(false) => "否".to_owned(),
                    Value::Number(_) if name == "estimatedMinutes" && as_f64(v).is_some() => {
                        format!("{} 小時", round_even(as_f64(v).unwrap_or(0.0) / 60.0, 2))
                    }
                    Value::Number(_) => raw_text(v),
                    Value::String(_) if name.ends_with("Id") => "有變更".to_owned(),
                    Value::String(s) => {
                        if utf16_len(s) > 40 {
                            format!("{}…", truncate_utf16(s, 40))
                        } else {
                            s.clone()
                        }
                    }
                    _ => "有變更".to_owned(),
                };
                format!("{label}：{value}")
            })
            .collect()
    }

    /// 核准或拒絕；watch：核准並關注那個任務；note：給申請人的說明；reject_action：拒絕新增申請時 delete（刪掉那個任務，C# 預設 null）。
    pub async fn review_change_request(
        &self,
        id: &str,
        approve: bool,
        watch: bool,
        note: Option<&str>,
        reject_action: Option<&str>,
    ) -> Result<(), McpError> {
        let mut args = Map::new();
        args.insert("id".into(), json!(id));
        args.insert("decision".into(), json!(if approve { "approve" } else { "reject" }));
        if !approve && let Some(a) = reject_action {
            args.insert("rejectAction".into(), json!(a));
        }
        if approve && watch {
            args.insert("watch".into(), json!(true));
        }
        if let Some(n) = note.filter(|n| !is_blank(n)) {
            args.insert("note".into(), json!(n));
        }
        self.call("review_change_request", Value::Object(args)).await.map(drop)
    }

    pub async fn withdraw_change_request(&self, id: &str) -> Result<(), McpError> {
        self.call("withdraw_change_request", json!({ "id": id })).await.map(drop)
    }

    // ───────────── 專案動態、工作負荷 ─────────────

    /// types：progress、assign、talk、other；project 為 None 是看得到的全部專案。
    /// members：只看這些人（帳號、顯示名稱或 "me"，C# 預設 null）。
    pub async fn project_activity(
        &self,
        project: Option<&str>,
        from: NaiveDate,
        to: NaiveDate,
        types: &[String],
        members: Option<&[String]>,
    ) -> Result<ActivityFeed, McpError> {
        let mut args = Map::new();
        args.insert("from".into(), json!(iso(from)));
        args.insert("to".into(), json!(iso(to)));
        args.insert("types".into(), json!(types));
        if let Some(p) = project {
            args.insert("project".into(), json!(p));
        }
        if let Some(m) = members.filter(|m| !m.is_empty()) {
            args.insert("members".into(), json!(m));
        }
        Ok(Self::parse_activity(&self.call("list_project_activity", Value::Object(args)).await?, from, to))
    }

    pub fn parse_activity(r: &Value, from: NaiveDate, to: NaiveDate) -> ActivityFeed {
        let summary = obj(r, "summary");
        let num = |name: &str| prop(summary, name).and_then(as_i32).unwrap_or(0);
        let top = items(summary, &["topMembers"])
            .iter()
            .filter_map(|m| Some((string(m, "name")?, prop(m, "count").and_then(as_i32).unwrap_or(0))))
            .collect();
        let items_ = items(r, &["items"])
            .iter()
            .filter_map(|i| {
                Some(ActivityItem {
                    text: string(i, "text")?,
                    at: str_(i, "at").unwrap_or("").replace('\u{2009}', " "),
                    project: string(i, "project").unwrap_or_default(),
                })
            })
            .collect();
        ActivityFeed {
            from: day(r, "from").unwrap_or(from),
            to: day(r, "to").unwrap_or(to),
            created: num("created"),
            started: num("started"),
            completed: num("completed"),
            overdue: num("overdue"),
            top_members: top,
            items: items_,
        }
    }

    /// 我在一段期間的工作負荷；mode：planned（預計工時照容量分配）或 actual（實際計時）。
    pub async fn my_workload(&self, from: NaiveDate, to: NaiveDate, mode: &str) -> Result<Workload, McpError> {
        let r = self.call("get_workload", json!({ "from": iso(from), "to": iso(to), "mode": mode })).await?;
        Ok(Self::parse_workload(&r, from, to, mode))
    }

    pub fn parse_workload(r: &Value, from: NaiveDate, to: NaiveDate, mode: &str) -> Workload {
        let actual = str_(r, "mode").unwrap_or(mode) == "actual";
        let field = if actual { "actual" } else { "scheduled" };
        let person = items(r, &["people"]).first().unwrap_or(&NULL);
        let has_person = person.is_object();
        let days = if has_person {
            items(person, &["days"])
                .iter()
                .filter_map(|d| {
                    Some(WorkloadDay {
                        date: day(d, "date")?,
                        minutes: minutes_of(d, &[field, "minutes"]),
                        capacity_minutes: minutes_of(d, &["capacity"]),
                        task_count: prop(d, "taskCount")
                            .and_then(as_i32)
                            .unwrap_or_else(|| items(d, &["tasks"]).len() as i32),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let tasks = if has_person {
            items(person, &["tasks"])
                .iter()
                .filter_map(|t| {
                    Some(WorkloadTask {
                        key: string(t, "task")?,
                        title: string(t, "title").unwrap_or_default(),
                        minutes: minutes_of(t, &[field, "minutes"]),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let incomplete = match prop(r, "incomplete") {
            Some(inc @ Value::Object(_)) => items(inc, &["tasks"])
                .iter()
                .filter_map(|t| {
                    Some((
                        string(t, "task")?,
                        string(t, "title").unwrap_or_default(),
                        string(t, "reason").unwrap_or_default(),
                    ))
                })
                .collect(),
            _ => Vec::new(),
        };
        let total = if has_person { minutes_of(person, &[field]) } else { minutes_of(r, &[field]) };
        let capacity = if has_person { minutes_of(person, &["capacity"]) } else { 0 };
        Workload {
            mode: if actual { "actual" } else { "planned" }.to_owned(),
            from: day(r, "from").unwrap_or(from),
            to: day(r, "to").unwrap_or(to),
            total_minutes: total,
            capacity_minutes: capacity,
            days,
            tasks,
            incomplete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 ProjectsApiTests（樣本取自實際回應）

    #[test]
    fn parse_pins_讀取今天累積與能不能計時() {
        let pins = HayakuApi::parse_pins(&j(r#"
            {"today":"2026-10-30","pins":[
              {"task":"F2P-126","title":"更新版本覆蓋設定問題","project":"F2P","status":"處理中","statusCategory":"IN_PROGRESS",
               "today":{"minutes":5,"display":"0.08 小時"},"running":false,"canTime":true,"isCategory":false},
              {"task":"F2P-25","title":"第二階段","project":"F2P","status":"進行中","today":{"minutes":0},"running":true,"canTime":false}
            ]}"#));
        assert_eq!(pins.len(), 2);
        assert_eq!(
            (pins[0].key.as_str(), pins[0].today_minutes, pins[0].running, pins[0].can_time),
            ("F2P-126", 5, false, true)
        );
        assert!(pins[1].running);
        assert!(!pins[1].can_time);
    }

    #[test]
    fn parse_change_requests_讀取任務申請人與改了什麼() {
        let list = HayakuApi::parse_change_requests(&j(r#"
            {"items":[
              {"project":"F2P","id":"a1","type":"UPDATE","status":"PENDING",
               "task":{"key":"F2P-97","number":97,"title":"報表頁面新增歸0按鈕"},
               "requester":{"username":"harry.gu","displayName":"古信汯"},
               "changes":{"statusId":"dcc4d548","plannedEnd":"2026-11-05","estimatedMinutes":90,"note":null},
               "reason":"客戶要求延後","loggedSeconds":12876,"descendantCount":2,"createdAt":"2026-10-27T08:53:13.107Z"},
              {"project":"TEST","id":"b2","type":"CREATE","status":"PENDING","task":{"key":"TEST-11","title":"42"},
               "requester":{"username":"bruce.cheng","displayName":"鄭為元"},"changes":null,"createdAt":"2026-10-27T07:49:26.094Z"}
            ]}"#));
        assert_eq!(list.len(), 2);
        let c = &list[0];
        assert_eq!(
            (
                c.id.as_str(),
                c.r#type.as_str(),
                c.task_key.as_deref(),
                c.requester.as_ref().map(|r| r.username.as_str())
            ),
            ("a1", "UPDATE", Some("F2P-97"), Some("harry.gu"))
        );
        assert_eq!(c.changes, vec!["狀態：有變更", "預計結束：2026-11-05", "預計工時：1.5 小時", "備註：清空"]);
        assert_eq!((c.logged_minutes, c.descendant_count, c.reason.as_deref()), (214, 2, Some("客戶要求延後")));
        assert!(list[1].changes.is_empty());
    }

    #[test]
    fn parse_activity_讀取摘要與動態() {
        let feed = HayakuApi::parse_activity(
            &j(r#"
            {"from":"2026-10-26","to":"2026-10-30",
             "summary":{"created":294,"started":63,"completed":54,"overdue":45,"topMembers":[{"name":"古信汯","count":109}]},
             "items":[{"at":"10/30 13:41","project":"EG","text":"邱子恩 開始做 EG-10 新增一個轉正類別"}]}"#),
            NaiveDate::default(),
            NaiveDate::default(),
        );
        assert_eq!((feed.created, feed.started, feed.completed, feed.overdue), (294, 63, 54, 45));
        assert_eq!(Some(feed.from), NaiveDate::from_ymd_opt(2026, 10, 26));
        assert_eq!(feed.top_members, vec![("古信汯".to_owned(), 109)]);
        assert_eq!(feed.items[0].at, "10/30 13:41");
        assert_eq!(feed.items[0].project, "EG");
    }

    #[test]
    fn parse_workload_實際模式讀每天的時數與容量() {
        let load = HayakuApi::parse_workload(
            &j(r#"
            {"from":"2026-10-26","to":"2026-11-01","mode":"actual","actual":{"minutes":1724},
             "people":[{"username":"harry.gu","capacity":{"minutes":1920},"actual":{"minutes":1724},
               "days":[{"date":"2026-10-27","actual":{"minutes":485},"capacity":{"minutes":480}}],
               "tasks":[{"task":"F2P-31","title":"Server端開發","actual":{"minutes":434}}]}]}"#),
            NaiveDate::default(),
            NaiveDate::default(),
            "actual",
        );
        assert_eq!((load.mode.as_str(), load.total_minutes, load.capacity_minutes), ("actual", 1724, 1920));
        assert_eq!(
            load.days[0],
            WorkloadDay {
                date: NaiveDate::from_ymd_opt(2026, 10, 27).expect("date"),
                minutes: 485,
                capacity_minutes: 480,
                task_count: 0
            }
        );
        assert_eq!(load.tasks[0], WorkloadTask { key: "F2P-31".into(), title: "Server端開發".into(), minutes: 434 });
    }

    #[test]
    fn parse_workload_預計模式列出算不進來的任務() {
        let load = HayakuApi::parse_workload(
            &j(r#"
            {"mode":"planned","scheduled":{"minutes":0},"people":[{"scheduled":{"minutes":0},"capacity":{"minutes":1920},"days":[],"tasks":[]}],
             "incomplete":{"count":1,"tasks":[{"task":"F2P-19","title":"自訂槽測試","reason":"尚未填寫預計工時"}]}}"#),
            NaiveDate::from_ymd_opt(2026, 10, 26).expect("date"),
            NaiveDate::from_ymd_opt(2026, 11, 1).expect("date"),
            "planned",
        );
        assert_eq!(load.mode, "planned");
        assert_eq!(load.capacity_minutes, 1920);
        assert_eq!(
            load.incomplete,
            vec![("F2P-19".to_owned(), "自訂槽測試".to_owned(), "尚未填寫預計工時".to_owned())]
        );
    }

    // 對應 TestingTests.ParsePins_會議也讀得到_已取消的不能加入
    #[test]
    fn parse_pins_會議也讀得到_已取消的不能加入() {
        let pins = HayakuApi::parse_pins(&j(r#"
            {"today":"2026-11-02","pins":[
              {"task":"F2P-126","title":"任務","project":"F2P","status":"處理中","today":{"minutes":5},"running":false,"canTime":true},
              {"meeting":"REVIVAL-9","kind":"MEETING","title":"週會","project":"REVIVAL","startAt":"2026-11-02T06:00:00Z","endAt":"2026-11-02T07:00:00Z"},
              {"task":"REVIVAL-10","kind":"MEETING","title":"取消的會","startAt":"2026-11-02T08:00:00Z","endAt":"2026-11-02T09:00:00Z","status":"已取消"}
            ],"timezone":"Asia/Taipei"}"#));
        assert_eq!(pins.len(), 3);
        assert!(!pins[0].is_meeting);
        assert!(pins[1].is_meeting);
        assert_eq!(pins[1].key, "REVIVAL-9");
        let start = parse_date_time_offset("2026-11-02T06:00:00Z").expect("start");
        let m = chrono::TimeDelta::minutes;
        assert!(!pins[1].can_join(start + m(-11)));
        assert!(pins[1].can_join(start + m(-10)));
        assert!(pins[1].can_join(start + m(30)));
        assert!(!pins[1].can_join(start + m(60))); // 已結束
        assert!(pins[2].cancelled);
        assert!(!pins[2].can_join(parse_date_time_offset("2026-11-02T08:30:00Z").expect("t")));
        assert!(HayakuApi::parse_pins(&j(r#"{"today":"2026-11-02","pins":[],"timezone":"Asia/Taipei"}"#)).is_empty());
    }
}
