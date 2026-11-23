//! 會議詳細、修改、會議紀錄、檔案、白板。對應 C# `Mcp/HayakuApi.Meetings.cs`。

use serde_json::{Map, Value, json};

use super::HayakuApi;
use crate::json::*;
use crate::models::*;
use crate::transport::McpError;

const WEEKDAY_NAMES: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];

impl HayakuApi {
    pub async fn get_meeting(&self, meeting: &str) -> Result<MeetingDetail, McpError> {
        let r = self.call("get_meeting", json!({ "meeting": meeting })).await?;
        Self::parse_meeting_detail(&r).ok_or_else(|| McpError(format!("讀不到會議 {meeting}")))
    }

    /// 只給要改的欄位（title、startAt、endAt、location、note、addAttendees、removeAttendees、scope…），照 update_meeting 的參數。
    pub async fn update_meeting(&self, meeting: &str, fields: Map<String, Value>) -> Result<(), McpError> {
        let mut args = fields;
        args.insert("meeting".into(), json!(meeting));
        self.call("update_meeting", Value::Object(args)).await.map(drop)
    }

    pub async fn meeting_notes(&self, meeting: &str) -> Result<Vec<MeetingNote>, McpError> {
        Ok(Self::parse_meeting_notes(&self.call("get_meeting_notes", json!({ "meeting": meeting })).await?))
    }

    /// 第一份會議紀錄最近幾版的更新歷程（新到舊）。
    pub async fn meeting_note_history(&self, meeting: &str, limit: i32) -> Result<Vec<MeetingNoteVersion>, McpError> {
        Ok(Self::parse_meeting_note_history(
            &self.call("get_meeting_note_history", json!({ "meeting": meeting, "limit": limit })).await?,
        ))
    }

    pub async fn meeting_files(&self, meeting: &str) -> Result<Vec<MeetingFile>, McpError> {
        Ok(Self::parse_meeting_files(&self.call("list_meeting_files", json!({ "meeting": meeting })).await?))
    }

    pub async fn meeting_whiteboards(&self, meeting: &str) -> Result<Vec<MeetingWhiteboard>, McpError> {
        Ok(Self::parse_whiteboards(&self.call("get_meeting_whiteboards", json!({ "meeting": meeting })).await?))
    }

    pub fn parse_meeting_detail(r: &Value) -> Option<MeetingDetail> {
        if !r.is_object() {
            return None;
        }
        let start = date(r, "startAt")?;
        let end = date(r, "endAt")?;
        let attendees = items(r, &["attendees"])
            .iter()
            .map(|a| MeetingAttendee {
                username: first_string(a, &["user", "username"]).unwrap_or_default(),
                name: first_string(a, &["name", "displayName", "user"]).unwrap_or_default(),
                state: first_string(a, &["state", "status"]).unwrap_or_default(),
                logged_minutes: match prop(a, "logged") {
                    Some(l @ Value::Object(_)) => prop(l, "minutes").and_then(as_i32).unwrap_or(0),
                    _ => 0,
                },
            })
            .filter(|a| !a.name.is_empty())
            .collect();
        let recurring = bool_(r, "recurring");
        let series = prop(r, "series").unwrap_or(&NULL);
        Some(MeetingDetail {
            key: first_string(r, &["meeting", "id"]).unwrap_or_default(),
            id: string(r, "id").unwrap_or_default(),
            title: string(r, "title").unwrap_or_default(),
            project: string(r, "project").unwrap_or_default(),
            start_at: start,
            end_at: end,
            location: string(r, "location"),
            recurring: recurring || series.is_object(),
            recurrence: Self::describe_recurrence(series)
                .or_else(|| if recurring { Some("重複會議".to_owned()) } else { None }),
            organizer: first_string(r, &["organizer", "host"]),
            my_state: string(r, "me"),
            attendees,
            note: string(r, "note"),
            can_edit: bool_(r, "canEdit"),
            space_binding: Self::parse_space_binding(r),
            tasks: Self::parse_related_tasks(r),
        })
    }

