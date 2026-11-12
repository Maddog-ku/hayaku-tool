//! 測試流程（測試結果、復驗、三種視圖）、讀檔案內容、已完成的任務、關聯任務、問題回報。
//! 對應 C# `Mcp/HayakuApi.Testing.cs`（型別在 `models::testing`）。

use chrono::NaiveDate;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

use super::HayakuApi;
use crate::json::*;
use crate::key_finder::KeyFinder;
use crate::models::*;
use crate::transport::McpError;

impl HayakuApi {
    // ───────────── 測試結果 ─────────────

    /// 在測試任務底下加一筆測試結果（我是回報的人）。to_fix：要修（待修復，預設）或待測（未開始）。回傳新測試結果的編號。
    #[allow(clippy::too_many_arguments)]
    pub async fn add_test_result(
        &self,
        test_task: &str,
        title: &str,
        content: Option<&str>,
        category: Option<&str>,
        severity: Option<&str>,
        assignees: Option<&[String]>,
        related: Option<&[String]>,
        to_fix: bool,
        due: Option<NaiveDate>,
    ) -> Result<Option<String>, McpError> {
        let mut args = Map::new();
        args.insert("testTask".into(), json!(test_task));
        args.insert("title".into(), json!(title));
        args.insert("start".into(), json!(if to_fix { "要修" } else { "待測" }));
        if let Some(c) = content.filter(|c| !is_blank(c)) {
            args.insert("content".into(), json!(c));
        }
        if let Some(c) = category {
            args.insert("category".into(), json!(c));
        }
        if let Some(s) = severity {
            args.insert("severity".into(), json!(s));
        }
        if let Some(a) = assignees.filter(|a| !a.is_empty()) {
            args.insert("assignees".into(), json!(a));
        }
        if let Some(r) = related.filter(|r| !r.is_empty()) {
            args.insert("related".into(), json!(r));
        }
        if let Some(d) = due {
            args.insert("plannedEnd".into(), json!(iso(d)));
        }
        let r = self.call("add_test_result", Value::Object(args)).await?;
        Ok(Self::created_key(&r, Some(test_task)))
    }

    /// 復驗：pass＝通過（已完成），否則退回（待修復，輪數 +1）；comment 變成一則留言。只有回報的人或專案管理員能用。
    pub async fn retest(
        &self,
        task: &str,
        pass: bool,
        comment: Option<&str>,
        assignees: Option<&[String]>,
        due: Option<NaiveDate>,
    ) -> Result<Option<String>, McpError> {
        let mut args = Map::new();
        args.insert("task".into(), json!(task));
        args.insert("result".into(), json!(if pass { "pass" } else { "reopen" }));
        if !pass && let Some(c) = comment.filter(|c| !is_blank(c)) {
            args.insert("comment".into(), json!(c));
        }
        if !pass && let Some(a) = assignees.filter(|a| !a.is_empty()) {
            args.insert("assignees".into(), json!(a));
        }
        if !pass && let Some(d) = due {
            args.insert("plannedEnd".into(), json!(iso(d)));
        }
        Ok(message(&self.call("retest_test_result", Value::Object(args)).await?))
    }

    /// 批次把測試結果推到下一個子狀態（例 已更新到測試機：待更新測試機 → 待復驗）。from：子狀態名稱或批次按鈕文字；
    /// tasks 為 None 是那個子狀態的全部；comment（更新說明，例版本號）變成每筆的留言。專案管理員、編輯者、測試員能用。
    pub async fn advance_test_results(
        &self,
        project: &str,
        from: &str,
        tasks: Option<&[String]>,
        comment: Option<&str>,
    ) -> Result<Option<String>, McpError> {
        let mut args = Map::new();
        args.insert("project".into(), json!(project));
        args.insert("from".into(), json!(from));
        if let Some(t) = tasks.filter(|t| !t.is_empty()) {
            args.insert("tasks".into(), json!(t));
        }
        if let Some(c) = comment.filter(|c| !is_blank(c)) {
            args.insert("comment".into(), json!(c.trim()));
        }
        Ok(message(&self.call("advance_test_results", Value::Object(args)).await?))
    }

    /// testSubStatuses（get_project）或 subStatuses（測試總覽）；順便記下子狀態屬於哪個階段。
    pub fn parse_sub_statuses(r: &Value, names: &[&str]) -> Vec<TestSubStatus> {
        let list: Vec<TestSubStatus> = items(r, names)
            .iter()
            .filter_map(|s| {
                let name = string(s, "name")?;
                let stage = string(s, "stage")?;
                Some(TestSubStatus {
                    stage,
                    name,
                    batch_label: string(s, "batchLabel"),
                    color: string(s, "color"),
                    count: number(s, &["count"]),
                    oldest_days: number(s, &["oldestDays"]),
                })
            })
            .collect();
        TaskKinds::learn_stages(&list);
        list
    }

