//! 移植自 AutoCreateTests.cs 的 Claude 部分（AutoCreateTests 前 7 個、EstimateTests）。
//! ProjectConfig_預設值不用存、AppState 在 hb-backend，ProjectGuess 在 hb-core。

use std::path::Path;

use chrono::Local;
use hb_backend::config::{IndexMap, ProjectConfig, RepoConfig};
use serde_json::Value;

use super::{ClaudeDir, js, path_str, temp_dir};
use crate::util::now;
use crate::{ClaudeEstimator, ClaudeMonitor, RepoActivity};

/// 在暫存的 Claude 設定資料夾裡寫一個 session 檔，掃描後回傳那個監看單位。
fn scan_session(repo_path: &str, claude: &ClaudeDir, lines: &[String]) -> RepoActivity {
    let session_dir = claude.projects().join(ClaudeMonitor::encode(repo_path).unwrap());
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("s1.jsonl"), lines.join("\n") + "\n").unwrap();
    let mut monitor = ClaudeMonitor::new();
    monitor.is_known_key = std::sync::Arc::new(|k: &str| k.starts_with("EG-"));
    monitor.sync(&[RepoConfig {
        path: ClaudeMonitor::normalize(repo_path).unwrap(),
        auto_create: true,
        ..Default::default()
    }]);
    monitor.scan();
    monitor.resolve(now(), 10, &IndexMap::new());
    assert_eq!(1, monitor.repos().count());
    monitor.repos().next().unwrap().clone()
}

fn user(cwd: &str, text: &str) -> String {
    format!(
        r#"{{"type":"user","message":{{"role":"user","content":{}}},"cwd":{},"sessionId":"s1","timestamp":"2026-10-28T02:00:00Z"}}"#,
        js(text),
        js(cwd)
    )
}

fn tool_use(name: &str, file: &str, at: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"改一下"}},{{"type":"tool_use","id":"t1","name":"{name}","input":{{"file_path":{}}}}}]}},"timestamp":"{at}"}}"#,
        js(file)
    )
}

const AT: &str = "2026-10-28T02:05:00Z";

fn local(ts: &str) -> chrono::NaiveDateTime {
    chrono::DateTime::parse_from_rfc3339(ts).unwrap().with_timezone(&Local).naive_local()
}

fn with_claude_dir(test: impl FnOnce(&ClaudeDir, &str)) {
    let claude = ClaudeDir::new();
    let repo_dir = temp_dir();
    test(&claude, &path_str(repo_dir.path()));
}

#[test]
fn 在專案裡改檔_記下改檔時間與第一句指令() {
    with_claude_dir(|claude, repo| {
        let a = scan_session(
            repo,
            claude,
            &[
                user(repo, "繼續"),
                user(repo, "列印頁面加上全部列印按鈕\n細節如下…"),
                tool_use("Edit", &path_str(&Path::new(repo).join("src").join("Print.cs")), AT),
            ],
        );
        assert!(a.auto_create);
        assert_eq!(Some("列印頁面加上全部列印按鈕"), a.first_prompt.as_deref()); // 「繼續」這類沒內容的跳過
        assert_eq!(Some(local(AT)), a.edited_at);
        assert_eq!(None, a.resolved_task);
    });
}

#[test]
fn 只是問問題_沒有改檔時間() {
    with_claude_dir(|claude, repo| {
        let a = scan_session(
            repo,
            claude,
            &[
                user(repo, "這個函式在做什麼？"),
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"x"}}]}}"#.to_owned(),
                r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"git status"}}]}}"#.to_owned(),
            ],
        );
        assert_eq!(None, a.edited_at);
        assert_eq!(Some("這個函式在做什麼"), a.first_prompt.as_deref());
    });
}

#[test]
fn 用bash改檔_也記下改檔時間() {
    with_claude_dir(|claude, repo| {
        let bash = |cwd: &str, command: &str, at: &str| {
            format!(
                r#"{{"type":"assistant","cwd":{},"message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t1","name":"Bash","input":{{"command":{}}}}}]}},"timestamp":"{at}"}}"#,
                js(cwd),
                js(command)
            )
        };
        let a = scan_session(
            repo,
            claude,
            &[
                user(repo, "匯入時順便更新使用者資訊"),
                bash(repo, "sed -n 1,60p a.java", "2026-10-28T02:03:00Z"),
                bash(repo, "sed -i 's/a/b/' a.java", "2026-10-28T02:04:00Z"),
            ],
        );
        assert_eq!(Some(local("2026-10-28T02:04:00Z")), a.edited_at);
    });
}

