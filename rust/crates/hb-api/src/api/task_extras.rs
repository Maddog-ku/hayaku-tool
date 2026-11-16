//! 任務詳細的附加資料：相依、附件、文件、異動紀錄、刪除。都是展開那一塊才讀。對應 C# `Mcp/HayakuApi.TaskExtras.cs`。

use serde_json::{Map, Value, json};

use super::HayakuApi;
use super::projects::change_label;
use crate::json::*;
use crate::key_finder::KeyFinder;
use crate::models::*;
use crate::transport::McpError;
use crate::work_math::WorkMath;

fn activity_label(name: &str) -> Option<&'static str> {
    Some(match name {
        "parent" => "上層",
        "plannedStartDate" => "預計開始",
        "plannedEndDate" => "預計結束",
        "actualStartDate" => "實際開始",
        "actualEndDate" => "實際結束",
        "fields" => "欄位",
        // 跟上 Hayaku 1.5.5（異動紀錄不再出現英文代碼）
        "type" => "類型",
        "approvalState" => "核准狀態",
        "subtask" => "子任務",
        "milestones" => "里程碑",
        "category" => "分類",
        "severity" => "嚴重度",
        "durationSeconds" => "工時",
        _ => return None,
    })
}

/// 異動的值：日期時間換成本地時間、陣列用頓號接起來、太長的截斷。
fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Bool(true) => Some("是".to_owned()),
        Value::Bool(false) => Some("否".to_owned()),
        Value::Array(a) => Some(a.iter().filter_map(value_text).collect::<Vec<_>>().join("、")),
        Value::String(s) => {
            if utf16_len(s) > 10
                && utf16_at(s, 4) == Some(u16::from(b'-'))
                && s.contains('T')
                && let Some(at) = parse_date_time_offset(s)
            {
                return Some(local_short(&at));
            }
            Some(if utf16_len(s) > 60 { format!("{}…", truncate_utf16(s, 60)) } else { s.clone() })
        }
        other => Some(raw_text(other)),
    }
}

/// {"title": ["舊", "新"], …} → 「名稱：舊 → 新、狀態：處理中」。
fn field_changes(changes: &Value, skip: Option<&str>) -> String {
    field_changes_with(changes, skip.as_slice(), "：")
}

/// 同上，可以略過多個欄位、指定名稱和值中間的分隔（Hayaku 1.5.5 起寫成「類型 一般 → 測試」，用空白）。
fn field_changes_with(changes: &Value, skip: &[&str], sep: &str) -> String {
    let Some(o) = changes.as_object() else {
        return String::new();
    };
    let mut parts = Vec::new();
    for (name, v) in o {
        let Some(pair) = v.as_array().filter(|a| a.len() == 2) else { continue };
        if skip.contains(&name.as_str()) {
            continue;
        }
        let label = change_label(name).or_else(|| activity_label(name)).unwrap_or(name);
        let text = |v: &Value| match (name.as_str(), as_f64(v)) {
            ("durationSeconds", Some(secs)) => Some(WorkMath::hours_minutes(secs / 60.0)),
            _ => value_text(v),
        };
        let (before, after) = (text(&pair[0]), text(&pair[1]));
        let after = after.unwrap_or_else(|| "清空".to_owned());
        parts.push(match before {
            None => format!("{label}{sep}{after}"),
            Some(b) => format!("{label}{sep}{b} → {after}"),
        });
    }
    parts.join("、")
}

/// 計時的欄位：開始、結束時間跟這則的時間重複，工時另外接在最後，都不列在改了什麼裡。
const TIMER_FIELDS: [&str; 4] = ["startedAt", "endedAt", "durationSeconds", "stopReason"];