    /// spaceBinding：沒綁是 null；可能是 { space: { id, name }, folderId… }、{ spaceId, spaceName… } 或直接是空間 id。
    /// 沒有實際資料，照說明寬鬆猜。
    pub fn parse_space_binding(r: &Value) -> Option<SpaceBinding> {
        let b = prop(r, "spaceBinding")?;
        match b {
            Value::String(s) if !is_blank(s) => {
                Some(SpaceBinding { space_id: Some(s.clone()), space_name: None, folder_id: None })
            }
            Value::Object(_) => {
                let space = prop(b, "space").unwrap_or(&NULL);
                let (space_id, space_name) = match space {
                    Value::String(s) => (Some(s.clone()), None),
                    Value::Object(_) => (first_string(space, &["id", "spaceId"]), first_string(space, &["name", "title"])),
                    _ => (None, None),
                };
                let folder = prop(b, "folder").unwrap_or(&NULL);
                let binding = SpaceBinding {
                    space_id: space_id.or_else(|| first_string(b, &["spaceId"])),
                    space_name: space_name.or_else(|| first_string(b, &["spaceName", "spaceTitle", "name", "title"])),
                    folder_id: first_string(b, &["folderId", "folderPageId", "pageId", "rootPageId"]).or_else(|| match folder {
                        Value::String(s) => Some(s.clone()),
                        _ => first_string(folder, &["id", "pageId"]),
                    }),
                };
                (binding.space_id.is_some() || binding.space_name.is_some() || binding.folder_id.is_some())
                    .then_some(binding)
            }
            _ => None,
        }
    }