#[test]
fn 改的是專案外的檔案_例如計畫檔_不算() {
    with_claude_dir(|claude, repo| {
        let plan = path_str(&Path::new(&claude.path()).join("plans").join("plan.md"));
        let a = scan_session(repo, claude, &[user(repo, "先規劃一下"), tool_use("Write", &plan, AT)]);
        assert_eq!(None, a.edited_at);
    });
}

#[test]
fn 有任務編號的session_resolved_task有值() {
    with_claude_dir(|claude, repo| {
        let a = scan_session(
            repo,
            claude,
            &[user(repo, "修 EG-7 的查詢欄位"), tool_use("MultiEdit", &path_str(&Path::new(repo).join("a.cs")), AT)],
        );
        assert!(a.edited_at.is_some());
        assert_eq!(Some("EG-7"), a.resolved_task.as_deref()); // 已有任務，引擎不會詢問
    });
}

#[test]
fn edit_time_git_bash路徑也認得() {
    let doc: Value = serde_json::from_str(&tool_use("Write", "/d/repo/Print-App/src/a.ts", AT)).unwrap();
    assert!(ClaudeMonitor::edit_time(&doc, r"D:\repo\Print-App").is_some());
    assert!(ClaudeMonitor::edit_time(&doc, r"D:\repo\Other").is_none());
}

#[test]
fn 子專案的自動建立任務設定() {
    let _claude = ClaudeDir::new();
    let root = temp_dir();
    std::fs::create_dir_all(root.path().join("Print-App").join(".git")).unwrap();
    std::fs::create_dir_all(root.path().join("Api").join(".git")).unwrap();
    let mut monitor = ClaudeMonitor::new();
    monitor.sync(&[RepoConfig {
        path: path_str(root.path()),
        projects: vec![ProjectConfig { name: "Print-App".into(), auto_create: true, ..Default::default() }],
        ..Default::default()
    }]);
    let single = |pred: &dyn Fn(&RepoActivity) -> bool| {
        let found: Vec<&RepoActivity> = monitor.repos().filter(|r| pred(r)).collect();
        assert_eq!(1, found.len());
        found[0].auto_create
    };
    assert!(single(&|r| r.name() == "Print-App"));
    assert!(!single(&|r| r.name() == "Api")); // 預設關閉
    assert!(!single(&|r| r.root_path.is_none()));
}

// ───────────── EstimateTests ─────────────

#[test]
fn 回覆換成以半小時為單位() {
    for (reply, expected) in [("4", 4.0), ("1.5\n", 1.5), ("約 2.3 小時", 2.5), ("0.2", 0.5), ("3.75", 4.0)] {
        assert_eq!(Some(expected), ClaudeEstimator::parse_hours(Some(reply)), "{reply}");
    }
}

#[test]
fn 看不懂或不合理回null() {
    for reply in ["", "不知道", "0", "5000"] {
        assert_eq!(None, ClaudeEstimator::parse_hours(Some(reply)), "{reply}");
    }
}

#[test]
fn 提示包含名稱專案與指令() {
    let prompt =
        ClaudeEstimator::build_prompt("列印頁面加上全部列印按鈕", Some("Print-App"), Some("幫我在列印頁加按鈕"));
    assert!(prompt.contains("任務名稱：列印頁面加上全部列印按鈕"));
    assert!(prompt.contains("專案：Print-App"));
    assert!(prompt.contains("幫我在列印頁加按鈕"));
}

#[test]
fn 讀session開頭的指令原文_跳過繼續與圖片標記() {
    let u = |text: &str| {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":{}}},"cwd":"D:\repo","sessionId":"s1"}}"#,
            js(text)
        )
    };
    let dir = temp_dir();
    let file = dir.path().join("x.jsonl");
    std::fs::write(
        &file,
        [
            u("[Image #1] 列印頁面加上全部列印按鈕\n按鈕放在右上角"),
            u("繼續"),
            u("順便加選擇列印"),
            u("第三句"),
            u("第四句不讀"),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    let text = ClaudeMonitor::read_opening_prompts(&file);
    assert_eq!(Some("列印頁面加上全部列印按鈕\n按鈕放在右上角\n---\n順便加選擇列印\n---\n第三句"), text.as_deref());
}

#[test]
fn 讀不到檔案回null() {
    let dir = temp_dir();
    assert_eq!(None, ClaudeMonitor::read_opening_prompts(&dir.path().join("nope.jsonl")));
}