/// Hayaku 1.5.5 起 action 是中文（「修改了任務」「暫停了計時」）：動作＋改了什麼；
/// 申請類型、經誰申請接在動作後面。什麼都組不出來時用伺服器的 summary（去掉開頭的人名）。
fn localized_text(i: &Value, action: &str, changes: &Value, duration: &str) -> String {
    let new = |field: &str| -> Option<String> {
        match prop(changes, field) {
            Some(Value::Array(pair)) if pair.len() == 2 => value_text(&pair[1]),
            _ => None,
        }
    };
    let mut skip: Vec<&str> = TIMER_FIELDS.to_vec();
    let mut head = match new("stopReason").as_deref() {
        Some("BREAK") => "休息時間自動暫停".to_owned(),
        Some("END" | "OFF_DUTY") => "下班自動暫停".to_owned(),
        _ => action.to_owned(),
    };
    if action.contains("變更申請")
        && let Some(t) = new("type")
    {
        head += &format!("（{t}）");
        skip.push("type");
    }
    if action.contains("子任務")
        && let Some(sub) = new("subtask")
    {
        head += &format!(" {sub}");
        skip.push("subtask");
    } else if action.contains("建立")
        && let Some(title) = new("title")
    {
        // 建立時帶的整份初始欄位太長，跟以前一樣只寫名稱
        return format!("{head}「{title}」{duration}");
    }
    if let Some(by) = prop(i, "requestedBy").and_then(person) {
        head += &format!("（經 {} 申請）", by.display_name);
    }
    let details = field_changes_with(changes, &skip, " ");
    let text = if details.is_empty() { head } else { format!("{head}：{details}") };
    if !text.is_empty() {
        return text + duration;
    }
    let summary = str_(i, "summary").unwrap_or_default();
    let actor = prop(i, "actor").and_then(person).map(|p| p.display_name).unwrap_or_default();
    summary.strip_prefix(&format!("{actor} ")).unwrap_or(summary).to_owned()
}

/// 這則對變更申請做了什麼：舊版是 change_request.* 代碼，1.5.5 起是中文動作＋changeRequestId。
fn request_step(i: &Value) -> Option<RequestStep> {
    let action = str_(i, "action").unwrap_or_default();
    let submitted_type = || match prop(obj(i, "changes"), "type") {
        Some(Value::Array(pair)) if pair.len() == 2 => match value_text(&pair[1]).unwrap_or_default().as_str() {
            "CREATE" => "新增".to_owned(),
            "UPDATE" => "修改".to_owned(),
            "DELETE" => "刪除".to_owned(),
            other => other.to_owned(),
        },
        _ => String::new(),
    };
    if let Some(code) = action.strip_prefix("change_request.") {
        return Some(match code {
            "create" => RequestStep::Submit(submitted_type()),
            "approve" => RequestStep::Approve,
            "reject" => RequestStep::Reject,
            "withdraw" => RequestStep::Withdraw,
            _ => RequestStep::Other,
        });
    }
    str_(i, "changeRequestId")?;
    Some(if action.contains("提出") {
        RequestStep::Submit(submitted_type())
    } else if action.contains("核准") {
        RequestStep::Approve
    } else if action.contains("拒絕") || action.contains("駁回") {
        RequestStep::Reject
    } else if action.contains("撤回") {
        RequestStep::Withdraw
    } else {
        RequestStep::Other
    })
}