    /// 需修復（fix）或需測試（test）的列表；project 為 None 是全部專案，mine 只看跟我有關的。
    pub async fn list_test_results(
        &self,
        view: &str,
        project: Option<&str>,
        mine: bool,
    ) -> Result<Vec<TestResultRow>, McpError> {
        let mut args = Map::new();
        args.insert("view".into(), json!(view));
        args.insert("mine".into(), json!(mine));
        if let Some(p) = project {
            args.insert("project".into(), json!(p));
        }
        Ok(Self::parse_test_rows(&self.call("list_test_results", Value::Object(args)).await?))
    }

    /// 測試總覽；period：week、month、30d、all。
    pub async fn test_overview(&self, project: Option<&str>, period: &str) -> Result<TestOverview, McpError> {
        let mut args = Map::new();
        args.insert("view".into(), json!("overview"));
        args.insert("period".into(), json!(period));
        if let Some(p) = project {
            args.insert("project".into(), json!(p));
        }
        Ok(Self::parse_test_overview(&self.call("list_test_results", Value::Object(args)).await?))
    }

    /// fix 視圖的 results；test 視圖的 retest、toTest、tests 三組（照這個順序）。
    pub fn parse_test_rows(r: &Value) -> Vec<TestResultRow> {
        let mut list: Vec<TestResultRow> = Vec::new();
        for group in ["results", "retest", "toTest", "tests", "items"] {
            let g = if group == "results" || group == "items" { "fix" } else { group };
            list.extend(items(r, &[group]).iter().filter_map(|t| Self::parse_test_row(t, g)));
        }
        let mut seen = std::collections::HashSet::new();
        list.retain(|t| seen.insert((t.key.clone(), t.group.clone())));
        list
    }

    pub fn parse_test_row(t: &Value, group: &str) -> Option<TestResultRow> {
        if !t.is_object() {
            return None;
        }
        let key = first_key(t, &["task", "key", "number"])?;
        let info = Self::read_test_info(t);
        let kind = if info.kind != TaskKind::Normal {
            info.kind
        } else if group == "tests" {
            TaskKind::Test
        } else {
            TaskKind::TestResult
        };
        let mut test_task: Option<String> = None;
        let mut test_title: Option<String> = None;
        for name in ["testTask", "parent", "test"] {
            match prop(t, name) {
                Some(Value::String(s)) => {
                    if test_task.is_none() {
                        test_task = KeyFinder::parse(Some(s));
                    }
                }
                Some(p @ Value::Object(_)) => {
                    if test_task.is_none() {
                        test_task = first_key(p, &["task", "key", "number"]);
                    }
                    if test_title.is_none() {
                        test_title = first_string(p, &["title", "name"]);
                    }
                }
                _ => {}
            }
        }
        let path = items(t, &["path"]);
        if test_task.is_none() {
            test_task = path.iter().filter_map(|x| string(x, "task")).next_back();
        }
        if test_title.is_none() {
            test_title = path.iter().filter_map(|x| string(x, "title")).next_back();
        }
        Some(TestResultRow {
            key,
            title: first_string(t, &["title", "name"]).unwrap_or_default(),
            project: project_name(t),
            status: first_string(t, &["status"]).unwrap_or_default(),
            kind,
            category: info.category,
            severity: info.severity,
            round: info.round,
            assignees: names(t, "assignees"),
            reporter: info.reporter,
            planned_end: day(t, "plannedEnd").or_else(|| day(t, "due")).or_else(|| day(t, "dueDate")),
            test_task,
            test_task_title: test_title,
            stale: bool_(t, "stale") || bool_(t, "isStale") || bool_(t, "overdue"),
            group: group.to_owned(),
            stage: info.stage,
        })
    }

