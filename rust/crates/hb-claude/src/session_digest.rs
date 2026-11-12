//! 對應 C# `Claude/SessionDigest.cs`。
//!
//! 把一天的 Claude Code 對話與 git commit 整理成工作日誌草稿，也能替還沒寫備註的計時段產生備註。
//! 只讀「專案」頁設定過的資料夾；任務判斷跟自動計時一樣：手動指定 → 對話裡的編號 → 分支；都沒有就歸到「其他」。

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use hb_api::{KeyFinder, TimeEntry, WorkLogEntry, WorkMath};
use hb_backend::config::IndexMap;
use regex::Regex;
use serde_json::Value;

use crate::claude_monitor::{ClaudeMonitor, RepoActivity, is_under, normalize};
use crate::util::{
    eq_ic, flag, fold, jsonl_files, left_utf16, max_by_first, str_prop, subdirs, utf16_len, zh_month_day,
};

const SUMMARY_LENGTH: usize = 60;
const NOTE_LENGTH: usize = 200;

/// 沒有內容的回覆（繼續、好、yes…），不寫進日誌。
const NOISE: [&str; 26] = [
    "continue",
    "go",
    "go on",
    "ok",
    "okay",
    "yes",
    "y",
    "no",
    "n",
    "sure",
    "thanks",
    "thank you",
    "繼續",
    "好",
    "好的",
    "對",
    "是",
    "可以",
    "沒問題",
    "謝謝",
    "嗯",
    "收到",
    "開始",
    "同意",
    "確定",
    "執行",
];

/// 貼上的圖片標記（[Image #3]）。
static IMAGE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[Image #\d+\]").expect("固定的 regex"));

/// 只是在下指令的短句（commit 並 push、直接做到完成…），沒有工作內容。
static INSTRUCTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(?i)commit|push|[直值]到完成|做完|繼續").expect("固定的 regex"));

/// 某天在 Claude Code 送出的一句指令（已整理成一行摘要）。
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudePrompt {
    pub at: NaiveDateTime,
    pub folder: String,
    pub task: Option<String>,
    pub summary: String,
}

impl ClaudePrompt {
    pub fn new(at: NaiveDateTime, folder: impl Into<String>, task: Option<&str>, summary: impl Into<String>) -> Self {
        Self { at, folder: folder.into(), task: task.map(str::to_owned), summary: summary.into() }
    }
}

/// 某天在 repo 裡的一個 commit（只算自己的）。
#[derive(Debug, Clone, PartialEq)]
pub struct GitCommit {
    pub at: NaiveDateTime,
    pub folder: String,
    pub hash: String,
    pub subject: String,
    pub task: Option<String>,
}

impl GitCommit {
    pub fn new(
        at: NaiveDateTime,
        folder: impl Into<String>,
        hash: impl Into<String>,
        subject: impl Into<String>,
        task: Option<&str>,
    ) -> Self {
        Self { at, folder: folder.into(), hash: hash.into(), subject: subject.into(), task: task.map(str::to_owned) }
    }
}

/// `StreamReader.ReadLine` 的切行：\r\n、\n、\r 都算換行；開頭的 BOM 拿掉。
fn read_lines(bytes: &[u8]) -> Vec<String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                lines.push(std::mem::take(&mut current));
            }
            '\n' => lines.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// 讀某天所有在監看資料夾裡的 session，整理出每句指令。
pub fn collect(
    day: NaiveDate,
    units: &[RepoActivity],
    session_tasks: &IndexMap<String, String>,
    is_known: &dyn Fn(&str) -> bool,
) -> Vec<ClaudePrompt> {
    let mut result = Vec::new();
    let root = ClaudeMonitor::projects_root();
    let enabled: Vec<RepoActivity> = units.iter().filter(|u| u.enabled).cloned().collect();
    if !root.is_dir() || enabled.is_empty() {
        return result;
    }
    let day_start = day.and_time(NaiveTime::MIN);
    // 只看各專案資料夾第一層的 session 檔（子代理的紀錄放在更深的資料夾，不是自己送出的指令）
    let dirs = match subdirs(&root) {
        Ok(d) => d,
        Err(err) => {
            log::error!("列出 {} 失敗: {err}", root.display());
            return result;
        }
    };
    for dir in dirs {
        let files = match jsonl_files(Path::new(&dir)) {
            Ok(f) => f,
            Err(err) => {
                log::error!("列出 {dir} 失敗: {err}");
                continue;
            }
        };
        for file in files {
            if file.last_write < day_start || file.creation >= day_start + TimeDelta::days(1) {
                continue;
            }
            match std::fs::read(&file.full_name) {
                Ok(bytes) => {
                    let lines = read_lines(&bytes);
                    let session = file.stem();
                    result.extend(parse(
                        lines.iter().map(String::as_str),
                        day,
                        &enabled,
                        session_tasks.get(&session).map(String::as_str),
                        is_known,
                    ));
                }
                Err(err) => log::error!("讀取 Claude session {} 失敗: {err}", file.name),
            }
        }
    }
    result.sort_by_key(|p| p.at);
    result
}

