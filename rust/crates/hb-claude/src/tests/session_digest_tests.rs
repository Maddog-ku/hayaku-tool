//! 移植自 SessionDigestTests.cs。

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone};
use hb_api::{DateTimeOffset, TimeEntry, WorkLogEntry};
use serde_json::json;

use crate::RepoActivity;
use crate::session_digest::{self, ClaudePrompt, GitCommit};

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 28).unwrap()
}

fn at(h: u32, m: u32) -> NaiveDateTime {
    day().and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap())
}

fn offset(t: NaiveDateTime) -> DateTimeOffset {
    Local.from_local_datetime(&t).single().unwrap().fixed_offset()
}

fn line(
    ty: &str,
    text: &str,
    at: NaiveDateTime,
    cwd: &str,
    branch: Option<&str>,
    meta: bool,
    sidechain: bool,
) -> String {
    // C# ToUniversalTime().ToString("O")：小數 7 位
    let utc = offset(at).with_timezone(&chrono::Utc);
    let ts = format!("{}.{:07}+00:00", utc.format("%Y-%m-%dT%H:%M:%S"), chrono::Timelike::nanosecond(&utc) / 100);
    json!({
        "type": ty,
        "cwd": cwd,
        "gitBranch": branch,
        "isMeta": meta,
        "isSidechain": sidechain,
        "timestamp": ts,
        "message": { "role": ty, "content": text },
    })
    .to_string()
}

fn user(text: &str, at: NaiveDateTime, cwd: &str) -> String {
    line("user", text, at, cwd, None, false, false)
}

fn unit(path: &str) -> RepoActivity {
    RepoActivity::new(path)
}

fn all(_: &str) -> bool {
    true
}

#[test]
fn summarize_取第一行並略過沒內容的回覆() {
    let cases: [(&str, Option<&str>); 11] = [
        ("修正列印按鈕位置。\n細節在下面", Some("修正列印按鈕位置")),
        ("  continue ", None),
        ("繼續", None),
        ("/clear", None),
        ("<command-name>/model</command-name>", None),
        ("[Request interrupted by user]", None),
        ("commit 並Push", None),
        ("直接開始實作值到完成", None),
        ("[Image #8]", None),
        ("[Image #9] 不要有全部歸零按鈕", Some("不要有全部歸零按鈕")),
        (
            "版本號用2.1.28，傳真接收放在部門那邊，然後 commit 並 push",
            Some("版本號用2.1.28，傳真接收放在部門那邊，然後 commit 並 push"),
        ),
    ];
    for (prompt, expected) in cases {
        assert_eq!(expected, session_digest::summarize(Some(prompt)).as_deref(), "{prompt}");
    }
}

#[test]
fn summarize_太長會截斷() {
    assert_eq!(61, session_digest::summarize(Some(&"字".repeat(100))).unwrap().chars().count());
}

#[test]
fn parse_依對話編號與分支歸屬且只取當天() {
    let t = at(10, 0);
    let app = r"D:\repo\app";
    let lines = [
        user("昨天的事", t - TimeDelta::days(1), app),
        line("user", "先看一下登入流程", t, app, Some("feature/F2P-12-login"), false, false),
        line("assistant", "好的", t + TimeDelta::minutes(1), app, None, false, false),
        user("F2P-97 修正列印按鈕", t + TimeDelta::minutes(5), app),
        user("再調整間距", t + TimeDelta::minutes(9), app),
        line("user", "系統訊息", t + TimeDelta::minutes(10), app, None, true, false),
        line("user", "子代理的指令", t + TimeDelta::minutes(11), app, None, false, true),
        user("別的資料夾", t + TimeDelta::minutes(12), r"E:\other"),
    ];
    let prompts =
        session_digest::parse(lines.iter().map(String::as_str), day(), &[unit(r"D:\repo"), unit(app)], None, &all);
    assert_eq!(
        vec!["先看一下登入流程", "F2P-97 修正列印按鈕", "再調整間距"],
        prompts.iter().map(|p| p.summary.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(
        vec![Some("F2P-12"), Some("F2P-97"), Some("F2P-97")],
        prompts.iter().map(|p| p.task.as_deref()).collect::<Vec<_>>()
    );
    assert!(prompts.iter().all(|p| p.folder == app));
}

#[test]
fn parse_手動指定的任務優先() {
    let lines = [user("F2P-97 做這個", at(9, 0), r"D:\repo")];
    let prompts =
        session_digest::parse(lines.iter().map(String::as_str), day(), &[unit(r"D:\repo")], Some("EG-7"), &all);
    assert_eq!(1, prompts.len());
    assert_eq!(Some("EG-7"), prompts[0].task.as_deref());
}

fn hours(h: f64) -> NaiveDateTime {
    at(0, 0) + TimeDelta::milliseconds((h * 3_600_000.0) as i64)
}

#[test]
fn notes_for_只填沒有備註且時間對得上的計時段() {
    let entry = |id: &str, task: &str, from: f64, to: f64, note: Option<&str>| {
        let mut e =
            TimeEntry::new(id, task, task, Some(offset(hours(from))), Some(offset(hours(to))), None, false, None);
        e.note = note.map(str::to_owned);
        e
    };
    let entries = [
        entry("a", "F2P-97", 9.0, 11.0, None),
        entry("b", "F2P-97", 13.0, 14.0, Some("已經寫了")),
        entry("c", "F2P-12", 9.0, 11.0, None),
        entry("d", "F2P-97", 15.0, 16.0, None),
    ];
    let prompts = [
        ClaudePrompt::new(hours(9.5), r"D:\repo", Some("F2P-97"), "修正列印"),
        ClaudePrompt::new(hours(10.0), r"D:\repo", Some("F2P-97"), "修正列印"),
        ClaudePrompt::new(hours(13.5), r"D:\repo", Some("F2P-97"), "不會寫進已有備註的"),
    ];
    let commits = [GitCommit::new(hours(10.5), r"D:\repo", "abc123", "fix: 列印按鈕", Some("F2P-97"))];
    let notes = session_digest::notes_for(&entries, &prompts, &commits, hours(20.0));
    assert_eq!(Some("修正列印；fix: 列印按鈕"), notes.get("a").map(String::as_str));
    assert_eq!(1, notes.len());
}

#[test]
fn markdown_依任務分組並附上時數與commit() {
    let prompts = [
        ClaudePrompt::new(hours(9.0), r"D:\repo", Some("F2P-97"), "修正列印"),
        ClaudePrompt::new(hours(10.0), r"D:\repo", None, "看一下環境"),
    ];
    let commits = [GitCommit::new(hours(11.0), r"D:\repo", "abc123", "fix: 列印按鈕", Some("F2P-97"))];
    let log = [WorkLogEntry { task: "F2P-97".into(), minutes: 130, ranges: vec![] }];
    let md = session_digest::markdown(day(), &prompts, &commits, &log, &|k: &str| {
        (k == "F2P-97").then(|| "面板列印".to_owned())
    });
    assert!(
        md.contains(
            &"## F2P-97 面板列印（2 小時 10 分）\n- 修正列印\n- commit abc123：fix: 列印按鈕".replace('\n', "\r\n")
        ),
        "{md}"
    );
    assert!(md.contains("## 其他（沒對應到任務）\r\n- 看一下環境"), "{md}");
    assert!(md.find("F2P-97").unwrap() < md.find("其他").unwrap());
    assert!(md.starts_with("# 10/28（週三） 工作日誌\r\n"), "{md}");
}