    pub fn parse_test_overview(r: &Value) -> TestOverview {
        let sub_statuses = Self::parse_sub_statuses(r, &["subStatuses"]);
        let cards = obj(r, "cards");
        let open = obj(cards, "open");
        let retest = obj(cards, "retest");
        let closed = obj(cards, "closed");
        let avg = obj(cards, "avgRound");
        let by_severity = obj(r, "bySeverity");
        let severities: Vec<(String, i32)> = by_severity
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), as_i32(v).unwrap_or(0))).collect())
            .unwrap_or_default();
        let tests = items(r, &["tests"])
            .iter()
            .filter_map(|t| {
                let key = first_key(t, &["task", "key"])?;
                let title = first_string(t, &["title", "name"]).unwrap_or_default();
                // 實際回應：results { total, open, byStatus { 狀態: 筆數 } }，依狀態分組加總
                let results = obj(t, "results");
                if let Some(by_status) = obj(results, "byStatus").as_object() {
                    let mut buckets: HashMap<&str, i32> = HashMap::new();
                    for (status, n) in by_status {
                        *buckets.entry(TaskKinds::bucket(status, None, None)).or_insert(0) += as_i32(n).unwrap_or(0);
                    }
                    let b = |bucket: &str| buckets.get(bucket).copied().unwrap_or(0);
                    return Some(TestTaskProgress {
                        key,
                        title,
                        project: project_name(t),
                        total: number(results, &["total"]).unwrap_or_else(|| buckets.values().sum()),
                        passed: b("pass"),
                        fixing: b("fixing"),
                        testing: b("testing"),
                        retest: b("retest"),
                    });
                }
                let counts = if obj(t, "counts").is_object() { obj(t, "counts") } else { t };
                let n = |names: &[&str]| number(counts, names).unwrap_or(0);
                Some(TestTaskProgress {
                    key,
                    title,
                    project: project_name(t),
                    total: n(&["total", "count"]),
                    passed: n(&["passed", "pass", "done"]),
                    fixing: n(&["fixing", "fix"]) + n(&["toFix"]),
                    testing: n(&["testing", "test", "todo"]),
                    retest: n(&["retest", "toRetest"]),
                })
            })
            .collect();
        let stale = obj(r, "stale");
        let stale_items = items(if stale.is_object() { stale } else { r }, &["items"])
            .iter()
            .filter_map(|t| Self::parse_test_row(t, "stale"))
            .collect();
        let avg_round = if avg.is_object() { prop(avg, "value").and_then(as_f64) } else { None };
        TestOverview {
            open: number(open, &["total"]).unwrap_or(0),
            critical: number(open, &["critical"]).unwrap_or(0),
            major: number(open, &["major"]).unwrap_or(0),
            retest: number(retest, &["total"]).unwrap_or(0),
            oldest_retest_days: number(retest, &["oldestDays"]),
            created: number(obj(cards, "created"), &["count"]).unwrap_or(0),
            done: number(closed, &["done"]).unwrap_or(0),
            wont_fix: number(closed, &["wontfix", "wontFix"]).unwrap_or(0),
            avg_round,
            round3_plus: number(avg, &["round3Plus"]).unwrap_or(0),
            stale: number(cards, &["stale"]).or_else(|| number(stale, &["total"])).unwrap_or(0),
            by_severity: severities,
            tests,
            stale_items,
            pending_release: number(closed, &["pendingRelease"]).unwrap_or(0),
            sub_statuses: Some(sub_statuses),
        }
    }

    /// 任務的測試欄位；可能直接放在這一層（type、category、severity、round），或包在 testResult／test 物件裡。
    /// 一般任務通常沒有這些欄位。
    pub fn read_test_info(t: &Value) -> TestInfo {
        if !t.is_object() {
            return TestInfo::default();
        }
        let nested =
            ["testResult", "test", "bug"].iter().find_map(|n| prop(t, n).filter(|o| o.is_object())).unwrap_or(&NULL);
        let sources = [t, nested];
        let field = |names: &[&str]| -> Option<String> {
            for source in sources {
                if !source.is_object() {
                    continue;
                }
                for name in names {
                    let Some(v) = prop(source, name) else { continue };
                    if let Some(s) = v.as_str().filter(|s| !s.is_empty()) {
                        return Some(s.to_owned());
                    }
                    if v.is_object()
                        && let Some(named) = first_str(v, &["name", "label", "value"])
                    {
                        return Some(named.to_owned());
                    }
                }
            }
            None
        };
        let int = |names: &[&str]| sources.iter().find_map(|s| number(s, names));
        let mut kind = TaskKinds::parse(field(&["type", "taskType"]).as_deref());
        if kind == TaskKind::Normal && nested.is_object() {
            kind = TaskKind::TestResult;
        }
        let mut reporter = field(&["reporter", "reportedBy", "createdBy"]);
        if reporter.is_none() {
            reporter = ["reporter", "reportedBy", "createdBy"]
                .iter()
                .find_map(|n| prop(t, n).and_then(person))
                .map(|p| p.display_name);
        }
        TestInfo {
            kind,
            category: field(&["category", "testCategory"]),
            severity: field(&["severity"]),
            round: int(&["round", "fixRound", "rounds"]).unwrap_or(0),
            reporter,
            stage: field(&["testStage", "stage"]),
        }
    }

    // ───────────── 讀檔案內容 ─────────────

    /// 讀附件、會議檔案、白板、空間檔案、問題回報截圖的內容：圖片回圖、文件回文字（一次最多 5 萬字，offset 往下讀）。
    /// Hayaku 限制每人 10 分鐘最多讀 30 次（跟 Claude 共用），呼叫端要快取。
    pub async fn read_file(&self, id: &str, offset: i32) -> Result<FileContent, McpError> {
        let mut args = Map::new();
        args.insert("id".into(), json!(id));
        if offset > 0 {
            args.insert("offset".into(), json!(offset));
        }
        Ok(Self::parse_file_content(&self.call_raw("read_file", Value::Object(args)).await?, id))
    }

    pub fn parse_file_content(result: &Value, id: &str) -> FileContent {
        #[derive(Default)]
        struct Acc {
            image: Option<Vec<u8>>,
            mime: Option<String>,
            text: Option<String>,
            name: Option<String>,
            note: Option<String>,
            next: Option<i32>,
        }
        fn from_object(a: &mut Acc, o: &Value) {
            if !o.is_object() {
                return;
            }
            a.name = a.name.take().or_else(|| first_string(o, &["name", "fileName", "filename", "title"]));
            a.mime = a.mime.take().or_else(|| first_string(o, &["mimeType", "contentType", "mime"]));
            a.text = a.text.take().or_else(|| first_string(o, &["text", "content", "markdown"]));
            a.note = a.note.take().or_else(|| first_string(o, &["note", "message", "warning"]));
            if let Some(n @ Value::Number(_)) = prop(o, "nextOffset")
                && let Some(nv) = as_i32(n)
            {
                a.next = a.next.or(Some(nv));
            }
        }
        let mut a = Acc::default();
        if let Some(structured) = prop(result, "structuredContent") {
            from_object(&mut a, structured);
        }
        if let Some(Value::Array(content)) = prop(result, "content") {
            let mut texts: Vec<&str> = Vec::new();
            for item in content {
                match str_(item, "type") {
                    Some("image") if a.image.is_none() => {
                        if let Some(data) = str_(item, "data")
                            && let Some(bytes) = from_base64(data)
                        {
                            a.image = Some(bytes);
                            a.mime = string(item, "mimeType").or(a.mime.take());
                        }
                    }
                    Some("text") => {
                        if let Some(t) = str_(item, "text") {
                            texts.push(t);
                        }
                    }
                    _ => {}
                }
            }
            // 文字可能是 JSON（{ name, text, nextOffset }），也可能就是內容
            for t in texts {
                if t.trim_start().starts_with('{')
                    && let Ok(doc) = serde_json::from_str::<Value>(t)
                {
                    from_object(&mut a, &doc);
                    continue;
                }
                if a.image.is_some() {
                    if a.note.is_none() {
                        a.note = Some(t.to_owned()); // 圖片旁邊的說明
                    }
                } else {
                    a.text = Some(match a.text.take() {
                        None => t.to_owned(),
                        Some(prev) => format!("{prev}\n{t}"),
                    });
                }
            }
        }
        let text = if a.image.is_none() { Some(a.text.unwrap_or_default()) } else { None };
        FileContent {
            id: id.to_owned(),
            name: a.name,
            mime_type: a.mime,
            image: a.image,
            text,
            next_offset: a.next,
            note: a.note,
        }
    }

    // ───────────── 已完成的任務 ─────────────

    /// 我做完的任務，最近完成的在最上面；days：7、30、90 或 all。
    pub async fn my_done_tasks(&self, days: &str) -> Result<Vec<TaskSummary>, McpError> {
        Ok(Self::parse_task_summaries(&self.call("my_tasks", json!({ "tab": "done", "days": days })).await?))
    }

    // ───────────── 關聯任務 ─────────────

    /// 關聯任務（不擋開始、不影響日期，可以跨專案）。
    pub async fn add_related(&self, task: &str, other: &str) -> Result<Option<String>, McpError> {
        Ok(message(&self.call("add_dependency", json!({ "task": task, "kind": "related", "related": other })).await?))
    }

    pub async fn remove_related(&self, task: &str, other: &str) -> Result<Option<String>, McpError> {
        Ok(message(
            &self.call("remove_dependency", json!({ "kind": "related", "task": task, "related": other })).await?,
        ))
    }

    // ───────────── 問題回報 ─────────────

    pub async fn get_feedback(&self, number: i32) -> Result<FeedbackDetail, McpError> {
        Ok(Self::parse_feedback(&self.call("get_feedback", json!({ "number": number })).await?, number))
    }

    pub fn parse_feedback(r: &Value, number_: i32) -> FeedbackDetail {
        let f = if obj(r, "feedback").is_object() { obj(r, "feedback") } else { r };
        let mut env = Vec::new();
        let env_obj = if obj(f, "environment").is_object() { obj(f, "environment") } else { obj(f, "env") };
        if let Some(o) = env_obj.as_object() {
            let label = |name: &str| -> String {
                match name {
                    "url" => "網址",
                    "version" => "版本",
                    "browser" | "userAgent" => "瀏覽器",
                    "viewport" | "window" => "視窗",
                    "theme" | "colorScheme" => "色系",
                    "timezone" => "時區",
                    other => other,
                }
                .to_owned()
            };
            for (name, v) in o {
                if matches!(v, Value::String(_) | Value::Number(_)) {
                    let text = element_to_string(v);
                    if !text.is_empty() {
                        env.push((label(name), text));
                    }
                }
            }
        }
        let history = items(f, &["history", "events", "logs"])
            .iter()
            .map(|h| match h {
                Value::String(s) => s.clone(),
                _ => {
                    let parts = [
                        first_date(h, &["at", "createdAt"]).map(|at| local_short(&at)),
                        first_name(h, "actor").or_else(|| first_name(h, "user")),
                        first_string(h, &["text", "action", "note", "message", "status"]),
                    ];
                    parts.into_iter().flatten().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("  ")
                }
            })
            .filter(|x| !is_blank(x))
            .collect();
        let tasks = items(f, &["tasks", "createdTasks"])
            .iter()
            .filter_map(|t| match t {
                Value::String(s) => Some(s.clone()),
                _ => first_key(t, &["task", "key"]),
            })
            .collect();
        let shots = items(f, &["screenshots", "attachments", "images"])
            .iter()
            .filter_map(|s| match s {
                Value::String(s) => Some(s.clone()),
                _ => first_string(s, &["id", "attachmentId", "fileId"]),
            })
            .collect();
        FeedbackDetail {
            number: number(f, &["number"]).unwrap_or(number_),
            title: first_string(f, &["title", "summary"]).unwrap_or_else(|| format!("#{number_}")),
            status: first_string(f, &["status", "state"]).unwrap_or_default(),
            reporter: first_name(f, "reporter")
                .or_else(|| first_name(f, "createdBy"))
                .or_else(|| first_name(f, "user")),
            created_at: first_date(f, &["createdAt"]),
            body: first_string(f, &["body", "description", "content", "text"]).unwrap_or_default(),
            resolution: first_string(f, &["resolution", "handlingNote", "response", "reply"]),
            environment: env,
            history,
            created_tasks: tasks,
            screenshot_ids: shots,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 TestingTests 的 API 解析部分

    #[test]
    fn task_kinds_parse_代碼或中文都認得() {
        let cases = [
            (None, TaskKind::Normal),
            (Some("一般"), TaskKind::Normal),
            (Some("NORMAL"), TaskKind::Normal),
            (Some("測試"), TaskKind::Test),
            (Some("TEST"), TaskKind::Test),
            (Some("測試結果"), TaskKind::TestResult),
            (Some("TEST_RESULT"), TaskKind::TestResult),
            (Some("test_result"), TaskKind::TestResult),
        ];
        for (value, expected) in cases {
            assert_eq!(TaskKinds::parse(value), expected, "{value:?}");
        }
    }

    #[test]
    fn task_kinds_bucket_測試結果的狀態分組() {
        let cases = [
            ("待修復", "fixing"),
            ("修復中", "fixing"),
            ("待復驗", "retest"),
            ("未開始", "testing"),
            ("測試中", "testing"),
            ("已完成", "pass"),
            ("不處理", "closed"),
        ];
        for (status, bucket) in cases {
            assert_eq!(TaskKinds::bucket(status, None, None), bucket, "{status}");
        }
    }

    #[test]
    fn parse_task_summaries_一般任務沒有類型欄位_測試結果讀分類嚴重度輪數() {
        let list = HayakuApi::parse_task_summaries(&j(r#"
            {"tasks":[
              {"task":"F2P-129","title":"動態調整優化","project":"F2P-臉部辨識結合印表機","status":"處理中","statusCategory":"IN_PROGRESS",
               "priority":"無","plannedStart":"2026-11-02","plannedEnd":"2026-11-02","approvalState":"APPROVED","timing":true,
               "path":[{"task":"F2P-5","title":"第一階段"}],"childCount":0},
              {"task":"F2P-140","title":"登入頁跑版","project":"F2P","status":"待修復","statusCategory":"TODO","type":"測試結果",
               "category":"Bug","severity":"嚴重","round":3,"reporter":{"username":"qa","displayName":"小美"},
               "path":[{"task":"F2P-139","title":"第二階段測試"}],"childCount":0},
              {"task":"F2P-139","title":"第二階段測試","project":"F2P","status":"測試中","statusCategory":"IN_PROGRESS","type":"TEST"}
            ],"cursor":null,"timezone":"Asia/Taipei"}"#));
        assert_eq!(list[0].kind(), TaskKind::Normal);
        assert_eq!(
            (
                list[1].kind(),
                list[1].test.category.as_deref(),
                list[1].test.severity.as_deref(),
                list[1].test.round,
                list[1].test.reporter.as_deref()
            ),
            (TaskKind::TestResult, Some("Bug"), Some("嚴重"), 3, Some("小美"))
        );
        assert!(list[1].test.hot());
        assert_eq!(list[2].kind(), TaskKind::Test);
    }

    #[test]
    fn parse_test_rows_空的視圖_取自實際回應() {
        assert!(
            HayakuApi::parse_test_rows(&j(
                r#"{"view":"需修復任務","count":0,"truncated":false,"results":[],"timezone":"Asia/Taipei"}"#
            ))
            .is_empty()
        );
        assert!(
            HayakuApi::parse_test_rows(&j(
                r#"{"view":"需測試任務","retest":[],"toTest":[],"tests":[],"truncated":false,"timezone":"Asia/Taipei"}"#
            ))
            .is_empty()
        );
    }

    #[test]
    fn parse_test_rows_需測試三組照順序_測試任務從上層讀() {
        let rows = HayakuApi::parse_test_rows(&j(r#"
            {"view":"需測試任務",
             "retest":[{"task":"F2P-141","title":"按鈕沒反應","status":"待復驗","severity":"一般","round":1,
                        "assignees":[{"username":"harry.gu","displayName":"古信汯"}],"testTask":{"task":"F2P-139","title":"第二階段測試"}}],
             "toTest":[{"task":"F2P-142","title":"列印失敗","status":"未開始","parent":"F2P-139"}],
             "tests":[{"task":"F2P-139","title":"第二階段測試","status":"測試中"}]}"#));
        assert_eq!(rows.iter().map(|r| r.group.as_str()).collect::<Vec<_>>(), vec!["retest", "toTest", "tests"]);
        assert_eq!(
            (rows[0].test_task.as_deref(), rows[0].test_task_title.as_deref(), rows[0].assignees[0].as_str()),
            (Some("F2P-139"), Some("第二階段測試"), "古信汯")
        );
        assert_eq!(rows[1].test_task.as_deref(), Some("F2P-139"));
        assert_eq!(rows[1].kind, TaskKind::TestResult);
        assert_eq!(rows[2].kind, TaskKind::Test);
    }

    #[test]
    fn parse_test_overview_讀卡片數字_欄位同實際回應() {
        let o = HayakuApi::parse_test_overview(&j(r#"
            {"view":"測試總覽","from":"2026-11-02","to":"2026-11-02",
             "cards":{"open":{"total":5,"critical":1,"major":2},"retest":{"total":2,"oldestDays":3},"created":{"count":7,"previous":null},
                      "closed":{"done":4,"wontfix":1},"avgRound":{"value":1.5,"round3Plus":1},"stale":2},
             "bySeverity":{"致命":1,"嚴重":2,"一般":2,"輕微":0},"byCategory":{"待判斷":{},"Bug":{},"需求變更":{}},
             "tests":[{"task":"F2P-139","title":"第二階段測試","counts":{"total":6,"passed":3,"fixing":2,"testing":1}}],
             "stale":{"total":1,"items":[{"task":"F2P-150","title":"很久沒人修","status":"待修復"}]},"projects":[],"timezone":"Asia/Taipei"}"#));
        assert_eq!(
            (
                o.open,
                o.critical,
                o.major,
                o.retest,
                o.oldest_retest_days,
                o.created,
                o.done,
                o.wont_fix,
                o.avg_round,
                o.round3_plus,
                o.stale
            ),
            (5, 1, 2, 2, Some(3), 7, 4, 1, Some(1.5), 1, 2)
        );
        assert_eq!(o.by_severity[0], ("致命".to_owned(), 1));
        assert_eq!((o.tests[0].total, o.tests[0].passed, o.tests[0].fixing, o.tests[0].testing), (6, 3, 2, 1));
        assert_eq!(o.stale_items.len(), 1);
        assert_eq!(o.stale_items[0].key, "F2P-150");
    }

    #[test]
    fn parse_test_overview_測試任務的筆數讀results的by_status_取自實際回應() {
        let o = HayakuApi::parse_test_overview(&j(r#"
            {"view":"測試總覽","cards":{},
             "tests":[{"task":"F2P-149","title":"bug修復","status":"測試中","parent":"F2P-50 第四階段","assignees":[],"plannedEnd":null,
                       "results":{"total":5,"open":3,"byStatus":{"已完成":1,"待修復":1,"修復中":1,"待復驗":1,"不處理":1}},
                       "avgRound":1,"oldestOpenDays":null,"stale":0},
                      {"task":"F2P-146","title":"bug修復","status":"未開始","results":{"total":0,"open":0,"byStatus":{}}}],
             "stale":{"total":0,"items":[]},"timezone":"Asia/Taipei"}"#));
        assert_eq!(
            (o.tests[0].total, o.tests[0].passed, o.tests[0].fixing, o.tests[0].testing, o.tests[0].retest),
            (5, 1, 2, 0, 1)
        );
        assert_eq!((o.tests[1].total, o.tests[1].passed), (0, 0));
    }

    #[test]
    fn parse_test_overview_沒有資料時平均輪數是null_取自實際回應() {
        let o = HayakuApi::parse_test_overview(&j(r#"
            {"view":"測試總覽","from":"2026-11-02","to":"2026-11-02",
             "cards":{"open":{"total":0,"critical":0,"major":0},"retest":{"total":0,"oldestDays":null},"created":{"count":0,"previous":null},
                      "closed":{"done":0,"wontfix":0},"avgRound":{"value":null,"round3Plus":0},"stale":0},
             "bySeverity":{"致命":0,"嚴重":0,"一般":0,"輕微":0},"byCategory":{"待判斷":{},"Bug":{},"需求變更":{}},
             "tests":[],"stale":{"total":0,"items":[]},"projects":[],"timezone":"Asia/Taipei"}"#));
        assert_eq!(o.avg_round, None);
        assert_eq!(o.oldest_retest_days, None);
        assert_eq!(o.by_severity.len(), 4);
        assert!(o.tests.is_empty());
    }

    #[test]
    fn parse_file_content_圖片回圖_文件回文字與下一段() {
        let image = HayakuApi::parse_file_content(
            &j(
                r#"{"content":[{"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"},{"type":"text","text":"截圖.png（縮到 1568 px）"}]}"#,
            ),
            "a1",
        );
        assert!(image.image.is_some());
        assert_eq!(image.mime_type.as_deref(), Some("image/png"));
        assert_eq!(image.text, None);
        assert_eq!(image.note.as_deref(), Some("截圖.png（縮到 1568 px）"));

        let doc = HayakuApi::parse_file_content(
            &j(
                r#"{"content":[{"type":"text","text":"{\"name\":\"規格.pdf\",\"text\":\"第 1 頁\\n內容\",\"nextOffset\":50000}"}]}"#,
            ),
            "a2",
        );
        assert_eq!(
            (doc.name.as_deref(), doc.text.as_deref(), doc.next_offset),
            (Some("規格.pdf"), Some("第 1 頁\n內容"), Some(50000))
        );

        let last = HayakuApi::parse_file_content(
            &j(r#"{"structuredContent":{"text":"最後一段","nextOffset":null},"content":[]}"#),
            "a4",
        );
        assert_eq!((last.text.as_deref(), last.next_offset), (Some("最後一段"), None));

        let plain = HayakuApi::parse_file_content(&j(r#"{"content":[{"type":"text","text":"a,b\n1,2"}]}"#), "a3");
        assert_eq!(plain.text.as_deref(), Some("a,b\n1,2"));
        assert_eq!(plain.next_offset, None);
    }

    // 對應 Hayaku14Tests 的測試子狀態部分
    #[test]
    fn 專案的測試子狀態_記下所屬階段_分組照階段() {
        let subs = HayakuApi::parse_sub_statuses(
            &j(r##"
            {"testSubStatuses":[
              {"stage":"修復中","statusId":"a","name":"修復中","color":"#FA8C16","batchLabel":null},
              {"stage":"修復中","statusId":"b","name":"待更新測試機","color":"#9C7A3C","batchLabel":"更新到測試機"},
              {"stage":"待復驗","statusId":"c","name":"待復驗","color":"#722ED1","batchLabel":null},
              {"stage":"待復驗","statusId":"d","name":"待更新正式機","color":"#52C41A","batchLabel":"更新正式機"}]}"##),
            &["testSubStatuses"],
        );
        assert_eq!(
            subs.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["修復中", "待更新測試機", "待復驗", "待更新正式機"]
        );
        assert_eq!(subs[1].batch_text().as_deref(), Some("已更新到測試機"));
        assert_eq!(subs[0].batch_text(), None);

        // 待更新正式機 是 IN_PROGRESS 類別，以前會被當成修復中；現在照階段算待復驗
        assert_eq!(TaskKinds::bucket("待更新正式機", Some("IN_PROGRESS"), None), "retest");
        assert_eq!(TaskKinds::bucket("待更新測試機", Some("IN_PROGRESS"), None), "fixing");
        // 回應有 testStage 時以它為準
        assert_eq!(TaskKinds::bucket("待上正式機", Some("IN_PROGRESS"), Some("待復驗")), "retest");
        assert_eq!(TaskKinds::stage_of("待更新正式機", None), "待復驗");
    }

    #[test]
    fn 測試結果列_讀_test_stage() {
        let rows = HayakuApi::parse_test_rows(&j(r#"
            {"view":"需修復任務","results":[{"task":"TEST-148","title":"按鈕被擋住","status":"待上線","testStage":"待復驗","type":"測試結果","round":2}]}"#));
        assert_eq!(rows[0].stage.as_deref(), Some("待復驗"));
        assert_eq!(rows[0].stage_name(), "待復驗");
    }

    #[test]
    fn 測試總覽_子狀態數量與待上線() {
        let o = HayakuApi::parse_test_overview(&j(r#"
            {"cards":{"open":{"total":11},"retest":{"total":0},"created":{"count":97},"closed":{"done":86,"wontfix":0,"pendingRelease":3}},
             "subStatuses":[{"stage":"修復中","name":"待更新測試機","count":2,"oldestDays":1,"batchLabel":"更新到測試機"},
                            {"stage":"待復驗","name":"待更新正式機","count":0,"oldestDays":null,"batchLabel":"更新正式機"}]}"#));
        assert_eq!(o.pending_release, 3);
        let subs = o.sub_statuses.expect("sub statuses");
        assert_eq!(subs.len(), 2);
        assert_eq!((subs[0].count.unwrap_or(0), subs[0].oldest_days.unwrap_or(0)), (2, 1));
    }

    // 對應 Hayaku142Tests
    fn with_built_in() -> Vec<TestSubStatus> {
        vec![
            TestSubStatus::new("修復中", "修復中", None, None),
            TestSubStatus::new("修復中", "待更新測試機", Some("更新到測試機"), None),
            TestSubStatus::new("待復驗", "待復驗", None, None),
            TestSubStatus::new("待復驗", "待上正式機", Some("更新到正式機"), None),
        ]
    }

    fn custom_only() -> Vec<TestSubStatus> {
        with_built_in().into_iter().filter(|s| s.name != s.stage).collect()
    }

    #[test]
    fn 修好了_內建修復中的下一個是第一個自訂子狀態() {
        for subs in [with_built_in(), custom_only()] {
            assert_eq!(
                TaskKinds::next_in_stage(&subs, "修復中", "修復中").map(|s| s.name.as_str()),
                Some("待更新測試機")
            );
            assert_eq!(TaskKinds::next_in_stage(&subs, "修復中", "待更新測試機"), None); // 最後一個：送復驗
        }
    }

    #[test]
    fn 通過_內建待復驗的下一個是第一個自訂子狀態() {
        for subs in [with_built_in(), custom_only()] {
            assert_eq!(
                TaskKinds::next_in_stage(&subs, "待復驗", "待復驗").map(|s| s.name.as_str()),
                Some("待上正式機")
            );
            assert_eq!(TaskKinds::next_in_stage(&subs, "待復驗", "待上正式機"), None);
        }
    }

    #[test]
    fn 沒有子狀態或不在這一段_沒有下一步() {
        assert_eq!(TaskKinds::next_in_stage(&[], "修復中", "修復中"), None);
        assert_eq!(TaskKinds::next_in_stage(&with_built_in(), "修復中", "待上正式機"), None);
        assert_eq!(TaskKinds::next_in_stage(&with_built_in(), "修復中", "待修復"), None);
    }

    #[test]
    fn parse_feedback_寬鬆讀取() {
        let f = HayakuApi::parse_feedback(
            &j(r#"{"feedback":{"title":"匯出失敗","status":"處理中","reporter":{"username":"a","displayName":"小明"},
                 "environment":{"url":"https://h/x","viewport":"1920x1080","empty":""},
                 "history":["建立",{"actor":"小華","text":"已確認"}],"tasks":["F2P-1",{"key":"F2P-2"}],
                 "screenshots":[{"attachmentId":"s1"},"s2"]}}"#),
            128,
        );
        assert_eq!((f.number, f.title.as_str(), f.reporter.as_deref()), (128, "匯出失敗", Some("小明")));
        assert_eq!(
            f.environment,
            vec![("網址".to_owned(), "https://h/x".to_owned()), ("視窗".to_owned(), "1920x1080".to_owned())]
        );
        assert_eq!(f.history, vec!["建立", "小華  已確認"]);
        assert_eq!(f.created_tasks, vec!["F2P-1", "F2P-2"]);
        assert_eq!(f.screenshot_ids, vec!["s1", "s2"]);
    }
}
