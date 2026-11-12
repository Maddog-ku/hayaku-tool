//! 任務、提醒、請假、團隊、工作日誌、聊天、會議。對應 C# `Mcp/HayakuApi.Work.cs`。

use chrono::NaiveDate;
use serde_json::{Map, Value, json};

use super::HayakuApi;
use crate::json::*;
use crate::key_finder::KeyFinder;
use crate::models::*;
use crate::transport::McpError;

/// 上層、路徑、子任務裡的編號不是新任務的，找的時候跳過（C# 不分大小寫）。
const NOT_THE_NEW_TASK: [&str; 7] = ["parent", "path", "ancestors", "children", "project", "dependsOn", "blockedBy"];

fn eq_ic(a: &str, b: &str) -> bool {
    a.to_uppercase() == b.to_uppercase()
}

impl HayakuApi {
    // ───────────── 任務 ─────────────

    /// 指派給我、還沒完成的全部任務（不給分頁，順序跟網頁一樣）；一頁放不下時自動翻頁，最多 max_pages 頁（C# 預設 10）。
    /// 四個分頁用 MyTaskTabs 從這份算，不用一個分頁打一次。
    pub async fn my_open_tasks(&self, max_pages: i32) -> Result<Vec<TaskSummary>, McpError> {
        let mut list: Vec<TaskSummary> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..max_pages {
            let args = match &cursor {
                None => json!({}),
                Some(c) => json!({ "cursor": c }),
            };
            let r = self.call("my_tasks", args).await?;
            list.extend(Self::parse_task_summaries(&r));
            cursor = string(&r, "cursor").or_else(|| string(&r, "nextCursor"));
            if cursor.as_deref().is_none_or(str::is_empty) {
                break;
            }
        }
        let mut seen = std::collections::HashSet::new();
        list.retain(|t| seen.insert(t.key.to_uppercase()));
        Ok(list)
    }

    /// 指派給我、某個狀態類別的任務（例 DONE）；最多讀 max_pages 頁。
    pub async fn my_tasks_by_category(
        &self,
        status_category: &str,
        max_pages: i32,
    ) -> Result<Vec<TaskSummary>, McpError> {
        let mut list = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..max_pages {
            let mut args = Map::new();
            args.insert("statusCategory".into(), json!(status_category));
            if let Some(c) = &cursor {
                args.insert("cursor".into(), json!(c));
            }
            let r = self.call("my_tasks", Value::Object(args)).await?;
            list.extend(Self::parse_task_summaries(&r));
            cursor = string(&r, "cursor").or_else(|| string(&r, "nextCursor"));
            if cursor.as_deref().is_none_or(str::is_empty) {
                break;
            }
        }
        Ok(list)
    }

    pub fn parse_task_summaries(r: &Value) -> Vec<TaskSummary> {
        let mut list = Vec::new();
        for t in items(r, &["tasks"]) {
            let Some(key) = string(t, "task") else { continue };
            let path_items = items(t, &["path"]);
            let ancestors: Vec<(String, String)> = path_items
                .iter()
                .filter_map(|x| Some((string(x, "task")?, string(x, "title").unwrap_or_default())))
                .collect();
            let path = path_items.iter().filter_map(|x| str_(x, "title")).collect::<Vec<_>>().join(" › ");
            list.push(TaskSummary {
                key,
                title: string(t, "title").unwrap_or_default(),
                project: string(t, "project").unwrap_or_default(),
                status: string(t, "status").unwrap_or_default(),
                status_category: string(t, "statusCategory").unwrap_or_default(),
                priority: string(t, "priority").unwrap_or_default(),
                planned_start: day(t, "plannedStart"),
                planned_end: day(t, "plannedEnd"),
                timing: bool_(t, "timing"),
                path,
                ancestors: Some(ancestors),
                child_count: prop(t, "childCount").and_then(as_i32),
                test: Self::read_test_info(t),
            });
        }
        list
    }

    /// 專案的全部任務（含各層子任務），依 Hayaku 的順序；一頁 200 筆，自動翻頁。view：套用專案的視圖篩選。
    pub async fn list_project_tasks(&self, project: &str, view: Option<&str>) -> Result<Vec<ProjectTask>, McpError> {
        let mut list = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..25 {
            let mut args = Map::new();
            args.insert("project".into(), json!(project));
            args.insert("limit".into(), json!(200));
            if let Some(v) = view {
                args.insert("view".into(), json!(v));
            }
            if let Some(c) = &cursor {
                args.insert("cursor".into(), json!(c));
            }
            let r = self.call("list_tasks", Value::Object(args)).await?;
            list.extend(Self::parse_project_tasks(&r));
            cursor = string(&r, "cursor");
            if cursor.as_deref().is_none_or(str::is_empty) {
                break;
            }
        }
        Ok(list)
    }