/// 解析一個 session 檔的每一行，取出當天使用者送出的指令。
pub fn parse<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    day: NaiveDate,
    units: &[RepoActivity],
    picked_task: Option<&str>,
    is_known: &dyn Fn(&str) -> bool,
) -> Vec<ClaudePrompt> {
    let mut list = Vec::new();
    let (mut cwd, mut branch, mut prompt_key): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    for line in lines {
        if utf16_len(line) < 2 {
            continue;
        }
        let Ok(r) = serde_json::from_str::<Value>(line) else { continue };
        if !r.is_object() {
            continue;
        }
        if let Some(c) = str_prop(&r, "cwd").filter(|c| !c.is_empty()) {
            cwd = Some(c.to_owned());
        }
        if let Some(b) = str_prop(&r, "gitBranch").filter(|b| !b.is_empty()) {
            branch = Some(b.to_owned());
        }
        if str_prop(&r, "type") != Some("user") || flag(&r, "isMeta") || flag(&r, "isSidechain") {
            continue;
        }
        let Some(content) = r.get("message").filter(|m| m.is_object()).and_then(|m| m.get("content")) else {
            continue;
        };
        let text = prompt_text(content);
        if let Some(key) = KeyFinder::find_in_prompt(text.as_deref(), is_known) {
            prompt_key = Some(key); // 跟自動計時一樣：提到的編號延續到之後的指令
        }
        let Some(at) = str_prop(&r, "timestamp")
            .and_then(hb_api::json::parse_date_time_offset)
            .map(|at| at.with_timezone(&Local).naive_local())
        else {
            continue;
        };
        if at.date() != day {
            continue;
        }
        let Some(cwd) = cwd.as_deref() else { continue };
        let Some(summary) = summarize(text.as_deref()) else { continue };
        let Ok(folder) = normalize(cwd) else { continue };
        let Some(unit) = max_by_first(units.iter().filter(|u| is_under(&folder, &u.path)), |u| u.path.len()) else {
            continue;
        };
        let task = picked_task
            .map(str::to_owned)
            .or_else(|| prompt_key.clone())
            .or_else(|| KeyFinder::find_in_branch(branch.as_deref(), is_known));
        list.push(ClaudePrompt { at, folder: unit.path.clone(), task, summary });
    }
    list
}

/// 一句指令 → 一行摘要（第一行有內容的文字，最多 60 字）；指令、系統訊息、「繼續」這類回覆回 None。
pub fn summarize(prompt: Option<&str>) -> Option<String> {
    let prompt = prompt?;
    if prompt.trim().is_empty() {
        return None;
    }
    let text = prompt.trim();
    if text.starts_with('<')
        || text.starts_with('/')
        || text.starts_with("[Request interrupted")
        || text.starts_with("Caveat:")
    {
        return None;
    }
    let first = text
        .split('\n')
        .map(|l| IMAGE_TAG.replace_all(l, "").trim().to_owned())
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let first = first.trim_end_matches(['。', '.', '，', ',', '？', '?', '！', '!', ' ']);
    let len = utf16_len(first);
    if len < 2 || NOISE.iter().any(|n| eq_ic(n, first)) || len <= 20 && INSTRUCTION.is_match(first) {
        return None;
    }
    Some(if len > SUMMARY_LENGTH { format!("{}…", left_utf16(first, SUMMARY_LENGTH)) } else { first.to_owned() })
}

/// 拿掉貼上的圖片標記（[Image #3]）。
pub fn strip_image_tags(text: &str) -> String {
    IMAGE_TAG.replace_all(text, "").into_owned()
}

