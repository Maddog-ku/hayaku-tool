//! 移植自 ClaudeTests.cs（ClaudeMonitorTests、HookInstallerTests）。

use std::path::Path;
use std::time::Duration;

use chrono::TimeDelta;
use hb_backend::config::{IndexMap, ProjectConfig, RepoConfig};
use serde_json::{Value, json};

use super::{ClaudeDir, js, path_str, temp_dir};
use crate::util::now;
use crate::{ClaudeMonitor, HookInstaller};

fn repo_config(path: &str) -> RepoConfig {
    RepoConfig { path: path.to_owned(), ..Default::default() }
}

fn no_tasks() -> IndexMap<String, String> {
    IndexMap::new()
}

fn mkdir(p: &Path) -> String {
    std::fs::create_dir_all(p).unwrap();
    path_str(p)
}

#[test]
fn encode_跟claude_code的資料夾命名一致() {
    assert_eq!("D--sj-hayaku-tool", ClaudeMonitor::encode(r"D:\sj\hayaku tool").unwrap());
    assert_eq!("C--Users-harry-gu", ClaudeMonitor::encode(r"C:\Users\harry.gu\").unwrap());
}

#[test]
fn scan_從session紀錄抓分支與對話中的任務() {
    let claude = ClaudeDir::new();
    let repo_dir = temp_dir();
    let repo_path = path_str(repo_dir.path());
    let session_dir = claude.projects().join(ClaudeMonitor::encode(&repo_path).unwrap());
    std::fs::create_dir_all(&session_dir).unwrap();
    let cwd = js(&repo_path);
    let lines = [
        r#"{"type":"mode","mode":"normal","sessionId":"s1"}"#.to_owned(),
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":"幫我修 EG-7 的查詢欄位"}},"gitBranch":"feature/f2p-97-reset","cwd":{cwd},"sessionId":"s1","entrypoint":"claude-desktop"}}"#
        ),
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"好的 F2P-14"}]},"gitBranch":"feature/f2p-97-reset"}"#.to_owned(),
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"EG-11 meta 不算"}}"#.to_owned(),
    ];
    std::fs::write(session_dir.join("s1.jsonl"), lines.join("\n") + "\n").unwrap();

    let mut monitor = ClaudeMonitor::new();
    monitor.is_known_key = std::sync::Arc::new(|k: &str| k.starts_with("EG-") || k.starts_with("F2P-"));
    let repo = repo_config(&ClaudeMonitor::normalize(&repo_path).unwrap());
    monitor.sync(&[repo]);
    monitor.scan();
    monitor.resolve(now(), 10, &no_tasks());

    assert_eq!(1, monitor.repos().count());
    let a = monitor.repos().next().unwrap();
    assert_eq!(Some("s1"), a.session_id.as_deref());
    assert_eq!(Some("feature/f2p-97-reset"), a.branch.as_deref());
    assert!(a.active);
    assert_eq!("桌面版", a.source_text());
    assert_eq!(Some("EG-7"), a.resolved_task.as_deref()); // 對話內容優先於分支

    let mut picked = IndexMap::new();
    picked.insert("s1".to_owned(), "F2P-21".to_owned());
    monitor.resolve(now(), 10, &picked);
    assert_eq!(Some("F2P-21"), monitor.repos().next().unwrap().resolved_task.as_deref()); // 手動指定最優先

    monitor.resolve(now() + TimeDelta::minutes(30), 10, &no_tasks());
    assert!(!monitor.repos().next().unwrap().active); // 超過閒置時間
}