    pub fn parse_project_tasks(r: &Value) -> Vec<ProjectTask> {
        items(r, &["tasks"])
            .iter()
            .filter_map(|t| {
                let key = string(t, "task")?;
                Some(ProjectTask {
                    key,
                    title: string(t, "title").unwrap_or_default(),
                    status: string(t, "status").unwrap_or_default(),
                    status_category: string(t, "statusCategory").unwrap_or_default(),
                    parent: items(t, &["path"]).iter().filter_map(|p| string(p, "task")).next_back(),
                    planned_start: day(t, "plannedStart"),
                    planned_end: day(t, "plannedEnd"),
                    is_category: bool_(t, "isCategory"),
                    assignees: match prop(t, "assignees") {
                        Some(Value::Array(a)) => Some(
                            a.iter()
                                .filter_map(|x| match x {
                                    Value::String(s) => Some(s.clone()),
                                    other => string(other, "username"),
                                })
                                .collect(),
                        ),
                        _ => None,
                    },
                    priority: string(t, "priority"),
                    test: Self::read_test_info(t),
                })
            })
            .collect()
    }

    pub async fn get_task(&self, task: &str) -> Result<TaskDetail, McpError> {
        let r = self.call("get_task", json!({ "task": task })).await?;
        Ok(Self::parse_task_detail(&r))
    }

    pub fn parse_task_detail(r: &Value) -> TaskDetail {
        let assignees = items(r, &["assignees"]).iter().filter_map(person).collect();
        let path = items(r, &["path"])
            .iter()
            .map(|x| (string(x, "task").unwrap_or_default(), string(x, "title").unwrap_or_default()))
            .collect();
        let children = items(r, &["children"])
            .iter()
            .map(|x| {
                (
                    string(x, "task").unwrap_or_default(),
                    string(x, "title").unwrap_or_default(),
                    string(x, "status").unwrap_or_default(),
                )
            })
            .collect();
        let estimated = match prop(r, "estimated") {
            Some(v @ Value::Number(_)) => as_f64(v),
            Some(v @ Value::Object(_)) => prop(v, "minutes").and_then(as_f64).map(|m| round_even(m / 60.0, 2)),
            _ => None,
        };
        let lg = obj(r, "logged");
        let logged = if lg.is_object() { string(lg, "display").unwrap_or_default() } else { String::new() };
        let logged_minutes = if lg.is_object() { prop(lg, "minutes").and_then(as_f64) } else { None };
        TaskDetail {
            key: string(r, "task").unwrap_or_default(),
            title: string(r, "title").unwrap_or_default(),
            status: string(r, "status").unwrap_or_default(),
            status_category: string(r, "statusCategory").unwrap_or_default(),
            priority: string(r, "priority").unwrap_or_default(),
            assignees,
            planned_start: day(r, "plannedStart"),
            planned_end: day(r, "plannedEnd"),
            actual_start: day(r, "actualStart"),
            actual_end: day(r, "actualEnd"),
            estimated_hours: estimated,
            logged_text: logged,
            note: string(r, "note"),
            approval_state: string(r, "approvalState").unwrap_or_default(),
            path,
            children,
            pending_change_requests: items(r, &["pendingChangeRequests"]).len() as i32,
            logged_minutes,
            is_category: bool_(r, "isCategory"),
            content_block_count: items(r, &["contentBlocks"]).len() as i32,
            test: Self::read_test_info(r),
            created_by: first_name(r, "createdBy"),
        }
    }

    /// 部分更新任務；fields 的值為 null 代表清空。回傳伺服器的說明文字（例如送出變更申請）。
    pub async fn update_task(&self, task: &str, fields: Map<String, Value>) -> Result<Option<String>, McpError> {
        let mut args = fields;
        args.insert("task".into(), json!(task));
        let r = self.call("update_task", Value::Object(args)).await?;
        Ok(message(&r))
    }

    /// 在任務內容（第一份文件）最後補一段 Markdown，原本的圖片、附件、表格都不動（append_to_task_note）。
    /// 回傳伺服器的說明文字。
    pub async fn append_task_note(&self, task: &str, markdown: &str) -> Result<Option<String>, McpError> {
        Ok(message(&self.call("append_to_task_note", json!({ "task": task, "markdown": markdown })).await?))
    }

    pub async fn create_task(&self, fields: Map<String, Value>) -> Result<String, McpError> {
        let parent = fields.get("parent").and_then(Value::as_str).map(str::to_owned);
        let r = self.call("create_task", Value::Object(fields)).await?;
        let key = Self::created_key(&r, parent.as_deref());
        if key.is_none() {
            // C# 的 Undefined（沒有內容）在這裡是 null
            let raw = if r.is_null() { String::new() } else { raw_text(&r) };
            let shown = if utf16_len(&raw) > 500 { format!("{}…", truncate_utf16(&raw, 500)) } else { raw };
            log::info!("create_task 的回應裡找不到任務編號：{shown}");
        }
        Ok(key.unwrap_or_default())
    }