/// 一則異動 → 中文說明；認不得的動作照原本的代碼顯示。
fn activity_text(i: &Value) -> String {
    if matches!(str_(i, "type"), Some("comment" | "chat")) {
        return format!("留言：{}", str_(i, "body").or_else(|| str_(i, "text")).unwrap_or(""));
    }
    let changes = obj(i, "changes");
    let new = |field: &str| -> Option<String> {
        match prop(changes, field) {
            Some(Value::Array(pair)) if pair.len() == 2 => value_text(&pair[1]),
            _ => None,
        }
    };
    let duration = match prop(changes, "durationSeconds") {
        Some(Value::Array(ds)) if ds.len() == 2 => match as_f64(&ds[1]) {
            Some(secs) => format!("（{}）", WorkMath::hours_minutes(secs / 60.0)),
            None => String::new(),
        },
        _ => String::new(),
    };
    let Some(action) = str_(i, "action") else {
        return String::new();
    };
    if action.is_empty() || !action.is_ascii() {
        return localized_text(i, action, changes, &duration);
    }
    match action {
        "task.create" => format!("建立任務「{}」", new("title").unwrap_or_default()),
        "task.subtask.create" => format!("新增子任務 {}", new("subtask").unwrap_or_default()),
        "task.update" => format!("修改：{}", field_changes(changes, None)),
        "task.delete" => "刪除任務".to_owned(),
        "task.restore" => "還原任務".to_owned(),
        "task.move" => format!("移動到 {}", new("parent").unwrap_or_else(|| "最上層".to_owned())),
        "time_entry.start" => "開始計時".to_owned(),
        "time_entry.pause" => format!("暫停計時{duration}"),
        "time_entry.auto_pause" => {
            let head = match new("stopReason").as_deref() {
                Some("BREAK") => "休息時間自動暫停",
                Some("END" | "OFF_DUTY") => "下班自動暫停",
                _ => "自動暫停計時",
            };
            format!("{head}{duration}")
        }
        "time_entry.discard" => format!("捨棄太短的計時{duration}"),
        "time_entry.update" => "修改計時段".to_owned(),
        "time_entry.create" | "time_entry.add" => format!("補登工時{duration}"),
        "time_entry.delete" => "刪除計時段".to_owned(),
        "time_entry.move" => "計時段移到別的任務".to_owned(),
        "change_request.create" => format!("送出變更申請：{}", field_changes(changes, Some("type"))),
        "change_request.update" => format!("修改變更申請：{}", field_changes(changes, Some("type"))),
        "change_request.approve" => "核准變更申請".to_owned(),
        "change_request.reject" => "拒絕變更申請".to_owned(),
        "change_request.withdraw" => "撤回變更申請".to_owned(),
        a if a.starts_with("dependency.") => {
            if a.ends_with("delete") || a.ends_with("remove") { "移除相依" } else { "設定相依" }.to_owned()
        }
        a if a.starts_with("attachment.") => {
            if a.ends_with("delete") {
                "刪除附件".to_owned()
            } else {
                format!("上傳附件 {}", new("name").unwrap_or_default()).trim_end().to_owned()
            }
        }
        a => a.to_owned(),
    }
}

impl HayakuApi {
    /// 新增相依時可選的常用規則（template 代碼 → 說明）。
    pub const DEPENDENCY_TEMPLATES: [(&'static str, &'static str); 4] = [
        ("finishToStart", "完成後才能開始"),
        ("finishToFinish", "完成後才能完成"),
        ("finishThenTodo", "完成後自動變成待處理"),
        ("finishThenStart", "完成後自動開始"),
    ];

    /// 整棵子任務樹（照樹狀順序攤平）；第二個值：太大被截掉了。
    pub async fn task_tree(&self, task: &str, depth: i32) -> Result<(Vec<TaskTreeRow>, bool), McpError> {
        let r = self.call("get_task_tree", json!({ "task": task, "depth": depth })).await?;
        Ok((Self::parse_task_tree(&r), bool_(&r, "truncated")))
    }

    pub fn parse_task_tree(r: &Value) -> Vec<TaskTreeRow> {
        fn walk(rows: &mut Vec<TaskTreeRow>, node: &Value, depth: i32) {
            for c in items(node, &["children"]) {
                let Some(key) = first_key(c, &["task", "key"]) else { continue };
                let assignees = items(c, &["assignees"])
                    .iter()
                    .filter_map(|a| match a.as_str() {
                        Some(s) => Some(s.to_owned()),
                        None => person(a).map(|p| p.display_name).or_else(|| first_string(a, &["name"])),
                    })
                    .filter(|a| !is_blank(a))
                    .collect();
                rows.push(TaskTreeRow {
                    key,
                    title: first_string(c, &["title"]).unwrap_or_default(),
                    status: first_string(c, &["status"]).unwrap_or_default(),
                    status_category: first_string(c, &["statusCategory"]),
                    depth,
                    assignees,
                    priority: first_string(c, &["priority"]),
                    planned_start: day(c, "plannedStart"),
                    planned_end: day(c, "plannedEnd"),
                });
                walk(rows, c, depth + 1);
            }
        }
        let mut rows = Vec::new();
        walk(&mut rows, r, 1);
        rows
    }

    pub async fn list_dependencies(&self, task: &str) -> Result<DependencyInfo, McpError> {
        Ok(Self::parse_dependencies(&self.call("list_dependencies", json!({ "task": task })).await?))
    }