#[test]
fn normalize_git_bash路徑也轉成windows路徑() {
    for (input, expected) in [
        ("/d/repo/Print-App", r"D:\repo\Print-App"),
        ("/mnt/c/Users/x/", r"C:\Users\x"),
        (r"D:\發表報告\", r"D:\發表報告"),
    ] {
        assert_eq!(expected, ClaudeMonitor::normalize(input).unwrap(), "{input}");
    }
}

fn write_session(projects_dir: &Path, dir_name: &str, file: &str, cwd: &str, entrypoint: &str) {
    let dir = projects_dir.join(dir_name);
    std::fs::create_dir_all(&dir).unwrap();
    let line = format!(
        r#"{{"type":"user","message":{{"role":"user","content":"hi"}},"cwd":{},"sessionId":"x","entrypoint":"{entrypoint}"}}"#,
        js(cwd)
    );
    std::fs::write(dir.join(file), line + "\n").unwrap();
}

fn sleep() {
    std::thread::sleep(Duration::from_millis(30));
}

/// "/" + 磁碟機小寫 + 其餘路徑換成 /（Git Bash 格式）。
fn posix(path: &str) -> String {
    format!("/{}{}", path[..1].to_lowercase(), path[2..].replace('\\', "/"))
}

#[test]
fn scan_子資料夾開的session算進來_撞名與相鄰repo不算() {
    let claude = ClaudeDir::new();
    let work = temp_dir();
    let projects = claude.projects();
    let repo = mkdir(&work.path().join("卡機"));
    let other = mkdir(&work.path().join("報告")); // 編碼後跟「卡機」同名
    let sibling = mkdir(&work.path().join("卡機-v2"));
    let encoded = ClaudeMonitor::encode(&repo).unwrap();
    assert_eq!(encoded, ClaudeMonitor::encode(&other).unwrap());

    write_session(&projects, &encoded, "other.jsonl", &other, "cli");
    sleep();
    write_session(&projects, &ClaudeMonitor::encode(&sibling).unwrap(), "sibling.jsonl", &sibling, "cli");
    sleep();
    let sub = mkdir(&Path::new(&repo).join("src"));
    write_session(&projects, &ClaudeMonitor::encode(&sub).unwrap(), "mine.jsonl", &posix(&sub), "cli");
    // 最新的是撞名的那個，但它不屬於 repo
    sleep();
    write_session(&projects, &encoded, "other2.jsonl", &other, "cli");

    let mut monitor = ClaudeMonitor::new();
    monitor.sync(&[RepoConfig { path: repo.clone(), include_subprojects: false, ..Default::default() }]);
    monitor.scan();
    assert_eq!(1, monitor.repos().count());
    let a = monitor.repos().next().unwrap();
    assert_eq!(Some("mine"), a.session_id.as_deref());
    assert_eq!("CLI", a.source_text());
    let _ = claude.path();
}

#[test]
fn 主資料夾_自動偵測專案_各自計時_偵測不到任務就不計時() {
    let claude = ClaudeDir::new();
    let work = temp_dir();
    let projects = claude.projects();
    let root = mkdir(&work.path().join("repo"));
    let app = mkdir(&Path::new(&root).join("Print-App"));
    let server = mkdir(&Path::new(&root).join("Print-Server"));
    mkdir(&Path::new(&server).join(".git")); // git repo，沒有 session
    let plain = mkdir(&Path::new(&root).join("docs")); // 一般資料夾，沒有 session → 不算專案
    mkdir(&Path::new(&root).join("node_modules").join(".git"));

    write_session(&projects, &ClaudeMonitor::encode(&app).unwrap(), "a.jsonl", &app, "cli");
    sleep();
    let app_sub = mkdir(&Path::new(&app).join("src"));
    write_session(&projects, &ClaudeMonitor::encode(&app_sub).unwrap(), "b.jsonl", &app_sub, "claude-desktop");
    write_session(&projects, &ClaudeMonitor::encode(&root).unwrap(), "c.jsonl", &root, "cli");

    let mut monitor = ClaudeMonitor::new();
    let detected = monitor.detect_projects(&root, false);
    let mut names: Vec<&str> = detected.iter().map(|p| p.name.as_str()).collect();
    names.sort();
    assert_eq!(vec!["Print-App", "Print-Server"], names);
    assert!(!detected.iter().any(|p| p.path == plain));

    monitor.sync(&[RepoConfig {
        path: root.clone(),
        projects: vec![ProjectConfig { name: "Print-Server".into(), enabled: false, ..Default::default() }],
        ..Default::default()
    }]);
    monitor.scan();
    monitor.resolve(now(), 10, &no_tasks());

    let units: std::collections::HashMap<String, &crate::RepoActivity> =
        monitor.repos().map(|u| (u.name().to_owned(), u)).collect();
    assert_eq!(3, units.len()); // repo、Print-App、Print-Server
    // Print-App 子資料夾開的 session 算 Print-App，不重複算主資料夾
    assert_eq!(Some("b"), units["Print-App"].session_id.as_deref());
    assert!(units["Print-App"].active);
    assert_eq!(None, units["Print-App"].resolved_task); // 沒有編號、沒有分支：不計時，沒有預設任務
    assert_eq!(Some("c"), units["repo"].session_id.as_deref());
    assert_eq!(None, units["repo"].resolved_task);
    assert!(!units["Print-Server"].active);
    assert!(!units["Print-Server"].enabled);
    assert_eq!(Some(root.as_str()), units["Print-Server"].root_path.as_deref());
}

#[test]
fn 主資料夾本身是git_repo時_一般子資料夾不算專案() {
    let claude = ClaudeDir::new();
    let work = temp_dir();
    let root = mkdir(&work.path().join("single"));
    mkdir(&Path::new(&root).join(".git"));
    let src = mkdir(&Path::new(&root).join("src"));
    write_session(&claude.projects(), &ClaudeMonitor::encode(&src).unwrap(), "s.jsonl", &src, "cli");

    let mut monitor = ClaudeMonitor::new();
    assert!(monitor.detect_projects(&root, false).is_empty());
    monitor.sync(&[repo_config(&root)]);
    monitor.scan();
    assert_eq!(1, monitor.repos().count());
    assert_eq!(Some("s"), monitor.repos().next().unwrap().session_id.as_deref());
}

#[test]
fn discover_列出最近用過的資料夾() {
    let claude = ClaudeDir::new();
    let work = temp_dir();
    let projects = claude.projects();
    let a = mkdir(&work.path().join("a"));
    let b = mkdir(&work.path().join("b"));
    write_session(&projects, &ClaudeMonitor::encode(&a).unwrap(), "1.jsonl", &a, "claude-desktop");
    sleep();
    write_session(&projects, &ClaudeMonitor::encode(&b).unwrap(), "2.jsonl", &b, "cli");
    write_session(&projects, "gone", "3.jsonl", &path_str(&work.path().join("deleted")), "cli"); // 資料夾已不存在

    let found = ClaudeMonitor::discover(15);
    assert_eq!(vec![b, a], found.iter().map(|p| p.path.clone()).collect::<Vec<_>>());
    assert_eq!("桌面版", found[1].source);
}

#[test]
fn on_hook_git_bash格式的cwd也對得上() {
    let _claude = ClaudeDir::new();
    let repo_dir = temp_dir();
    let repo = path_str(repo_dir.path());
    let mut monitor = ClaudeMonitor::new();
    monitor.sync(&[repo_config(&repo)]);
    let payload = json!({ "cwd": posix(&repo) + "/src", "hook_event_name": "UserPromptSubmit", "session_id": "s" });
    assert!(monitor.on_hook(&payload, now()));
}

#[test]
fn on_hook_session_end後就不算活動中() {
    let _claude = ClaudeDir::new();
    let repo_dir = temp_dir();
    let repo = path_str(repo_dir.path());
    let mut monitor = ClaudeMonitor::new();
    monitor.sync(&[repo_config(&repo)]);
    let t = now();
    let hook = |ev: &str, prompt: Option<&str>| -> Value {
        json!({ "session_id": "s2", "cwd": path_str(&repo_dir.path().join("src")), "hook_event_name": ev, "prompt": prompt })
    };

    assert!(monitor.on_hook(&hook("UserPromptSubmit", Some("做 F2P-98")), t));
    monitor.resolve(t, 10, &no_tasks());
    assert_eq!(1, monitor.repos().count());
    let a = monitor.repos().next().unwrap();
    assert!(a.active);
    assert_eq!(Some("F2P-98"), a.resolved_task.as_deref());

    monitor.on_hook(&hook("SessionEnd", None), t + TimeDelta::seconds(5));
    monitor.resolve(t + TimeDelta::seconds(6), 10, &no_tasks());
    assert!(!monitor.repos().next().unwrap().active);
}

#[test]
fn on_hook_不是設定的repo就忽略() {
    let _claude = ClaudeDir::new();
    let mut monitor = ClaudeMonitor::new();
    monitor.sync(&[repo_config(r"C:\nowhere\repo")]);
    let payload = json!({ "cwd": r"C:\other", "hook_event_name": "Stop" });
    assert!(!monitor.on_hook(&payload, now()));
}

// ───────────── HookInstallerTests ─────────────

#[test]
fn install_保留既有設定_uninstall_還原() {
    let repo_dir = temp_dir();
    let repo = path_str(repo_dir.path());
    let path = HookInstaller::settings_path(&repo);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        "{\n  \"permissions\": { \"allow\": [\"Bash(npm test)\"] },\n  \"hooks\": { \"Stop\": [ { \"hooks\": [ { \"type\": \"command\", \"command\": \"echo mine\" } ] } ] }\n}",
    )
    .unwrap();

    HookInstaller::install(&repo, 47823).unwrap();
    HookInstaller::install(&repo, 47823).unwrap(); // 重複安裝不會重複加
    assert!(HookInstaller::is_installed(&repo, 47823).unwrap());
    let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!("Bash(npm test)", root["permissions"]["allow"][0].as_str().unwrap());
    assert_eq!(2, root["hooks"]["Stop"].as_array().unwrap().len());
    assert_eq!(1, root["hooks"]["UserPromptSubmit"].as_array().unwrap().len());
    let mut backup = path.as_os_str().to_owned();
    backup.push(".hayakubar.bak");
    assert!(Path::new(&backup).exists());

    HookInstaller::uninstall(&repo, 47823).unwrap();
    assert!(!HookInstaller::is_installed(&repo, 47823).unwrap());
    let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let stop = root["hooks"]["Stop"].as_array().unwrap();
    assert_eq!(1, stop.len());
    assert_eq!("echo mine", stop[0]["hooks"][0]["command"].as_str().unwrap());
    assert!(root["hooks"].get("UserPromptSubmit").is_none());
}

#[test]
fn install_寫出的格式跟c_sharp一樣() {
    let repo_dir = temp_dir();
    let repo = path_str(repo_dir.path());
    HookInstaller::install(&repo, 47823).unwrap();
    let text = std::fs::read_to_string(HookInstaller::settings_path(&repo)).unwrap();
    let cmd = r#"curl -s -m 2 -X POST -H \"Content-Type: application/json\" --data-binary @- http://127.0.0.1:47823/hook || exit 0"#;
    let group = format!(
        "[\r\n      {{\r\n        \"hooks\": [\r\n          {{\r\n            \"type\": \"command\",\r\n            \"command\": \"{cmd}\",\r\n            \"timeout\": 5\r\n          }}\r\n        ]\r\n      }}\r\n    ]"
    );
    let expected = format!(
        "{{\r\n  \"hooks\": {{\r\n    \"UserPromptSubmit\": {group},\r\n    \"Stop\": {group},\r\n    \"SessionEnd\": {group}\r\n  }}\r\n}}\r\n"
    );
    assert_eq!(expected, text);
}