    /// 一次建立整棵任務樹（create_tasks，全部成功或全部不建，最多 100 筆）；parent 有值時全部放在它底下。
    /// assign_me：每個任務都指派給自己。回傳伺服器的說明（例如執行者建的要等核准），沒有就是 None。
    pub async fn create_tasks(
        &self,
        project: &str,
        parent: Option<&str>,
        nodes: &[OutlineNode],
        assign_me: bool,
    ) -> Result<Option<String>, McpError> {
        fn item(n: &OutlineNode, assign_me: bool) -> Value {
            let mut obj = Map::new();
            obj.insert("title".into(), json!(n.title));
            if assign_me {
                obj.insert("assignees".into(), json!(["me"]));
            }
            if !n.children.is_empty() {
                obj.insert("children".into(), Value::Array(n.children.iter().map(|c| item(c, assign_me)).collect()));
            }
            Value::Object(obj)
        }
        let mut args = Map::new();
        args.insert("project".into(), json!(project));
        args.insert("tasks".into(), Value::Array(nodes.iter().map(|n| item(n, assign_me)).collect()));
        if let Some(p) = parent {
            args.insert("parent".into(), json!(p));
        }
        Ok(message(&self.call("create_tasks", Value::Object(args)).await?))
    }

    /// create_task 的回應 → 新任務的編號。直接建立是 { task: "F2P-103" }；執行者建立（要等核准）時可能包在物件裡，
    /// 所以由淺到深找 task／key／number／code 欄位（跳過上層、路徑）；都沒有才從訊息文字裡找（排除上層的編號）。
    pub fn created_key(r: &Value, parent: Option<&str>) -> Option<String> {
        let not_parent = |k: &String| parent.is_none_or(|p| !eq_ic(k, p));
        if let Some(s) = r.as_str() {
            return KeyFinder::parse(Some(s)).filter(not_parent);
        }
        if !r.is_object() {
            return None;
        }
        let mut level: Vec<&Value> = vec![r];
        let mut texts: Vec<&str> = Vec::new();
        let mut depth = 0;
        while depth < 4 && !level.is_empty() {
            let mut next = Vec::new();
            for o in &level {
                let Some(map) = o.as_object() else { continue };
                for (name, v) in map {
                    if NOT_THE_NEW_TASK.iter().any(|n| eq_ic(n, name)) {
                        continue;
                    }
                    match v {
                        Value::String(s) => {
                            if matches!(name.as_str(), "task" | "key" | "number" | "code")
                                && let Some(key) = KeyFinder::parse(Some(s))
                            {
                                return Some(key);
                            }
                            texts.push(s);
                        }
                        Value::Object(_) => next.push(v),
                        Value::Array(a) => next.extend(a.iter().filter(|x| x.is_object())),
                        _ => {}
                    }
                }
            }
            level = next;
            depth += 1;
        }
        texts.into_iter().flat_map(|t| KeyFinder::all(Some(t))).find(not_parent)
    }

    pub async fn search_tasks(&self, query: &str) -> Result<Vec<SearchHit>, McpError> {
        let r = self.call("search_tasks", json!({ "query": query })).await?;
        Ok(items(&r, &["tasks"])
            .iter()
            .filter_map(|t| {
                Some(SearchHit {
                    key: string(t, "task")?,
                    title: string(t, "title").unwrap_or_default(),
                    project: string(t, "project").unwrap_or_default(),
                })
            })
            .collect())
    }

    /// 看得到的專案（含圖示與我的角色）；include_archived 連封存的也列（C# 預設 false）。
    pub async fn list_projects(&self, include_archived: bool) -> Result<Vec<ProjectInfo>, McpError> {
        let args = if include_archived { json!({ "includeArchived": true }) } else { json!({}) };
        Ok(Self::parse_projects(&self.call("list_projects", args).await?))
    }

    pub fn parse_projects(r: &Value) -> Vec<ProjectInfo> {
        items(r, &["projects"])
            .iter()
            .filter_map(|p| {
                Some(ProjectInfo {
                    key: string(p, "key")?,
                    name: string(p, "name").unwrap_or_default(),
                    role: string(p, "role").unwrap_or_default(),
                    icon: string(p, "icon"),
                    icon_color: string(p, "iconColor"),
                    is_archived: bool_(p, "isArchived"),
                })
            })
            .collect()
    }

