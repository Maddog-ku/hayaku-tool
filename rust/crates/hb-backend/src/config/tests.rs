//! Config 的測試：C# 測試（AutoCreateTests 裡的 ProjectConfig、AppState 案例）＋ 跟 C# 檔案格式的相容測試。

use super::cs_fixtures::*;
use super::*;
use crate::infra::json_file;

#[test]
fn project_config_預設值不用存() {
    assert!(ProjectConfig { name: "a".into(), ..Default::default() }.is_default());
    assert!(!ProjectConfig { name: "a".into(), auto_create: true, ..Default::default() }.is_default());
    let json = serde_json::to_string(&ProjectConfig::default()).unwrap();
    assert!(!json.contains("IsDefault") && !json.contains("isDefault"));
}

#[test]
fn app_state_資料夾對應不分大小寫() {
    let state: AppState =
        serde_json::from_str(r#"{"folderHints":{"D:\\repo\\Print-App":{"project":"PRT","parent":"PRT-3"}}}"#).unwrap();
    assert_eq!(Some("PRT-3"), state.folder_hints.get(r"d:\REPO\print-app").and_then(|h| h.parent.as_deref()));
}

#[test]
fn app_state_資料夾對應_保留原本的鍵_設定時不分大小寫() {
    let mut hints = CiMap::new();
    hints.insert(r"D:\repo\A", FolderHint { project: "A".into(), parent: None });
    hints.insert(r"d:\REPO\a", FolderHint { project: "B".into(), parent: None });
    assert_eq!(1, hints.len());
    assert_eq!(vec![r"D:\repo\A"], hints.keys().collect::<Vec<_>>());
    assert_eq!("B", hints.get(r"D:\REPO\A").unwrap().project);
    assert!(hints.remove(r"d:\repo\a").is_some());
    assert!(hints.is_empty());
}

/// C# 寫的 config.json 加上 Rust 版（v2.1.0）新增的兩個欄位：緊接在 notificationPopupSeconds 後面、用預設值。
/// 其他內容、順序、縮排、換行都要跟 C# 一字不差（C# 讀到不認識的欄位會略過）。
fn with_rust_fields(cs: &str) -> String {
    let anchor = "  \"notificationPopupSeconds\": 3,\r\n";
    assert_eq!(1, cs.matches(anchor).count());
    cs.replace(anchor, &format!("{anchor}  \"reminderBeforeEndMinutes\": 15,\r\n  \"newWorkToastSeconds\": 300,\r\n"))
}

#[test]
fn app_config_預設值寫出來跟c_sharp一字不差() {
    assert_eq!(with_rust_fields(CS_CONFIG_DEFAULT), json_file::to_string(&AppConfig::default()).unwrap());
}

#[test]
fn app_config_讀c_sharp的檔再寫回_一字不差() {
    let config: AppConfig = serde_json::from_str(CS_CONFIG_CUSTOM).unwrap();
    assert_eq!("https://example.test/", config.base_url);
    assert_eq!(15, config.claude_idle_minutes);
    assert_eq!(-12.5, config.taskbar_offset);
    assert_eq!(0, config.panel_height);
    assert_eq!("", config.hotkey);
    assert_eq!(2, config.task_columns.len());
    assert_eq!(120.5, config.task_columns[1].width);
    assert!(!config.task_columns[1].visible);
    assert_eq!(r"D:\repo", config.repos[0].path);
    assert!(config.repos[0].auto_create && config.repos[0].include_subprojects);
    assert!(!config.repos[0].projects[0].enabled);
    // C# 的檔沒有 Rust 版新增的欄位：用預設值
    assert_eq!(15, config.reminder_before_end_minutes);
    assert_eq!(300, config.new_work_toast_seconds);
    assert_eq!(with_rust_fields(CS_CONFIG_CUSTOM), json_file::to_string(&config).unwrap());
}

#[test]
fn app_config_缺欄位用c_sharp的預設值() {
    let config: AppConfig =
        serde_json::from_str(r#"{"pollSeconds": 60, "repos": [{"path": "D:\\x"}], "unknownField": 1}"#).unwrap();
    assert_eq!(60, config.poll_seconds);
    assert_eq!(AppConfig { poll_seconds: 60, repos: config.repos.clone(), ..AppConfig::default() }, config);
    assert!(config.repos[0].enabled && config.repos[0].include_subprojects && !config.repos[0].auto_create);
    let column: TaskColumnConfig = serde_json::from_str(r#"{"key": "title"}"#).unwrap();
    assert!(column.visible);
    assert_eq!(AppConfig::default(), serde_json::from_str::<AppConfig>("{}").unwrap());
}

#[test]
fn app_state_預設值寫出來跟c_sharp一字不差() {
    assert_eq!(CS_STATE_DEFAULT, json_file::to_string(&AppState::default()).unwrap());
    assert_eq!(AppState::default(), serde_json::from_str::<AppState>(CS_STATE_DEFAULT).unwrap());
}

#[test]
fn app_state_讀c_sharp的檔再寫回_一字不差() {
    let state: AppState = serde_json::from_str(CS_STATE_CUSTOM).unwrap();
    assert_eq!("2026-11-12", state.date);
    assert!(state.clocked_out);
    let pause = state.meeting_pause.as_ref().unwrap();
    assert_eq!("REVIVAL-9", pause.meeting);
    assert_eq!(vec!["F2P-1", "EG-3"], pause.tasks);
    assert_eq!(Some("F2P-1"), state.session_tasks.get("abc-123").map(String::as_str));
    assert_eq!(None, state.folder_hints.get(r"d:\repo\other").unwrap().parent);
    assert_eq!(vec!["2026-11-14", "2026-11-17", "2026-11-18"], state.off_days.keys().collect::<Vec<_>>());
    assert_eq!("請假", state.off_days["2026-11-17"]);
    assert_eq!(Some("顧 \"小明\" <x> & 'y' 😀"), state.user_display_name.as_deref());
    assert_eq!(CS_STATE_CUSTOM, json_file::to_string(&state).unwrap());
}

#[test]
fn app_state_存檔讀檔() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let state: AppState = serde_json::from_str(CS_STATE_CUSTOM).unwrap();
    state.save_to(&path);
    assert_eq!(CS_STATE_CUSTOM, std::fs::read_to_string(&path).unwrap());
    assert_eq!(state, AppState::load_from(&path));
}

#[test]
fn app_config_load_save用app_paths的資料夾() {
    let _guard = crate::infra::app_paths::tests::lock();
    let dir = tempfile::tempdir().unwrap();
    crate::infra::AppPaths::set_dir(Some(dir.path()));
    let mut config = AppConfig::load();
    assert_eq!(AppConfig::default(), config);
    config.poll_seconds = 45;
    config.save();
    assert_eq!(45, AppConfig::load().poll_seconds);
    assert!(dir.path().join("config.json").is_file());
    crate::infra::AppPaths::set_dir(None);
}

#[test]
fn roll_day_換日重設當天欄位() {
    let mut state: AppState = serde_json::from_str(CS_STATE_CUSTOM).unwrap();
    assert!(!state.roll_day("2026-11-12"));
    assert!(state.clocked_out);
    state.asked_sessions = (0..250).map(|i| i.to_string()).collect();
    state.session_tasks = (0..201).map(|i| (i.to_string(), "X-1".to_owned())).collect();
    assert!(state.roll_day("2026-11-13"));
    assert_eq!("2026-11-13", state.date);
    assert!(state.handled_boundaries.is_empty());
    assert!(!state.clocked_out);
    assert!(state.claude_owned.is_empty());
    assert!(state.meeting_pause.is_none());
    assert!(state.session_tasks.is_empty());
    assert_eq!(100, state.asked_sessions.len());
    assert_eq!("150", state.asked_sessions[0]);
    assert_eq!(vec!["F2P-1"], state.resume_set); // 跨日保留
    assert_eq!(3, state.off_days.len());
}
