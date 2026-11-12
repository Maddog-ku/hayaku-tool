//! eip-sync.json 與 eip.bin 的格式（C# 沒有對應的測試；確認跟 C# 版互通）。

use chrono::{Local, NaiveDate, TimeZone};

use crate::{EipCredentialStore, EipCredentials, EipSyncState, SyncedLeave};

/// C# 版（System.Text.Json、JsonFile.Options）寫出的 eip-sync.json 格式：
/// 本機時間帶時區、假單起訖不帶時區、null 照寫、縮排兩格、CRLF、中文不跳脫。
fn cs_json(offset: &str) -> String {
    [
        "{",
        "  \"lastSyncDay\": \"2026-11-11\",",
        &format!("  \"lastSyncAt\": \"2026-11-11T17:31:02.1234567{offset}\","),
        "  \"lastResult\": \"新增 1 筆\",",
        "  \"morningDay\": \"\",",
        "  \"morningAt\": null,",
        "  \"morningResult\": \"\",",
        &format!("  \"afterWorkAt\": \"2026-11-11T17:31:02.12{offset}\","),
        "  \"afterWorkResult\": \"新增 1 筆\",",
        "  \"leaves\": [",
        "    {",
        "      \"no\": \"20261027-0078\",",
        "      \"type\": \"特休\",",
        "      \"start\": \"2026-10-30T08:30:00\",",
        "      \"end\": \"2026-10-30T17:30:00\",",
        "      \"hayakuIds\": [",
        "        \"lv1\"",
        "      ],",
        "      \"summary\": \"10/30（週五） 整天 特休\"",
        "    }",
        "  ]",
        "}",
    ]
    .join("\r\n")
}

fn local_offset() -> String {
    let t = NaiveDate::from_ymd_opt(2026, 11, 11).unwrap().and_hms_opt(17, 31, 2).unwrap();
    Local.from_local_datetime(&t).single().unwrap().format("%:z").to_string()
}

#[test]
fn eip_sync_json_跟c_sharp格式相容() {
    let text = cs_json(&local_offset());
    let state: EipSyncState = serde_json::from_str(&text).unwrap();
    assert_eq!("2026-11-11", state.last_sync_day);
    let at = NaiveDate::from_ymd_opt(2026, 11, 11).unwrap().and_hms_nano_opt(17, 31, 2, 123_456_700).unwrap();
    assert_eq!(Some(at), state.last_sync_at);
    assert_eq!(None, state.morning_at);
    assert_eq!(1, state.leaves.len());
    assert_eq!(NaiveDate::from_ymd_opt(2026, 10, 30).unwrap().and_hms_opt(8, 30, 0).unwrap(), state.leaves[0].start);
    assert!(state.is_synced("lv1"));
    assert!(!state.is_synced("lv2"));
    // 寫回去一字不差
    assert_eq!(text, hb_backend::infra::json_file::to_string(&state).unwrap());
}

#[test]
fn eip_sync_json_缺欄位用預設值_存檔再讀回() {
    let state: EipSyncState = serde_json::from_str("{\"leaves\":[{\"no\":\"A\"}]}").unwrap();
    assert_eq!("", state.last_sync_day);
    assert_eq!(None, state.last_sync_at);
    assert_eq!(SyncedLeave { no: "A".into(), ..Default::default() }, state.leaves[0]);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eip-sync.json");
    assert_eq!(EipSyncState::default(), EipSyncState::load_from(&path));
    let state = EipSyncState {
        last_sync_day: "2026-11-12".into(),
        last_sync_at: Some(Local::now().naive_local()),
        ..Default::default()
    };
    state.save_to(&path);
    assert_eq!(state, EipSyncState::load_from(&path));
}

#[test]
fn eip_bin_dpapi存檔再讀回() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eip.bin");
    assert!(EipCredentialStore::load_from(&path).is_empty());
    let cred = EipCredentials::new("Y2798", "密碼 pw");
    EipCredentialStore::save_to(&path, &cred).unwrap();
    assert!(!std::fs::read(&path).unwrap().windows(5).any(|w| w == b"Y2798"));
    assert_eq!(cred, EipCredentialStore::load_from(&path));
    // 壞掉的檔案：回空的
    std::fs::write(&path, b"garbage").unwrap();
    assert_eq!(EipCredentials::default(), EipCredentialStore::load_from(&path));
    EipCredentialStore::clear_at(&path);
    assert!(!path.exists());
    EipCredentialStore::clear_at(&path); // 不存在也不會出錯
}

#[test]
fn eip_bin_內容是c_sharp的json格式() {
    let cred = EipCredentials::new("a", "b");
    assert_eq!(
        "{\r\n  \"account\": \"a\",\r\n  \"password\": \"b\"\r\n}",
        hb_backend::infra::json_file::to_string(&cred).unwrap()
    );
    assert!(EipCredentials::new("a", "").is_empty());
}

/// 手動跟 C# 版互通測試（DPAPI 檔只有同一個 Windows 帳號解得開，所以不放進一般測試）：
/// 先用 C# 的 `EipCredentialStore.Save` 產生 `cs_eip.bin`（帳號 a、密碼 b），設定 `HB_CS_FIXTURE_DIR` 指向那個資料夾，
/// 執行 `cargo test -p hb-eip -- --ignored`；會寫出 `rust_eip.bin` 讓 C# 端解密驗證。
#[test]
#[ignore]
fn 手動_跟c_sharp的dpapi檔互通() {
    let dir = std::path::PathBuf::from(std::env::var("HB_CS_FIXTURE_DIR").unwrap());
    assert_eq!(EipCredentials::new("a", "b"), EipCredentialStore::load_from(&dir.join("cs_eip.bin")));
    EipCredentialStore::save_to(&dir.join("rust_eip.bin"), &EipCredentials::new("a", "b")).unwrap();
}
