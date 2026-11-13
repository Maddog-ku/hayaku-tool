//! 把需要的 Hayaku MCP 工具包成強型別方法。對應 C# `Mcp/HayakuApi.cs`；其他 partial 各一個子模組。
//!
//! C# 的 `CancellationToken` 不需要：Rust 的 future 被丟掉就等於取消。
//! C# 回傳 `Task` 的方法在這裡回 `Result<(), McpError>`；C# 會丟 `McpException` 的地方回 `Err`。
//! `parse_*` 是 C# 的 `internal static Parse…`（測試與其他 crate 會用到，所以公開）。

mod extras;
mod knowledge;
mod meetings;
mod projects;
mod stats;
mod task_extras;
mod templates;
mod testing;
mod views;
mod work;

use chrono::NaiveDate;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::json::{as_i32, date, iso, items, prop, str_, string};
use crate::models::{DaySchedule, TimeEntry, UserInfo};
use crate::transport::{McpError, McpTransport};
use crate::work_math::WorkMath;

/// 把需要的 Hayaku MCP 工具包成強型別方法。
pub struct HayakuApi {
    mcp: Arc<dyn McpTransport>,
    logged_todo_sample: AtomicBool,
    logged_reminder_sample: AtomicBool,
}

impl HayakuApi {
    pub fn new(mcp: Arc<dyn McpTransport>) -> Self {
        Self { mcp, logged_todo_sample: AtomicBool::new(false), logged_reminder_sample: AtomicBool::new(false) }
    }

    /// 底下的 MCP 連線（C# 的 `Mcp` 屬性）。
    pub fn mcp(&self) -> &Arc<dyn McpTransport> {
        &self.mcp
    }

    /// `Mcp.CallToolAsync(name, args)`；C# 傳 null 的地方傳 `json!({})`（C# 也是送空物件）。
    pub(crate) async fn call(&self, name: &str, args: Value) -> Result<Value, McpError> {
        self.mcp.call_tool(name, args).await
    }

    pub(crate) async fn call_raw(&self, name: &str, args: Value) -> Result<Value, McpError> {
        self.mcp.call_tool_raw(name, args).await
    }

    pub async fn who_am_i(&self) -> Result<UserInfo, McpError> {
        let r = self.call("whoami", json!({})).await?;
        let username = string(&r, "username").unwrap_or_default();
        let display_name =
            str_(&r, "displayName").filter(|d| !d.is_empty()).map_or_else(|| username.clone(), str::to_owned);
        Ok(UserInfo { username, display_name, hayaku_version: string(&r, "appVersion") })
    }

    pub async fn get_schedule(&self, day: NaiveDate) -> Result<DaySchedule, McpError> {
        let r = self.call("get_my_schedule", json!({ "from": iso(day), "to": iso(day) })).await?;
        if let Some(Value::Array(days)) = prop(&r, "days")
            && let Some(first) = days.first()
        {
            return Ok(Self::parse_day(first, day));
        }
        Ok(DaySchedule { date: day, working: Vec::new(), expected_minutes: 0, on_leave: false, holiday: None })
    }

    pub async fn list_time_entries(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<TimeEntry>, McpError> {
        let r = self.call("list_time_entries", json!({ "from": iso(from), "to": iso(to) })).await?;
        Ok(Self::parse_entries(&r, "entries"))
    }

    pub async fn running_timers(&self) -> Result<Vec<TimeEntry>, McpError> {
        let r = self.call("get_running_timers", json!({})).await?;
        Ok(Self::parse_entries(&r, "running"))
    }

    pub async fn start_timer(&self, task: &str) -> Result<(), McpError> {
        self.call("start_timer", json!({ "task": task })).await.map(drop)
    }

    /// task 為 None 時暫停所有正在計時的。
    pub async fn pause_timer(&self, task: Option<&str>) -> Result<(), McpError> {
        let args = match task {
            None => json!({}),
            Some(task) => json!({ "task": task }),
        };
        self.call("pause_timer", args).await.map(drop)
    }

    pub async fn project_keys(&self) -> Result<Vec<String>, McpError> {
        let r = self.call("list_projects", json!({ "includeArchived": true })).await?;
        Ok(items(&r, &["projects"]).iter().filter_map(|p| string(p, "key")).collect())
    }

    pub fn parse_day(d: &Value, fallback: NaiveDate) -> DaySchedule {
        let date = str_(d, "date").and_then(crate::json::parse_date_only).unwrap_or(fallback);
        let expected = match prop(d, "expected") {
            Some(e @ Value::Object(_)) => prop(e, "minutes").and_then(as_i32).unwrap_or(0),
            _ => 0,
        };
        let on_leave = matches!(prop(d, "onLeave"), Some(Value::Bool(true)));
        let holiday = match prop(d, "holiday") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(h @ Value::Object(_)) => Some(string(h, "name").unwrap_or_else(|| "假日".to_owned())),
            _ => None,
        };
        DaySchedule {
            date,
            working: WorkMath::parse_segments(str_(d, "working")),
            expected_minutes: expected,
            on_leave,
            holiday,
        }
    }