    /// waits_for：這個任務等 other（other 是前置）；否則 other 等這個任務。回傳伺服器的說明（執行者會變成變更申請）。
    pub async fn add_dependency(
        &self,
        task: &str,
        other: &str,
        waits_for: bool,
        template: &str,
    ) -> Result<Option<String>, McpError> {
        let mut args = Map::new();
        args.insert("task".into(), json!(task));
        args.insert(if waits_for { "waitsFor" } else { "blocks" }.into(), json!(other));
        args.insert("template".into(), json!(template));
        Ok(message(&self.call("add_dependency", Value::Object(args)).await?))
    }

    pub async fn remove_dependency(&self, id: &str) -> Result<Option<String>, McpError> {
        Ok(message(&self.call("remove_dependency", json!({ "id": id })).await?))
    }

    /// 附件清單；第二個值是網頁上這個任務的附件區（下載、上傳要到網頁）。
    pub async fn list_attachments(&self, task: &str) -> Result<(Vec<AttachmentItem>, Option<String>), McpError> {
        let r = self.call("list_attachments", json!({ "task": task })).await?;
        Ok((Self::parse_attachments(&r), string(&r, "webUrl")))
    }

    pub async fn get_task_content(&self, task: &str) -> Result<Vec<ContentDoc>, McpError> {
        Ok(Self::parse_content(&self.call("get_content", json!({ "task": task })).await?))
    }

    /// 異動紀錄與留言（新到舊，一頁 50 筆）；回傳下一頁的 cursor。filter：all／comments／changes。
    pub async fn task_activity(
        &self,
        task: &str,
        filter: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<TaskActivityItem>, Option<String>), McpError> {
        let mut args = Map::new();
        args.insert("task".into(), json!(task));
        args.insert("filter".into(), json!(filter));
        if let Some(c) = cursor {
            args.insert("cursor".into(), json!(c));
        }
        let r = self.call("get_task_activity", Value::Object(args)).await?;
        Ok((Self::parse_task_activity(&r), string(&r, "nextCursor").or_else(|| string(&r, "cursor"))))
    }

    /// 軟刪除（含子任務），可以從網頁垃圾桶還原；執行者會變成刪除申請（reason 給審核者看）。
    pub async fn delete_task(&self, task: &str, reason: Option<&str>) -> Result<Option<String>, McpError> {
        let mut args = Map::new();
        args.insert("task".into(), json!(task));
        args.insert("confirm".into(), json!(true));
        if let Some(r) = reason.filter(|r| !is_blank(r)) {
            args.insert("reason".into(), json!(r));
        }
        Ok(message(&self.call("delete_task", Value::Object(args)).await?))
    }

    // ───────────── 解析 ─────────────

    /// 關聯任務可能另外放在 related，也可能混在前置／後續裡（kind: related），都挑出來放 related。
    pub fn parse_dependencies(r: &Value) -> DependencyInfo {
        fn is_related(d: &Value) -> bool {
            first_str(d, &["kind", "type"]).is_some_and(|k| k.eq_ignore_ascii_case("related"))
        }
        let predecessors = items(r, &["predecessors"]);
        let successors = items(r, &["successors"]);
        let mut related: Vec<DependencyItem> = Vec::new();
        let candidates = items(r, &["related", "relatedTasks"])
            .iter()
            .chain(predecessors.iter().chain(successors.iter()).filter(|d| is_related(d)));
        for d in candidates {
            let Some(mut item) = Self::parse_dependency(d, "related") else { continue };
            item.rule = String::new();
            item.blocking = false;
            let upper = item.other_key.to_uppercase();
            if !related.iter().any(|x| x.other_key.to_uppercase() == upper) {
                related.push(item);
            }
        }
        DependencyInfo {
            predecessors: predecessors
                .iter()
                .filter(|d| !is_related(d))
                .filter_map(|d| Self::parse_dependency(d, "predecessor"))
                .collect(),
            successors: successors
                .iter()
                .filter(|d| !is_related(d))
                .filter_map(|d| Self::parse_dependency(d, "successor"))
                .collect(),
            related: Some(related),
        }
    }