    pub async fn get_project(&self, project: &str) -> Result<ProjectDetail, McpError> {
        let r = self.call("get_project", json!({ "project": project })).await?;
        let members = items(&r, &["members"]).iter().filter_map(person).collect();
        let statuses = items(&r, &["statuses"])
            .iter()
            .filter_map(|s| {
                Some(StatusOption {
                    name: string(s, "name")?,
                    category: string(s, "category").unwrap_or_default(),
                    is_default: bool_(s, "isDefault"),
                })
            })
            .collect();
        Ok(ProjectDetail {
            key: string(&r, "key").unwrap_or_else(|| project.to_owned()),
            name: string(&r, "name").unwrap_or_else(|| project.to_owned()),
            members,
            statuses,
            test_sub_statuses: Some(Self::parse_sub_statuses(&r, &["testSubStatuses"])),
        })
    }

    // ───────────── 提醒 ─────────────

    pub async fn list_reminders_full(&self, tab: &str, task: Option<&str>) -> Result<Vec<ReminderFull>, McpError> {
        let mut args = Map::new();
        args.insert("tab".into(), json!(tab));
        if let Some(t) = task {
            args.insert("task".into(), json!(t));
        }
        let r = self.call("list_reminders", Value::Object(args)).await?;
        let mut list = Vec::new();
        for t in items(&r, &["reminders", "items"]) {
            let Some(id) = string(t, "id") else { continue };
            let mut repeat = None;
            let mut paused = false;
            if let Some(rep @ Value::Object(_)) = prop(t, "repeat") {
                repeat = Some(Self::repeat_text(rep));
                paused = str_(rep, "pausedAt").is_some();
            }
            list.push(ReminderFull {
                id,
                text: first_string(t, &["message", "label", "text", "title"]).unwrap_or_else(|| "提醒".to_owned()),
                at: first_date(t, &["nextAt", "remindAt", "fireAt", "lastFiredAt", "at"]),
                task: first_key(t, &["task", "taskCode"]),
                repeat_text: repeat,
                paused,
                editable: !is_false(t, "editable"),
            });
        }
        Ok(list)
    }

    /// 重複規則 → 「每週四 09:00」這類文字（weekdays：0=週一…6=週日）。
    pub fn repeat_text(rep: &Value) -> String {
        let time = str_(rep, "timeOfDay").unwrap_or("");
        let rule = match prop(rep, "rule") {
            Some(rule @ Value::Object(_)) => rule,
            _ => return format!("重複 {time}").trim().to_owned(),
        };
        let int = |name: &str| prop(rule, name).and_then(as_i32).unwrap_or(0);
        let interval = int("interval").max(1);
        let names: Vec<char> = "一二三四五六日".chars().collect();
        let text = match str_(rule, "freq") {
            Some("DAILY") if bool_(rule, "workdaysOnly") => "每個工作日".to_owned(),
            Some("DAILY") => {
                if interval == 1 {
                    "每天".to_owned()
                } else {
                    format!("每 {interval} 天")
                }
            }
            Some("WEEKLY") => {
                let head = if interval == 1 { "每週".to_owned() } else { format!("每 {interval} 週") };
                let days: Vec<String> = items(rule, &["weekdays"])
                    .iter()
                    .filter_map(as_i32)
                    .filter(|n| (0..=6).contains(n))
                    .map(|n| names[n as usize].to_string())
                    .collect();
                head + &days.join("、")
            }
            Some("MONTHLY") if prop(rule, "lastDay").is_some() => "每月最後一天".to_owned(),
            Some("MONTHLY") if int("monthDay") > 0 => format!("每月 {} 日", int("monthDay")),
            Some("MONTHLY") => "每月".to_owned(),
            _ => "重複".to_owned(),
        };
        format!("{text} {time}").trim().to_owned()
    }

    pub async fn create_reminder(&self, args: Map<String, Value>) -> Result<(), McpError> {
        self.call("create_reminder", Value::Object(args)).await.map(drop)
    }

    pub async fn update_reminder(&self, args: Map<String, Value>) -> Result<(), McpError> {
        self.call("update_reminder", Value::Object(args)).await.map(drop)
    }

    pub async fn delete_reminder(&self, id: &str) -> Result<(), McpError> {
        self.call("delete_reminder", json!({ "id": id })).await.map(drop)
    }

    // ───────────── 通知 ─────────────

    pub async fn archive_notifications(&self, ids: &[String]) -> Result<(), McpError> {
        self.call("archive_notifications", json!({ "ids": ids })).await.map(drop)
    }

    pub async fn snooze_notifications(&self, ids: &[String], preset: &str) -> Result<(), McpError> {
        self.call("snooze_notifications", json!({ "ids": ids, "preset": preset })).await.map(drop)
    }

    // ───────────── 請假與團隊 ─────────────