    pub fn parse_entries(r: &Value, property: &str) -> Vec<TimeEntry> {
        let mut list = Vec::new();
        let Some(Value::Array(arr)) = prop(r, property) else {
            return list;
        };
        for e in arr {
            let Some(task) = string(e, "task") else { continue };
            let duration = match prop(e, "duration") {
                Some(d @ Value::Object(_)) => prop(d, "minutes").and_then(as_i32),
                _ => None,
            };
            list.push(TimeEntry {
                id: string(e, "id").unwrap_or_else(crate::json::new_guid),
                task,
                title: string(e, "title").unwrap_or_default(),
                started_at: date(e, "startedAt"),
                ended_at: date(e, "endedAt"),
                duration_minutes: duration,
                running: matches!(prop(e, "running"), Some(Value::Bool(true))),
                stop_reason: string(e, "stopReason"),
                note: string(e, "note"),
                user: string(e, "user"),
                source: string(e, "source"),
            });
        }
        list
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! 測試共用：從 JSON 文字建 Value，以及一個記錄呼叫的假 transport。
    use super::*;
    use crate::transport::BoxFuture;
    use std::sync::Mutex;

    pub fn j(s: &str) -> Value {
        serde_json::from_str(s).expect("test json")
    }

    /// 記下每次呼叫的工具與參數，回傳預先排好的結果。
    #[derive(Default)]
    pub struct FakeTransport {
        pub calls: Mutex<Vec<(String, Value)>>,
        pub replies: Mutex<Vec<Value>>,
    }

    impl McpTransport for FakeTransport {
        fn call_tool<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, McpError>> {
            Box::pin(async move {
                self.calls.lock().expect("lock").push((name.to_owned(), arguments));
                let mut replies = self.replies.lock().expect("lock");
                Ok(if replies.is_empty() { Value::Null } else { replies.remove(0) })
            })
        }

        fn call_tool_raw<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, McpError>> {
            self.call_tool(name, arguments)
        }
    }

    /// 不靠 async 執行環境跑完一個 future（假 transport 不會真的等待）。
    pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
        use std::task::{Context, Poll, Waker};
        let mut f = std::pin::pin!(f);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    // 對應 HayakuApiParseTests（樣本取自實際 Hayaku MCP 回應）

    #[test]
    fn parse_day_讀取上班時段與應出勤() {
        let json = j(r#"{"date":"2026-10-28","weekday":"週三","holiday":null,"scheduled":"08:30–12:00、13:00–17:30",
             "working":"08:30–12:00、13:00–17:30","onLeave":false,"expected":{"minutes":480,"display":"8 小時"}}"#);
        let day = HayakuApi::parse_day(&json, NaiveDate::default());
        assert_eq!(day.date, NaiveDate::from_ymd_opt(2026, 10, 28).expect("date"));
        assert_eq!(day.working.len(), 2);
        assert_eq!(day.expected_minutes, 480);
        assert_eq!(day.holiday, None);
    }

    #[test]
    fn parse_entries_讀取計時與停止原因() {
        let json = j(r#"{"entries":[
              {"id":"a","task":"F2P-5","title":"第一階段研究","startedAt":"2026-10-28T00:28:25.871Z","endedAt":null,"duration":null,"running":true,"source":"TIMER","note":null},
              {"id":"b","task":"F2P-25","title":"第二階段研究","startedAt":"2026-10-27T00:30:00.000Z","endedAt":"2026-10-27T04:00:00.000Z","duration":{"minutes":210,"display":"3.5 小時"},"running":false,"source":"TIMER","note":null,"stopReason":"BREAK"}
            ]}"#);
        let entries = HayakuApi::parse_entries(&json, "entries");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].running);
        assert_eq!(entries[0].ended_at, None);
        assert_eq!(entries[1].duration_minutes, Some(210));
        assert_eq!(entries[1].stop_reason.as_deref(), Some("BREAK"));
    }

    // 對應 ProjectsApiTests.ParseEntries_讀執行者與來源
    #[test]
    fn parse_entries_讀執行者與來源() {
        let entries = HayakuApi::parse_entries(
            &j(
                r#"{"entries":[{"id":"x","task":"F2P-26","title":"點數系統","user":"古信汯","startedAt":"2026-10-27T05:00:00.000Z",
              "endedAt":"2026-10-27T08:43:40.790Z","duration":{"minutes":224},"running":false,"source":"TIMER","note":null}]}"#,
            ),
            "entries",
        );
        assert_eq!(
            (entries[0].user.as_deref(), entries[0].source.as_deref(), entries[0].duration_minutes),
            (Some("古信汯"), Some("TIMER"), Some(224))
        );
    }

    #[test]
    fn 呼叫工具_參數與結果() {
        let fake = Arc::new(FakeTransport::default());
        fake.replies.lock().expect("lock").push(j(r#"{"username":"harry.gu","displayName":"","appVersion":"1.5.2"}"#));
        let api = HayakuApi::new(fake.clone());
        let me = block_on(api.who_am_i()).expect("whoami");
        assert_eq!(
            me,
            UserInfo {
                username: "harry.gu".into(),
                display_name: "harry.gu".into(),
                hayaku_version: Some("1.5.2".into())
            }
        );
        block_on(api.pause_timer(None)).expect("pause");
        block_on(api.pause_timer(Some("F2P-1"))).expect("pause");
        let calls = fake.calls.lock().expect("lock");
        assert_eq!(calls[0], ("whoami".to_owned(), json!({})));
        assert_eq!(calls[1], ("pause_timer".to_owned(), json!({})));
        assert_eq!(calls[2], ("pause_timer".to_owned(), json!({ "task": "F2P-1" })));
    }
}