    /// 關聯來源（紀錄提到、內容提到、手動加入）：字串、字串陣列、{ type／kind } 陣列，或 true／false 的旗標都接受。
    fn parse_link_sources(x: &Value) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut push = |s: String| {
            if !is_blank(&s) && !out.contains(&s) {
                out.push(s);
            }
        };
        for name in ["sources", "source", "via", "reasons", "reason", "relations", "relation"] {
            match prop(x, name) {
                Some(Value::String(s)) => push(link_source_text(s)),
                Some(Value::Array(arr)) => {
                    for v in arr {
                        match v.as_str() {
                            Some(s) => push(link_source_text(s)),
                            None => {
                                if let Some(s) = first_str(v, &["type", "kind", "source", "label"]) {
                                    push(link_source_text(s));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let flags: [(&[&str], &str); 3] = [
            (&["inNote", "inNotes", "noteMention", "mentionedInNote", "fromNote"], "紀錄提到"),
            (&["inContent", "contentMention", "mentionedInContent", "fromContent"], "內容提到"),
            (&["manual", "manually"], "手動加入"),
        ];
        for (names, text) in flags {
            if names.iter().any(|f| matches!(prop(x, f), Some(Value::Bool(true)))) {
                push(text.to_owned());
            }
        }
        out
    }

    /// get_task 的 meetings：看不到的只有 id（或直接是字串 id）。
    pub fn parse_related_meetings(r: &Value) -> Vec<RelatedMeeting> {
        items(r, &["meetings", "relatedMeetings"])
            .iter()
            .filter_map(|m| {
                if let Some(s) = m.as_str().filter(|s| !is_blank(s)) {
                    return Some(RelatedMeeting {
                        key: s.to_owned(),
                        id: Some(s.to_owned()),
                        title: String::new(),
                        project: None,
                        start_at: None,
                        end_at: None,
                        sources: Vec::new(),
                        visible: false,
                    });
                }
                let id = id_text(m, "id");
                let key = first_string(m, &["meeting", "key", "code"]).or_else(|| id.clone())?;
                let title = first_string(m, &["title", "name"]).unwrap_or_default();
                Some(RelatedMeeting {
                    key,
                    id,
                    visible: !title.is_empty() && !matches!(prop(m, "canOpen"), Some(Value::Bool(false))),
                    title,
                    project: first_string(m, &["project", "projectKey"]),
                    start_at: first_date(m, &["startAt", "start"]),
                    end_at: first_date(m, &["endAt", "end"]),
                    sources: Self::parse_link_sources(m),
                })
            })
            .collect()
    }

    /// get_meeting 的 tasks：看不到的只有 id（或直接是字串）。
    pub fn parse_related_tasks(r: &Value) -> Vec<RelatedTask> {
        items(r, &["tasks", "relatedTasks"])
            .iter()
            .filter_map(|t| {
                if let Some(s) = t.as_str().filter(|s| !is_blank(s)) {
                    return Some(RelatedTask {
                        key: s.to_owned(),
                        title: String::new(),
                        status: String::new(),
                        status_category: String::new(),
                        assignees: Vec::new(),
                        planned_end: None,
                        sources: Vec::new(),
                        visible: false,
                    });
                }
                let key = first_string(t, &["task", "key", "code"]).or_else(|| id_text(t, "id"))?;
                let title = first_string(t, &["title", "name"]).unwrap_or_default();
                let mut assignees = names(t, "assignees");
                if assignees.is_empty()
                    && let Some(a) = person_name(t, &["assignee"])
                {
                    assignees.push(a);
                }
                Some(RelatedTask {
                    key,
                    visible: !title.is_empty() && !matches!(prop(t, "canOpen"), Some(Value::Bool(false))),
                    title,
                    status: first_string(t, &["status", "statusName"]).unwrap_or_default(),
                    status_category: string(t, "statusCategory").unwrap_or_default(),
                    assignees,
                    planned_end: ["plannedEnd", "dueDate", "due", "plannedEndDate"].iter().find_map(|n| day(t, n)),
                    sources: Self::parse_link_sources(t),
                })
            })
            .collect()
    }

    /// 重複規則 → 「每週一、三」「每 2 天（只算工作日）」；series 可能直接有說明文字，或有 rule（同 create_meeting 的 recurrence.rule）。
    /// 格式沒有固定樣本，看不懂回 None。
    pub fn describe_recurrence(series: &Value) -> Option<String> {
        if !series.is_object() {
            return None;
        }
        if let Some(text) = first_string(series, &["text", "label", "summary", "description"]) {
            return Some(text);
        }
        let rule = if obj(series, "rule").is_object() { obj(series, "rule") } else { series };
        let interval = prop(rule, "interval").and_then(as_i32).filter(|n| *n > 1).unwrap_or(1);
        let every = if interval > 1 { format!("每 {interval} ") } else { "每".to_owned() };
        let mut result = match str_(rule, "freq") {
            Some("DAILY") => {
                (if interval > 1 { format!("每 {interval} 天") } else { "每天".to_owned() })
                    + if bool_(rule, "workdaysOnly") { "（只算工作日）" } else { "" }
            }
            Some("WEEKLY") => {
                let head = if interval > 1 { format!("每 {interval} 週") } else { "每週".to_owned() };
                let days = match prop(rule, "weekdays") {
                    Some(Value::Array(wd)) => wd
                        .iter()
                        .filter_map(as_i32)
                        .filter(|i| (0..=6).contains(i))
                        .map(|i| WEEKDAY_NAMES[i as usize])
                        .collect::<Vec<_>>()
                        .join("、"),
                    _ => String::new(),
                };
                head + &days
            }
            Some("MONTHLY") => format!("{every}個月"),
            Some("YEARLY") => format!("{every}年"),
            _ => return None,
        };
        if let Some(until) = str_(series, "until").filter(|u| utf16_len(u) >= 10) {
            result += &format!("，到 {}", truncate_utf16(until, 10));
        }
        Some(result)
    }

    pub fn parse_meeting_notes(r: &Value) -> Vec<MeetingNote> {
        items(r, &["notes"])
            .iter()
            .map(|n| MeetingNote {
                title: first_string(n, &["title", "name"]).unwrap_or_else(|| "會議紀錄".to_owned()),
                updated_at: first_date(n, &["updatedAt"]),
                markdown: first_string(n, &["markdown", "content", "text"]).unwrap_or_default(),
            })
            .collect()
    }

    pub fn parse_meeting_note_history(r: &Value) -> Vec<MeetingNoteVersion> {
        items(r, &["versions", "items"])
            .iter()
            .map(|v| MeetingNoteVersion {
                editor: match prop(v, "editor") {
                    Some(e @ Value::Object(_)) => {
                        first_string(e, &["displayName", "name", "username"]).unwrap_or_default()
                    }
                    _ => first_string(v, &["editor"]).unwrap_or_default(),
                },
                at: first_date(v, &["endedAt", "startedAt", "at"]),
                summary: first_string(v, &["summary"]).unwrap_or_default(),
                diff: first_string(v, &["diff"]).unwrap_or_default(),
            })
            .collect()
    }

    pub fn parse_meeting_files(r: &Value) -> Vec<MeetingFile> {
        items(r, &["files", "items"])
            .iter()
            .map(|f| MeetingFile {
                id: string(f, "id").unwrap_or_default(),
                file_name: first_string(f, &["fileName", "name", "filename"])
                    .unwrap_or_else(|| "（沒有檔名）".to_owned()),
                size_bytes: prop(f, "sizeBytes").and_then(as_i64).unwrap_or(0),
                source: string(f, "source").unwrap_or_default(),
                uploaded_by: first_string(f, &["uploadedBy", "uploader"]),
                uploaded_at: first_date(f, &["uploadedAt", "createdAt"]),
            })
            .collect()
    }

    pub fn parse_whiteboards(r: &Value) -> Vec<MeetingWhiteboard> {
        items(r, &["whiteboards", "items"])
            .iter()
            .map(|b| MeetingWhiteboard {
                title: first_string(b, &["title", "name"]).unwrap_or_else(|| "白板".to_owned()),
                updated_at: first_date(b, &["updatedAt"]),
                text: match prop(b, "text") {
                    Some(Value::Array(t)) => t
                        .iter()
                        .filter_map(|x| match x.as_str() {
                            Some(s) => Some(s.to_owned()),
                            None => first_string(x, &["text", "content"]),
                        })
                        .filter(|x| !is_blank(x))
                        .collect(),
                    _ => Vec::new(),
                },
                element_count: prop(b, "elementCount").and_then(as_i32).unwrap_or(0),
                image_count: items(b, &["images"]).len() as i32,
                snapshots: prop(b, "snapshots").and_then(as_i32).unwrap_or(0),
                id: first_string(b, &["id", "whiteboardId"]),
                image_ids: Some(
                    items(b, &["images"])
                        .iter()
                        .filter_map(|i| match i.as_str() {
                            Some(s) => Some(s.to_owned()),
                            None => first_string(i, &["attachmentId", "id", "fileId"]),
                        })
                        .collect(),
                ),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 NewToolsTests.ParseMeetingNoteHistory_讀編輯者時間摘要與差異
    #[test]
    fn parse_meeting_note_history_讀編輯者時間摘要與差異() {
        let versions = HayakuApi::parse_meeting_note_history(&j(r#"
            {"meeting":"REVIVAL-9","note":"會議紀錄","versions":[
              {"editor":"邱子恩","startedAt":"2026-10-27T07:42:49.206Z","endedAt":"2026-10-27T07:44:12.090Z","summary":"＋1 段 · 改了 1 段",
               "stats":{"added":1,"changed":1,"checked":0,"removed":0,"tasksInserted":0},"diff":"～ {++ok++}\n＋ `123`"},
              {"editor":"羅瑞騰","startedAt":"2026-10-27T07:41:57.451Z","endedAt":"2026-10-27T07:42:10.167Z","summary":"＋1 段 · 插入 1 個任務",
               "diff":"＋ 任務 EMMHP-123 程式開發"}],"timezone":"Asia/Taipei"}"#));
        assert_eq!(versions.len(), 2);
        assert_eq!(
            (versions[0].editor.as_str(), versions[0].summary.as_str(), versions[0].diff.as_str()),
            ("邱子恩", "＋1 段 · 改了 1 段", "～ {++ok++}\n＋ `123`")
        );
        assert_eq!(versions[0].at, parse_date_time_offset("2026-10-27T07:44:12.090Z"));
        assert!(
            HayakuApi::parse_meeting_note_history(&j(r#"{"meeting":"REVIVAL-57","note":"會議紀錄","versions":[]}"#))
                .is_empty()
        );
    }

    #[test]
    fn parse_meeting_detail_空間綁定與相關任務() {
        let d = HayakuApi::parse_meeting_detail(&j(r#"
            {"meeting":"HYK-M3","id":"m3","title":"週會","project":"HYK","startAt":"2026-11-20T02:00:00Z","endAt":"2026-11-20T03:00:00Z",
             "spaceBinding":{"space":{"id":"s1","name":"產品空間"},"folderId":"f9"},
             "tasks":[{"task":"HYK-130","title":"登入頁","status":"進行中","statusCategory":"IN_PROGRESS",
                       "assignees":[{"username":"amy","displayName":"Amy"}],"plannedEnd":"2026-11-25","sources":["NOTE","MANUAL"]},
                      {"id":"t-hidden"},"HYK-9"]}"#))
        .unwrap();
        let b = d.space_binding.unwrap();
        assert_eq!((b.space_id.as_deref(), b.label().as_str(), b.folder_id.as_deref()), (Some("s1"), "產品空間", Some("f9")));
        assert_eq!(d.tasks.len(), 3);
        let t = &d.tasks[0];
        assert!(t.visible);
        assert_eq!((t.key.as_str(), t.status.as_str(), t.assignees.clone()), ("HYK-130", "進行中", vec!["Amy".to_owned()]));
        assert_eq!(t.planned_end, chrono::NaiveDate::from_ymd_opt(2026, 11, 25));
        assert_eq!(t.sources, vec!["紀錄提到", "手動加入"]);
        assert!(!d.tasks[1].visible && d.tasks[1].key == "t-hidden");
        assert!(!d.tasks[2].visible && d.tasks[2].key == "HYK-9");

        let none = HayakuApi::parse_meeting_detail(&j(
            r#"{"meeting":"HYK-M4","startAt":"2026-11-20T02:00:00Z","endAt":"2026-11-20T03:00:00Z","spaceBinding":null}"#,
        ))
        .unwrap();
        assert!(none.space_binding.is_none() && none.tasks.is_empty());
    }

    #[test]
    fn parse_related_meetings_任務的會議() {
        let list = HayakuApi::parse_related_meetings(&j(r#"
            {"meetings":[{"meeting":"HYK-M3","id":"m3","title":"週會","startAt":"2026-11-20T02:00:00Z","endAt":"2026-11-20T03:00:00Z",
                          "source":"CONTENT"},
                         {"id":"m-hidden"}]}"#));
        assert_eq!(list.len(), 2);
        assert!(list[0].visible);
        assert_eq!((list[0].key.as_str(), list[0].sources.clone()), ("HYK-M3", vec!["內容提到".to_owned()]));
        assert!(list[0].start_at.is_some());
        assert!(!list[1].visible && list[1].key == "m-hidden");
    }

    #[test]
    fn describe_recurrence_重複規則() {
        assert_eq!(
            HayakuApi::describe_recurrence(&j(
                r#"{"rule":{"freq":"WEEKLY","interval":1,"weekdays":[1,3]},"until":"2026-12-31T00:00:00Z"}"#
            ))
            .as_deref(),
            Some("每週一、三，到 2026-12-31")
        );
        assert_eq!(
            HayakuApi::describe_recurrence(&j(r#"{"freq":"DAILY","interval":2,"workdaysOnly":true}"#)).as_deref(),
            Some("每 2 天（只算工作日）")
        );
        assert_eq!(HayakuApi::describe_recurrence(&j(r#"{"freq":"MONTHLY"}"#)).as_deref(), Some("每個月"));
        assert_eq!(HayakuApi::describe_recurrence(&j(r#"{"freq":"X"}"#)), None);
    }
}