    /// 另一端的任務可能放在 task、predecessor／successor、other 底下，或直接在這一層。
    fn parse_dependency(d: &Value, side: &str) -> Option<DependencyItem> {
        if let Some(s) = d.as_str() {
            return KeyFinder::parse(Some(s)).map(|k| DependencyItem {
                id: String::new(),
                other_key: k,
                other_title: String::new(),
                other_status: None,
                rule: String::new(),
                blocking: false,
            });
        }
        if !d.is_object() {
            return None;
        }
        let source =
            [side, "task", "other"].iter().find_map(|name| prop(d, name).filter(|o| o.is_object())).unwrap_or(d);
        let key = string(source, "task")
            .or_else(|| string(source, "key"))
            .or_else(|| string(d, "taskKey"))
            .or_else(|| string(d, &format!("{side}Key")))?;
        let template = str_(d, "template");
        let rule = string(d, "templateName")
            .or_else(|| string(d, "label"))
            .or_else(|| string(d, "description"))
            .or_else(|| {
                Self::DEPENDENCY_TEMPLATES
                    .iter()
                    .find(|(code, _)| Some(*code) == template)
                    .map(|(_, label)| (*label).to_owned())
            })
            .unwrap_or_else(|| {
                let head = match str_(d, "kind") {
                    Some("GATE") => "限制",
                    Some("TRIGGER") => "自動",
                    _ => "",
                };
                let cond = match prop(d, "condition") {
                    Some(Value::Array(c)) => {
                        format!("（{}）", c.iter().map(element_to_string).collect::<Vec<_>>().join("、"))
                    }
                    _ => String::new(),
                };
                format!("{head}{cond}")
            });
        let blocking = ["blocking", "isBlocking", "blocked"].iter().any(|n| bool_(d, n));
        Some(DependencyItem {
            id: string(d, "id").unwrap_or_default(),
            other_key: key,
            other_title: string(source, "title").unwrap_or_default(),
            other_status: string(source, "status"),
            rule,
            blocking,
        })
    }

    pub fn parse_attachments(r: &Value) -> Vec<AttachmentItem> {
        items(r, &["attachments"])
            .iter()
            .filter(|a| a.is_object())
            .map(|a| AttachmentItem {
                name: string(a, "name")
                    .or_else(|| string(a, "fileName"))
                    .or_else(|| string(a, "filename"))
                    .unwrap_or_else(|| "（沒有檔名）".to_owned()),
                size: match prop(a, "size").and_then(as_i64) {
                    Some(bytes) => Some(bytes),
                    None => prop(a, "sizeBytes").and_then(as_i64),
                },
                uploader: ["uploadedBy", "uploader", "createdBy"].iter().find_map(|n| prop(a, n)).and_then(|u| match u
                    .as_str()
                {
                    Some(s) => Some(s.to_owned()),
                    None => person(u).map(|p| p.display_name),
                }),
                at: date(a, "createdAt").or_else(|| date(a, "uploadedAt")),
                url: string(a, "webUrl").or_else(|| string(a, "url")),
                id: first_string(a, &["id", "attachmentId", "fileId"]),
            })
            .collect()
    }

    pub fn parse_content(r: &Value) -> Vec<ContentDoc> {
        let mut list: Vec<ContentDoc> = items(r, &["documents"])
            .iter()
            .map(|d| ContentDoc {
                title: string(d, "title").unwrap_or_else(|| "文件".to_owned()),
                text: string(d, "markdown").unwrap_or_default(),
            })
            .collect();
        for w in items(r, &["whiteboards"]) {
            let texts: Vec<&str> = match prop(w, "texts") {
                Some(Value::Array(t)) => t
                    .iter()
                    .filter_map(|x| match x.as_str() {
                        Some(s) => Some(s),
                        None => str_(x, "text"),
                    })
                    .filter(|x| !is_blank(x))
                    .collect(),
                _ => Vec::new(),
            };
            list.push(ContentDoc {
                title: format!("白板：{}", str_(w, "title").unwrap_or("白板")),
                text: texts.join("\n"),
            });
        }
        list
    }