    /// 請假與出差；user：帳號、顯示名稱或 "me"，None 列出看得到的所有人。kind：LEAVE 只看請假、TRIP 只看出差，None 都列。
    pub async fn list_leaves(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        user: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<LeaveItem>, McpError> {
        let mut args = Map::new();
        args.insert("from".into(), json!(iso(from)));
        args.insert("to".into(), json!(iso(to)));
        if let Some(u) = user {
            args.insert("user".into(), json!(u));
        }
        if let Some(k) = kind {
            args.insert("kind".into(), json!(k));
        }
        Ok(Self::parse_leaves(&self.call("list_leaves", Value::Object(args)).await?))
    }

    pub fn parse_leaves(r: &Value) -> Vec<LeaveItem> {
        let mut list = Vec::new();
        for l in items(r, &["leaves"]) {
            let (Some(id), Some(start), Some(end)) = (string(l, "id"), date(l, "startAt"), date(l, "endAt")) else {
                continue;
            };
            let user = prop(l, "user").and_then(person);
            let lo = obj(l, "lost");
            let lost = if lo.is_object() { string(lo, "display").unwrap_or_default() } else { String::new() };
            list.push(LeaveItem {
                id,
                user: user.unwrap_or_else(|| PersonRef::new("", "")),
                period: string(l, "period").unwrap_or_default(),
                start_at: start,
                end_at: end,
                lost_text: lost,
                kind: string(l, "kind"),
            });
        }
        list.sort_by_key(|a| a.start_at);
        list
    }

    /// 登記請假；回傳新請假的 id（回應裡沒有就是 None）。
    pub async fn add_leave(&self, args: Map<String, Value>) -> Result<Option<String>, McpError> {
        Ok(Self::created_id(&self.call("add_leave", Value::Object(args)).await?))
    }

    /// 新增類工具回傳的 id：{ id } 或 { leave: { id } }。
    pub fn created_id(r: &Value) -> Option<String> {
        if !r.is_object() {
            return None;
        }
        if let Some(id) = str_(r, "id").filter(|s| !s.is_empty()) {
            return Some(id.to_owned());
        }
        match prop(r, "leave") {
            Some(leave @ Value::Object(_)) => string(leave, "id"),
            _ => None,
        }
    }

    pub async fn cancel_leave(&self, id: &str) -> Result<(), McpError> {
        self.call("cancel_leave", json!({ "id": id })).await.map(drop)
    }

    pub async fn team_schedule(&self) -> Result<Vec<TeamMember>, McpError> {
        let r = self.call("team_schedule", json!({})).await?;
        let mut list = Vec::new();
        for p in items(&r, &["people"]) {
            let user = PersonRef::new(
                string(p, "username").unwrap_or_default(),
                string(p, "displayName").or_else(|| string(p, "username")).unwrap_or_default(),
            );
            let leaves = items(p, &["leaves"]).iter().filter_map(|x| x.as_str().map(str::to_owned)).collect();
            let timing = items(p, &["timing"])
                .iter()
                .map(|t| TeamTiming {
                    task: string(t, "task").unwrap_or_default(),
                    title: string(t, "title").unwrap_or_default(),
                    project: string(t, "project").unwrap_or_default(),
                    since: date(t, "since"),
                    can_open: bool_(t, "canOpen"),
                })
                .collect();
            list.push(TeamMember {
                user,
                job_title: string(p, "jobTitle"),
                online: bool_(p, "online"),
                on_leave: bool_(p, "onLeave"),
                leaves,
                timing,
            });
        }
        Ok(list)
    }

    pub async fn get_schedule_range(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<DaySchedule>, McpError> {
        let r = self.call("get_my_schedule", json!({ "from": iso(from), "to": iso(to) })).await?;
        Ok(items(&r, &["days"]).iter().map(|d| Self::parse_day(d, from)).collect())
    }

    // ───────────── 工作日誌 ─────────────

    pub async fn get_work_log(&self, date_: NaiveDate) -> Result<WorkLog, McpError> {
        let r = self.call("get_work_log", json!({ "date": iso(date_) })).await?;
        let entries = items(&r, &["entries"])
            .iter()
            .map(|e| WorkLogEntry {
                task: string(e, "task").unwrap_or_default(),
                minutes: prop(e, "minutes").and_then(as_i32).unwrap_or(0),
                ranges: items(e, &["ranges"])
                    .iter()
                    .filter_map(|x| Some((date(x, "start")?, date(x, "end")?)))
                    .collect(),
            })
            .collect();
        let total = prop(&r, "totalMinutes").and_then(as_i32).unwrap_or(0);
        Ok(WorkLog { date: date_, total_minutes: total, entries, markdown: string(&r, "markdown").unwrap_or_default() })
    }

    // ───────────── 聊天 ─────────────

    pub async fn list_channels(&self, project: &str) -> Result<Vec<String>, McpError> {
        let r = self.call("list_channels", json!({ "project": project })).await?;
        Ok(items(&r, &["channels"])
            .iter()
            .filter(|c| !bool_(c, "archived"))
            .filter_map(|c| string(c, "name"))
            .collect())
    }

    /// 給 task，或 project+channel；新到舊。
    pub async fn list_chat(
        &self,
        task: Option<&str>,
        project: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Vec<ChatMessage>, McpError> {
        let mut args = Map::new();
        args.insert("limit".into(), json!(50));
        match task {
            Some(t) => {
                args.insert("task".into(), json!(t));
            }
            None => {
                args.insert("project".into(), json!(project));
                args.insert("channel".into(), json!(channel));
            }
        }
        Ok(Self::parse_chat(&self.call("list_chat", Value::Object(args)).await?))
    }

    /// list_chat 的訊息；投票訊息帶題目與投票 id（get_poll 看結果）。
    pub fn parse_chat(r: &Value) -> Vec<ChatMessage> {
        let mut list = Vec::new();
        for m in items(r, &["items", "messages"]) {
            let Some(id) = string(m, "id") else { continue };
            let author = prop(m, "author").and_then(person);
            let poll = obj(m, "poll");
            list.push(ChatMessage {
                id,
                author: author.unwrap_or_else(|| PersonRef::new("", "系統")),
                body: string(m, "body").unwrap_or_default(),
                created_at: date(m, "createdAt").unwrap_or_else(now),
                deleted: matches!(prop(m, "deletedAt"), Some(Value::String(_))),
                poll_question: if poll.is_object() { string(poll, "question") } else { None },
                poll_id: if poll.is_object() { string(poll, "id") } else { None },
            });
        }
        list
    }

    pub async fn send_chat(
        &self,
        task: Option<&str>,
        project: Option<&str>,
        channel: Option<&str>,
        body: &str,
    ) -> Result<(), McpError> {
        let mut args = Map::new();
        args.insert("body".into(), json!(body));
        match task {
            Some(t) => {
                args.insert("task".into(), json!(t));
            }
            None => {
                args.insert("project".into(), json!(project));
                args.insert("channel".into(), json!(channel));
            }
        }
        self.call("send_chat", Value::Object(args)).await.map(drop)
    }

    // ───────────── 會議 ─────────────

    pub async fn upcoming_meetings(&self) -> Result<Vec<MeetingItem>, McpError> {
        Ok(Self::parse_meetings(&self.call("list_meetings", json!({ "limit": 30 })).await?))
    }

    pub async fn past_meetings(&self) -> Result<Vec<MeetingItem>, McpError> {
        let mut list = Self::parse_meetings(&self.call("list_meetings", json!({ "past": true, "limit": 20 })).await?);
        list.sort_by_key(|a| std::cmp::Reverse(a.start_at)); // OrderByDescending（穩定排序）
        Ok(list)
    }

    pub async fn create_meeting(&self, args: Map<String, Value>) -> Result<(), McpError> {
        self.call("create_meeting", Value::Object(args)).await.map(drop)
    }

    pub async fn leave_meeting(&self, meeting: &str) -> Result<(), McpError> {
        self.call("leave_meeting", json!({ "meeting": meeting })).await.map(drop)
    }

    pub async fn cancel_meeting(&self, meeting: &str) -> Result<(), McpError> {
        self.call("cancel_meeting", json!({ "meeting": meeting })).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 WorkTests（WorkParseTests）的 API 解析部分
    #[test]
    fn parse_task_summaries_讀取路徑與日期() {
        let list = HayakuApi::parse_task_summaries(&j(r#"
            {"tab":"逾期","tasks":[{"task":"F2P-15","title":"事務機APP開發","project":"F2P-臉部辨識結合印表機","status":"處理中",
              "statusCategory":"IN_PROGRESS","priority":"無","plannedStart":"2026-10-21","plannedEnd":"2026-10-23","timing":true,
              "path":[{"task":"F2P-5","title":"第一階段研究"},{"task":"F2P-14","title":"機密列印"}]}]}"#));
        assert_eq!(list.len(), 1);
        let t = &list[0];
        assert_eq!(t.key, "F2P-15");
        assert_eq!(t.path, "第一階段研究 › 機密列印");
        assert_eq!(t.planned_end, NaiveDate::from_ymd_opt(2026, 10, 23));
        assert!(t.timing);
    }

    #[test]
    fn parse_task_detail_讀取執行者工時與子任務() {
        let d = HayakuApi::parse_task_detail(&j(r#"
            {"task":"F2P-97","title":"報表頁面新增歸0按鈕","status":"客戶驗證中","statusCategory":"IN_PROGRESS","priority":"無",
             "assignees":[{"username":"harry.gu","displayName":"古信汯"}],"plannedStart":"2026-10-27","plannedEnd":"2026-10-28",
             "actualStart":"2026-10-27","actualEnd":null,"estimated":null,"logged":{"minutes":215,"display":"3.58 小時"},
             "note":null,"approvalState":"APPROVED","pendingChangeRequests":[],"path":[{"task":"F2P-5","title":"第一階段研究"}],
             "children":[{"task":"F2P-98","title":"子","status":"未開始"}]}"#));
        assert_eq!(d.assignees.len(), 1);
        assert_eq!(d.assignees[0].display_name, "古信汯");
        assert_eq!(d.logged_text, "3.58 小時");
        assert_eq!(d.logged_minutes, Some(215.0));
        assert_eq!(d.estimated_hours, None);
        assert_eq!(d.actual_end, None);
        assert_eq!(d.children.len(), 1);
        assert_eq!(d.children[0].0, "F2P-98");
        assert_eq!(d.pending_change_requests, 0);
    }

    #[test]
    fn repeat_text_轉成中文() {
        let cases = [
            (
                r#"{"rule":{"freq":"WEEKLY","interval":1,"weekdays":[3]},"timeOfDay":"09:00","pausedAt":null}"#,
                "每週四 09:00",
            ),
            (r#"{"rule":{"freq":"DAILY","interval":1,"workdaysOnly":true},"timeOfDay":"08:30"}"#, "每個工作日 08:30"),
            (r#"{"rule":{"freq":"MONTHLY","interval":1,"monthDay":5},"timeOfDay":"10:00"}"#, "每月 5 日 10:00"),
        ];
        for (json, expected) in cases {
            assert_eq!(HayakuApi::repeat_text(&j(json)), expected);
        }
    }

    // 對應 ProjectsApiTests.我的任務讀子任務數_分類與專案圖示
    #[test]
    fn 我的任務讀子任務數_分類與專案圖示() {
        let tasks = HayakuApi::parse_task_summaries(&j(r#"
            {"tasks":[{"task":"F2P-26","title":"點數系統","project":"F2P","status":"處理中","statusCategory":"IN_PROGRESS","childCount":2},
                      {"task":"F2P-28","title":"面板顯示餘額","project":"F2P","status":"處理中","statusCategory":"IN_PROGRESS","childCount":0},
                      {"task":"F2P-30","title":"舊版回應"}]}"#));
        assert_eq!(tasks.iter().map(TaskSummary::is_leaf).collect::<Vec<_>>(), vec![false, true, true]);

        let project_tasks = HayakuApi::parse_project_tasks(&j(
            r#"{"tasks":[{"task":"F2P-2","title":"文管","isCategory":true,"path":[]}]}"#,
        ));
        assert!(project_tasks[0].is_category);

        let projects = HayakuApi::parse_projects(&j(r#"
            {"projects":[{"key":"EG","name":"EG-長榮","role":"ASSIGNEE","isArchived":false,"icon":"rocket","iconColor":"moss"},
                         {"key":"HYK","name":"Hayaku","role":"ADMIN","isArchived":true}]}"#));
        assert_eq!(
            (projects[0].icon.as_deref(), projects[0].icon_color.as_deref(), projects[0].can_review()),
            (Some("rocket"), Some("moss"), false)
        );
        assert!(projects[1].can_review());
        assert!(projects[1].is_archived);
    }

    // 對應 ProjectsApiTests.TaskDetail讀分類
    #[test]
    fn task_detail讀分類() {
        let d =
            HayakuApi::parse_task_detail(&j(r#"{"task":"F2P-25","title":"第二階段","isCategory":true,"children":[]}"#));
        assert!(d.is_category);
    }

    // 對應 TaskTreeTests.ParseProjectTasks_用路徑最後一層當上層
    #[test]
    fn parse_project_tasks_用路徑最後一層當上層() {
        let tasks = HayakuApi::parse_project_tasks(&j(r#"
            {"tasks":[
              {"task":"F2P-2","title":"文管系統開發","status":"處理中","statusCategory":"IN_PROGRESS","path":[],"plannedStart":"2026-09-01","plannedEnd":null},
              {"task":"F2P-14","title":"機密列印","status":"處理中","statusCategory":"IN_PROGRESS",
               "path":[{"task":"F2P-2","title":"文管系統開發"},{"task":"F2P-5","title":"第一階段研究"}]}
            ],"cursor":null}"#));
        assert_eq!(tasks[0].parent, None);
        assert_eq!(tasks[0].planned_start, NaiveDate::from_ymd_opt(2026, 9, 1));
        assert_eq!(tasks[1].parent.as_deref(), Some("F2P-5"));
    }

    // 對應 Hayaku14Tests.請假列表_分得出出差
    #[test]
    fn 請假列表_分得出出差() {
        let leaves = HayakuApi::parse_leaves(&j(r#"
            {"leaves":[
              {"id":"1","user":{"username":"a","displayName":"甲"},"kind":"請假","period":"11/9 整天","startAt":"2026-11-08T16:00:00Z","endAt":"2026-11-09T16:00:00Z","lost":{"display":"8 小時"}},
              {"id":"2","user":{"username":"b","displayName":"乙"},"kind":"出差","period":"11/10 整天","startAt":"2026-11-09T16:00:00Z","endAt":"2026-11-10T16:00:00Z"}]}"#));
        assert!(!leaves[0].is_trip());
        assert!(leaves[1].is_trip());
        let old = LeaveItem {
            id: "3".into(),
            user: PersonRef::new("c", "丙"),
            period: String::new(),
            start_at: min_value(),
            end_at: min_value(),
            lost_text: String::new(),
            kind: None,
        };
        assert!(!old.is_trip()); // 舊的回應沒有 kind
    }

    // 對應 NewToolsTests.ParseChat_投票訊息帶投票id
    #[test]
    fn parse_chat_投票訊息帶投票id() {
        let messages = HayakuApi::parse_chat(&j(r#"
            {"messages":[{"id":"m1","author":{"username":"return.lo","displayName":"羅瑞騰"},"body":"Q","createdAt":"2026-10-27T07:10:35.148Z",
              "deletedAt":null,"poll":{"id":"0a793a8b-b232-4ed0-9fe0-e3aa7dd3d373","place":"CHAT","question":"Q","options":[]}},
             {"id":"m2","author":{"username":"a","displayName":"A"},"body":"hi","createdAt":"2026-10-27T07:11:00Z","deletedAt":null}]}"#));
        assert_eq!(
            (messages[0].poll_question.as_deref(), messages[0].poll_id.as_deref()),
            (Some("Q"), Some("0a793a8b-b232-4ed0-9fe0-e3aa7dd3d373"))
        );
        assert_eq!(messages[1].poll_id, None);
    }

    // 對應 CreatedKeyTests（AutoCreateTests.cs）
    fn key(json: &str, parent: Option<&str>) -> Option<String> {
        HayakuApi::created_key(&j(json), parent)
    }

    #[test]
    fn 直接建立_task欄位() {
        assert_eq!(key(r#"{"task":"F2P-103","title":"x"}"#, None).as_deref(), Some("F2P-103"));
    }

    #[test]
    fn 要核准_包在物件裡_跳過路徑與上層() {
        assert_eq!(
            key(
                r#"{"message":"已送出","parent":"F2P-5","task":{"path":[{"task":"F2P-5"}],"task":"F2P-103"}}"#,
                Some("F2P-5")
            )
            .as_deref(),
            Some("F2P-103")
        );
    }

    #[test]
    fn 要核准_包在change_request裡() {
        assert_eq!(
            key(r#"{"changeRequest":{"id":"abc","type":"CREATE","task":{"key":"F2P-103"}}}"#, None).as_deref(),
            Some("F2P-103")
        );
    }

    #[test]
    fn 只有訊息文字_排除上層編號() {
        assert_eq!(
            key(r#"{"message":"已在 F2P-5 底下建立 F2P-103，等管理者核准"}"#, Some("F2P-5")).as_deref(),
            Some("F2P-103")
        );
    }

    #[test]
    fn 找不到回null() {
        assert_eq!(key(r#"{"ok":true}"#, None), None);
    }

    // 對應 LeavePlannerTests.CreatedId_支援id或leave物件
    #[test]
    fn created_id_支援id或leave物件() {
        assert_eq!(HayakuApi::created_id(&json!({ "id": "a" })).as_deref(), Some("a"));
        assert_eq!(HayakuApi::created_id(&json!({ "leave": { "id": "b" } })).as_deref(), Some("b"));
        assert_eq!(HayakuApi::created_id(&json!("已登記")), None);
        assert_eq!(HayakuApi::created_id(&Value::Null), None);
    }

    #[test]
    fn create_tasks_參數() {
        let fake = std::sync::Arc::new(FakeTransport::default());
        let api = HayakuApi::new(fake.clone());
        let nodes = vec![OutlineNode::new("A", vec![OutlineNode::new("B", vec![])])];
        block_on(api.create_tasks("F2P", Some("F2P-1"), &nodes, true)).expect("create_tasks");
        let calls = fake.calls.lock().expect("lock");
        assert_eq!(
            calls[0].1,
            json!({"project":"F2P","tasks":[{"title":"A","assignees":["me"],"children":[{"title":"B","assignees":["me"]}]}],"parent":"F2P-1"})
        );
    }
}