/// 訊息內容：字串，或陣列裡各段 text（每段後面加換行，C# `AppendLine` 是 CRLF）。
fn prompt_text(content: &Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        return Some(s.to_owned());
    }
    let parts = content.as_array()?;
    let mut sb = String::new();
    for part in parts {
        if str_prop(part, "type") == Some("text")
            && let Some(t) = str_prop(part, "text")
        {
            sb.push_str(t);
            sb.push_str("\r\n");
        }
    }
    Some(sb)
}

// ───────────── git ─────────────

/// 這些資料夾當天自己的 commit（用各 repo 的 user.email 篩作者）；不是 git repo 或沒裝 git 就略過。
/// git 在 hb-backend 的 tokio 執行環境跑，這個 future 可以在任何執行器 await。
pub async fn commits(
    day: NaiveDate,
    folders: impl IntoIterator<Item = String>,
    prompts: &[ClaudePrompt],
    is_known: &dyn Fn(&str) -> bool,
) -> Vec<GitCommit> {
    let mut list = Vec::new();
    let start = day.and_time(NaiveTime::MIN);
    let since = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let until = (start + TimeDelta::days(1)).format("%Y-%m-%d %H:%M:%S").to_string();
    let mut distinct: Vec<String> = Vec::new();
    for folder in folders {
        if !distinct.iter().any(|d| eq_ic(d, &folder)) {
            distinct.push(folder);
        }
    }
    for folder in distinct {
        if !Path::new(&folder).join(".git").exists() {
            continue;
        }
        let email = git(&folder, &["config", "user.email"]).await.map(|e| e.trim().to_owned());
        let Some(email) = email.filter(|e| !e.is_empty()) else { continue };
        let output = git(
            &folder,
            &[
                "log",
                "--all",
                "--no-merges",
                &format!("--since={since}"),
                &format!("--until={until}"),
                &format!("--author={email}"),
                "--format=%h%x09%aI%x09%s",
            ],
        )
        .await;
        let Some(output) = output else { continue };
        // 沒寫編號的 commit 歸給這個資料夾當天最常出現的任務
        let mut groups: IndexMap<String, (String, usize)> = IndexMap::new();
        for p in prompts.iter().filter(|p| eq_ic(&p.folder, &folder)) {
            if let Some(task) = &p.task {
                groups.entry(fold(task)).or_insert_with(|| (task.clone(), 0)).1 += 1;
            }
        }
        let folder_task = max_by_first(groups.into_values(), |(_, count)| *count).map(|(task, _)| task);
        for line in output.split('\n').filter(|l| !l.is_empty()) {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() < 3 {
                continue;
            }
            let Some(at) = hb_api::json::parse_date_time_offset(parts[1]) else { continue };
            let subject = parts[2].trim().to_owned();
            let task = KeyFinder::find_in_branch(Some(&subject), is_known).or_else(|| folder_task.clone());
            list.push(GitCommit {
                at: at.with_timezone(&Local).naive_local(),
                folder: folder.clone(),
                hash: parts[0].to_owned(),
                subject,
                task,
            });
        }
    }
    list.sort_by_key(|c| c.at);
    list
}

/// 跑 git（視窗隱藏、最多等 10 秒）；結束碼不是 0 回 None。
async fn git(folder: &str, args: &[&str]) -> Option<String> {
    let folder = folder.to_owned();
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    hb_backend::rt::bg(async move {
        let result: Result<Option<String>, String> = async {
            let mut cmd = tokio::process::Command::new("git");
            cmd.arg("-C").arg(&folder).args(&args);
            cmd.stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .creation_flags(crate::CREATE_NO_WINDOW);
            let child = cmd.spawn().map_err(|e| e.to_string())?;
            let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
                .await
                .map_err(|_| "逾時".to_owned())?
                .map_err(|e| e.to_string())?;
            Ok(output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned()))
        }
        .await;
        match result {
            Ok(v) => v,
            Err(err) => {
                log::error!("git {}（{folder}）失敗: {err}", args.join(" "));
                None
            }
        }
    })
    .await
}

// ───────────── 輸出 ─────────────