    pub fn parse_task_activity(r: &Value) -> Vec<TaskActivityItem> {
        items(r, &["items"])
            .iter()
            .filter(|i| i.is_object())
            .map(|i| {
                let actor = prop(i, "actor").and_then(person).map(|p| p.display_name);
                let source = str_(i, "source");
                let mut text = activity_text(i);
                if source == Some("MCP") {
                    text += "（透過 MCP）";
                }
                if bool_(i, "undone") {
                    text += "（已復原）";
                }
                TaskActivityItem {
                    at: date(i, "createdAt").unwrap_or_else(min_value),
                    actor: actor.unwrap_or_else(|| if source == Some("SYSTEM") { "系統" } else { "" }.to_owned()),
                    text,
                    comment: matches!(str_(i, "type"), Some("comment" | "chat")),
                    change_request: string(i, "changeRequestId"),
                    request_step: request_step(i),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 TaskExtrasTests 的 API 解析部分

    #[test]
    fn 異動紀錄_換成中文說明() {
        let items = HayakuApi::parse_task_activity(&j(r#"
            {"items":[
              {"type":"change","action":"task.update","actor":{"username":"a","displayName":"小明"},"source":"MCP",
               "changes":{"title":["第一階段研究","第一階段"],"status":["客戶驗證中","處理中"]},"undone":false,"createdAt":"2026-10-30T05:19:49.782Z"},
              {"type":"change","action":"time_entry.auto_pause","actor":null,"source":"SYSTEM",
               "changes":{"stopReason":[null,"BREAK"],"durationSeconds":[null,12600]},"undone":false,"createdAt":"2026-10-27T08:26:55.782Z"},
              {"type":"change","action":"task.subtask.create","actor":{"username":"a","displayName":"小明"},"source":"WEB",
               "changes":{"subtask":[null,"F2P-76 系統規劃"]},"undone":true,"createdAt":"2026-10-27T07:34:50.333Z"},
              {"type":"change","action":"something.new","actor":{"username":"a","displayName":"小明"},"createdAt":"2026-10-27T07:00:00Z"}
            ],"nextCursor":null}"#));
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].text, "修改：名稱：第一階段研究 → 第一階段、狀態：客戶驗證中 → 處理中（透過 MCP）");
        assert_eq!(items[0].actor, "小明");
        assert_eq!(items[1].text, "休息時間自動暫停（3 小時 30 分）");
        assert_eq!(items[1].actor, "系統");
        assert_eq!(items[2].text, "新增子任務 F2P-76 系統規劃（已復原）");
        assert_eq!(items[3].text, "something.new");
    }

    #[test]
    fn 異動紀錄_1_5_5_中文動作() {
        // 實際回應（2026-11-16，F2P-97），時間與 id 簡化
        let items = HayakuApi::parse_task_activity(&j(r#"
            {"items":[
              {"type":"change","action":"修改了任務","actor":{"username":"a","displayName":"小明"},"requestedBy":null,"source":"WEB",
               "changes":{"status":["客戶驗證中","已完成"],"actualEndDate":[null,"2026-10-29"]},"undone":false,"changeRequestId":null,
               "createdAt":"2026-10-29T08:18:25.728Z","summary":"小明 修改了任務：狀態 客戶驗證中 → 已完成；實際結束 2026-10-29"},
              {"type":"change","action":"暫停了計時","actor":{"username":"a","displayName":"小明"},"source":"MCP",
               "changes":{"endedAt":[null,"2026-10-28T03:21:35.446Z"],"durationSeconds":[null,10394]},"createdAt":"2026-10-28T03:21:35.458Z"},
              {"type":"change","action":"核准了任務","actor":{"username":"b","displayName":"小華"},"requestedBy":{"username":"a","displayName":"小明"},
               "source":"WEB","changes":{"approvalState":["待核准","已核准"]},"changeRequestId":"cr1","createdAt":"2026-10-27T09:23:04.489Z"},
              {"type":"change","action":"提出了變更申請","actor":{"username":"a","displayName":"小明"},"source":"WEB",
               "changes":{"type":[null,"新增"]},"changeRequestId":"cr1","createdAt":"2026-10-27T08:53:13.111Z"},
              {"type":"change","action":"建立了任務","actor":{"username":"a","displayName":"小明"},"source":"WEB",
               "changes":{"title":[null,"報表頁面"],"status":[null,"未開始"]},"createdAt":"2026-10-27T08:53:13.094Z"},
              {"type":"change","action":"修改了任務","actor":{"username":"a","displayName":"小明"},"source":"WEB",
               "changes":{"type":["一般","測試"]},"createdAt":"2026-10-27T08:00:00Z"},
              {"type":"change","action":"自動暫停了計時","actor":null,"source":"SYSTEM",
               "changes":{"stopReason":[null,"BREAK"],"durationSeconds":[null,12600]},"createdAt":"2026-10-27T07:00:00Z"},
              {"type":"change","action":"","actor":{"username":"a","displayName":"小明"},"changes":null,
               "summary":"小明 做了某件事","createdAt":"2026-10-27T06:00:00Z"},
              {"type":"comment","body":"好","actor":{"username":"a","displayName":"小明"},"createdAt":"2026-10-27T05:00:00Z"}
            ],"nextCursor":null}"#));
        assert_eq!(items[0].text, "修改了任務：狀態 客戶驗證中 → 已完成、實際結束 2026-10-29");
        assert_eq!((items[0].change_request.as_deref(), items[0].request_step.as_ref()), (None, None));
        assert_eq!(items[1].text, "暫停了計時（2 小時 53 分）（透過 MCP）");
        assert_eq!(items[2].text, "核准了任務（經 小明 申請）：核准狀態 待核准 → 已核准");
        assert_eq!(
            (items[2].change_request.as_deref(), items[2].request_step.as_ref()),
            (Some("cr1"), Some(&RequestStep::Approve))
        );
        assert_eq!(items[3].text, "提出了變更申請（新增）");
        assert_eq!(items[3].request_step, Some(RequestStep::Submit("新增".into())));
        assert_eq!(items[4].text, "建立了任務「報表頁面」");
        assert_eq!(items[5].text, "修改了任務：類型 一般 → 測試");
        assert_eq!((items[6].text.as_str(), items[6].actor.as_str()), ("休息時間自動暫停（3 小時 30 分）", "系統"));
        assert_eq!(items[7].text, "做了某件事");
        assert!(items[8].comment && !items[0].comment);
    }

    #[test]
    fn 異動紀錄_舊代碼的變更申請也認得步驟() {
        let items = HayakuApi::parse_task_activity(&j(r#"
            {"items":[
              {"type":"change","action":"change_request.reject","createdAt":"2026-10-27T09:00:00Z"},
              {"type":"change","action":"change_request.create","changes":{"type":[null,"UPDATE"]},"createdAt":"2026-10-27T08:00:00Z"}
            ]}"#));
        assert_eq!(items[0].request_step, Some(RequestStep::Reject));
        assert_eq!(items[1].request_step, Some(RequestStep::Submit("修改".into())));
        assert_eq!(RequestStep::Reject.state_text(), "已拒絕");
    }

    #[test]
    fn 文件與白板() {
        let docs = HayakuApi::parse_content(&j(r##"
            {"task":"F2P-27","documents":[{"id":"x","title":"文件","markdown":"# 標題\n內容"}],
             "whiteboards":[{"title":"流程","texts":["開始","結束"]}]}"##));
        assert_eq!(docs.len(), 2);
        assert_eq!((docs[0].title.as_str(), docs[0].text.as_str()), ("文件", "# 標題\n內容"));
        assert_eq!((docs[1].title.as_str(), docs[1].text.as_str()), ("白板：流程", "開始\n結束"));
    }

    #[test]
    fn 相依關係_另一端的任務放在不同位置都認得() {
        let deps = HayakuApi::parse_dependencies(&j(r#"
            {"predecessors":[
               {"id":"d1","template":"finishToStart","blocking":true,"task":{"task":"F2P-10","title":"設計","status":"處理中"}},
               {"id":"d2","kind":"GATE","condition":["完成"],"predecessor":{"key":"F2P-11","title":"審核"}}],
             "successors":[{"id":"d3","templateName":"完成後自動開始","taskKey":"F2P-20"}]}"#));
        assert_eq!(deps.predecessors.len(), 2);
        let p0 = &deps.predecessors[0];
        assert_eq!(
            (p0.other_key.as_str(), p0.other_title.as_str(), p0.other_status.as_deref(), p0.rule.as_str(), p0.blocking),
            ("F2P-10", "設計", Some("處理中"), "完成後才能開始", true)
        );
        let p1 = &deps.predecessors[1];
        assert_eq!((p1.other_key.as_str(), p1.rule.as_str(), p1.blocking), ("F2P-11", "限制（完成）", false));
        let s0 = &deps.successors[0];
        assert_eq!((s0.id.as_str(), s0.other_key.as_str(), s0.rule.as_str()), ("d3", "F2P-20", "完成後自動開始"));
    }

    #[test]
    fn 附件() {
        let items = HayakuApi::parse_attachments(&j(r#"
            {"webUrl":"https://h/x","attachments":[{"name":"規格.pdf","size":2048,"uploadedBy":{"username":"a","displayName":"小明"},
              "createdAt":"2026-10-30T01:00:00Z","webUrl":"https://h/a"},{"fileName":"b.png"}]}"#));
        assert_eq!(
            (items[0].name.as_str(), items[0].size, items[0].uploader.as_deref(), items[0].url.as_deref()),
            ("規格.pdf", Some(2048), Some("小明"), Some("https://h/a"))
        );
        assert_eq!((items[1].name.as_str(), items[1].size), ("b.png", None));
    }

    // 對應 TestingTests.ParseDependencies_關聯任務另外放
    #[test]
    fn parse_dependencies_關聯任務另外放() {
        let deps = HayakuApi::parse_dependencies(&j(r#"
            {"predecessors":[{"id":"d1","task":{"task":"F2P-1","title":"前置"},"template":"finishToStart"},
                             {"kind":"related","task":{"task":"EG-3","title":"跨專案的"}}],
             "successors":[],"related":[{"task":"F2P-7","title":"相關的","status":"處理中"}],"timezone":"Asia/Taipei"}"#));
        assert_eq!(deps.predecessors.len(), 1);
        assert_eq!(deps.predecessors[0].other_key, "F2P-1");
        assert_eq!(
            deps.related_tasks().iter().map(|d| d.other_key.as_str()).collect::<Vec<_>>(),
            vec!["F2P-7", "EG-3"]
        );
        assert!(
            HayakuApi::parse_dependencies(&j(r#"{"predecessors":[],"successors":[],"timezone":"Asia/Taipei"}"#))
                .related_tasks()
                .is_empty()
        );
    }

    // 對應 NewToolsTests.ParseTaskTree_照樹狀順序攤平並記層數
    #[test]
    fn parse_task_tree_照樹狀順序攤平並記層數() {
        let rows = HayakuApi::parse_task_tree(&j(r#"
            {"task":"F2P-14","title":"機密列印","children":[
              {"task":"F2P-15","title":"事務機APP開發","status":"已完成","statusCategory":"DONE","assignees":["harry.gu"],"children":[]},
              {"task":"F2P-16","title":"Server端開發","status":"已完成","statusCategory":"DONE","assignees":["nick.chio"],"children":[
                {"task":"F2P-17","title":"規劃研究","status":"已完成","statusCategory":"DONE","assignees":["nick.chio"],"children":[]}]},
              {"task":"F2P-101","title":"面板新增全部列印及選擇列印按鈕","status":"處理中","statusCategory":"IN_PROGRESS","assignees":[],"children":[]}],
             "truncated":false}"#));
        assert_eq!(
            rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
            vec!["F2P-15", "F2P-16", "F2P-17", "F2P-101"]
        );
        assert_eq!(rows.iter().map(|r| r.depth).collect::<Vec<_>>(), vec![1, 1, 2, 1]);
        let more = HayakuApi::parse_task_tree(&j(r#"
            {"children":[{"task":"F2P-6","title":"a","priority":"高","plannedStart":"2026-09-28","plannedEnd":"2026-10-07","children":[]}]}"#));
        assert_eq!(
            (more[0].priority.as_deref(), more[0].planned_end, rows[0].planned_end),
            (Some("高"), chrono::NaiveDate::from_ymd_opt(2026, 10, 7), None)
        );
        assert_eq!(rows[2].assignees.len(), 1);
        assert_eq!(
            (
                rows[2].title.as_str(),
                rows[2].status.as_str(),
                rows[2].status_category.as_deref(),
                rows[2].assignees[0].as_str()
            ),
            ("規劃研究", "已完成", Some("DONE"), "nick.chio")
        );
    }
}
