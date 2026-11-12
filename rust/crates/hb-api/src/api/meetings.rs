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
        })
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