/// 整理成 Markdown 草稿：依任務分組（照工時多到少），列出做了什麼與 commit。
pub fn markdown(
    day: NaiveDate,
    prompts: &[ClaudePrompt],
    commits: &[GitCommit],
    log_entries: &[WorkLogEntry],
    title_of: &dyn Fn(&str) -> Option<String>,
) -> String {
    // C# ToDictionary（不分大小寫）
    let mut minutes: IndexMap<String, i32> = IndexMap::new();
    for e in log_entries {
        minutes.entry(fold(&e.task)).or_insert(e.minutes);
    }
    let minutes_of = |t: &str| minutes.get(&fold(t)).copied().unwrap_or(0);
    const NONE: &str = "";
    let mut groups: Vec<String> = Vec::new();
    let all = prompts
        .iter()
        .map(|p| p.task.as_deref().unwrap_or(NONE))
        .chain(commits.iter().map(|c| c.task.as_deref().unwrap_or(NONE)))
        .chain(log_entries.iter().map(|e| e.task.as_str()));
    for t in all {
        if !groups.iter().any(|g| eq_ic(g, t)) {
            groups.push(t.to_owned());
        }
    }
    groups.sort_by(|a, b| (a == NONE).cmp(&(b == NONE)).then_with(|| minutes_of(b).cmp(&minutes_of(a))));

    const NL: &str = "\r\n";
    let mut sb = String::new();
    sb.push_str(&format!("# {} 工作日誌{NL}", zh_month_day(day)));
    for task in &groups {
        let mut items: Vec<&str> = Vec::new();
        for p in prompts.iter().filter(|p| eq_ic(p.task.as_deref().unwrap_or(NONE), task)) {
            if !items.contains(&p.summary.as_str()) {
                items.push(&p.summary);
            }
        }
        let task_commits: Vec<&GitCommit> =
            commits.iter().filter(|c| eq_ic(c.task.as_deref().unwrap_or(NONE), task)).collect();
        sb.push_str(NL);
        if task == NONE {
            sb.push_str(&format!("## 其他（沒對應到任務）{NL}"));
        } else {
            let head = format!("## {task} {}", title_of(task).unwrap_or_default());
            let m = minutes_of(task);
            let hours = if m > 0 { format!("（{}）", WorkMath::hours_minutes(f64::from(m))) } else { String::new() };
            sb.push_str(&format!("{}{hours}{NL}", head.trim_end()));
        }
        for item in &items {
            sb.push_str(&format!("- {item}{NL}"));
        }
        for c in &task_commits {
            sb.push_str(&format!("- commit {}：{}{NL}", c.hash, c.subject));
        }
        if items.is_empty() && task_commits.is_empty() {
            sb.push_str(&format!("- （沒有 Claude 對話紀錄）{NL}"));
        }
    }
    format!("{}{NL}", sb.trim_end())
}

/// 還沒有備註的計時段 → 建議的備註：同任務、在這段時間內送出的指令與 commit（最多 200 字）。
/// 找不到對應內容的計時段不會出現在結果裡。
pub fn notes_for(
    entries: &[TimeEntry],
    prompts: &[ClaudePrompt],
    commits: &[GitCommit],
    now: NaiveDateTime,
) -> IndexMap<String, String> {
    let mut notes = IndexMap::new();
    for e in entries {
        if e.note.as_deref().is_some_and(|n| !n.trim().is_empty()) {
            continue;
        }
        let Some(started) = e.started_at else { continue };
        let start = started.with_timezone(&Local).naive_local();
        let end =
            if e.running { now } else { e.ended_at.map(|t| t.with_timezone(&Local).naive_local()).unwrap_or(start) };
        let inside =
            |task: Option<&str>, at: NaiveDateTime| task.is_some_and(|t| eq_ic(t, &e.task)) && at >= start && at <= end;
        let mut parts: Vec<&str> = Vec::new();
        let candidates = prompts
            .iter()
            .filter(|p| inside(p.task.as_deref(), p.at))
            .map(|p| p.summary.as_str())
            .chain(commits.iter().filter(|c| inside(c.task.as_deref(), c.at)).map(|c| c.subject.as_str()));
        for part in candidates {
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
        if parts.is_empty() {
            continue;
        }
        let note = parts.join("；");
        let note =
            if utf16_len(&note) > NOTE_LENGTH { format!("{}…", left_utf16(&note, NOTE_LENGTH - 1)) } else { note };
        notes.insert(e.id.clone(), note);
    }
    notes
}
