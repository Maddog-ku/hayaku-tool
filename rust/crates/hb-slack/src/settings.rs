//! 對應 C# `UI/SettingsWindow.Slack.cs` 裡不碰畫面的部分：設定的 Slack 頁要顯示的文字、連線並送測試訊息、中斷連線。
//! 畫面（輸入框、按鈕停用、剪貼簿、確認對話框、開網頁）由 hayakubar 處理，照下面每個函式註明的 C# 流程接。

use std::rc::Rc;

use chrono::Local;
use hb_core::SlackLink;

use crate::slack_client::{SlackClient, SlackError};
use crate::slack_credentials::{SlackCredentialStore, SlackCredentials};

/// 狀態文字的顏色（C# 的 Brush 名稱）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// `MutedBrush`
    Muted,
    /// `ErrorBrush`
    Error,
    /// `RunBrush`
    Run,
}

/// 設定頁的一行狀態（C# `ShowSlackStatus(text, brush)`／`SlackLiveStatus`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackStatus {
    pub text: String,
    pub tone: Tone,
}

impl SlackStatus {
    fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self { text: text.into(), tone }
    }
}

/// 「送測試訊息」的結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOutcome {
    /// 沒有存金鑰：畫面改回建立 App 的步驟（C# `ShowSlackAccount(null)`），不顯示狀態。
    NotConfigured,
    /// 顯示這行狀態（成功 `Run`、失敗 `Error`）。
    Done(SlackStatus),
}

pub struct SlackSettings;

impl SlackSettings {
    /// 按「連線」後、等結果時顯示（`MutedBrush`）。
    pub const CONNECTING: &'static str = "連線中…";
    /// 按「送測試訊息」後、等結果時顯示（`MutedBrush`）。
    pub const SENDING: &'static str = "傳送中…";
    /// 中斷連線的確認對話框（OK／Cancel、問號圖示）。
    pub const DISCONNECT_TITLE: &'static str = "Slack";
    pub const DISCONNECT_CONFIRM: &'static str =
        "中斷 Slack 連線？會刪掉存在這台電腦的金鑰，之後不再傳到 Slack。Slack 上的 App 不會被刪除。";
    /// 「建立 App」要開的網頁（C# `Web.Open(SlackClient.AppsUrl)`）。
    pub const APPS_URL: &'static str = SlackClient::APPS_URL;
    /// 「複製 App 設定」要放進剪貼簿的內容。
    pub const MANIFEST: &'static str = SlackClient::MANIFEST;

    /// C# `LoadSlack`／`ShowSlackAccount`：已存的金鑰；None＝沒連線（顯示建立 App 的步驟與輸入欄位、清空兩個 Token 欄位）。
    /// 有值時顯示工作區文字（[`SlackSettings::account_text`]）、「送測試訊息」「中斷連線」與轉送開關；
    /// 轉送開關是 `config.slack_forward`，改了存設定（C# `SlackForward_Click` → `Apply()`）。
    pub fn load_account() -> Option<SlackCredentials> {
        let cred = SlackCredentialStore::load();
        (!cred.is_empty()).then_some(cred)
    }

    /// 已連線時顯示的帳號說明。
    pub fn account_text(cred: &SlackCredentials) -> String {
        let team = if cred.team.is_empty() { String::new() } else { format!("{} · ", cred.team) };
        format!("{team}私訊給 {} · 金鑰用 Windows 加密存在這台電腦", cred.user_id)
    }

    /// C# `RefreshSlackStatus`：連線狀態那一行。設定頁開著時訂閱 `subscribe_status_changed`，關掉時取消。
    /// 有設定、不正常、又不是「連線中…」才用紅字。
    pub fn live_status(slack: Option<&dyn SlackLink>) -> SlackStatus {
        let Some(slack) = slack else {
            return SlackStatus::new("", Tone::Muted);
        };
        let text = slack.status_text();
        let error = slack.configured() && !slack.healthy() && !text.starts_with("連線中");
        SlackStatus::new(text, if error { Tone::Error } else { Tone::Muted })
    }

    /// C# `SlackCopyManifest_Click`：把 [`SlackSettings::MANIFEST`] 放進剪貼簿後，依成功與否顯示這行。
    pub fn copy_manifest_status(copied: bool) -> SlackStatus {
        if copied {
            SlackStatus::new("已複製 App 設定，到 Slack 貼上", Tone::Muted)
        } else {
            SlackStatus::new("複製失敗，剪貼簿被別的程式占用，再試一次", Tone::Error)
        }
    }

    /// C# `SlackConnect_Click` 前段：檢查輸入（前後空白去掉）。不對就直接顯示錯誤，不用停用按鈕。
    pub fn validate(bot_token: &str, app_token: &str, user_id: &str) -> Result<SlackCredentials, SlackStatus> {
        let cred = SlackCredentials {
            bot_token: bot_token.trim().to_owned(),
            app_token: app_token.trim().to_owned(),
            user_id: user_id.trim().to_owned(),
            team: String::new(),
        };
        if cred.is_empty() {
            return Err(SlackStatus::new("請填 Bot Token、App-Level Token 和成員 ID", Tone::Error));
        }
        if !cred.bot_token.starts_with("xoxb-") || !cred.app_token.starts_with("xapp-") {
            return Err(SlackStatus::new(SlackClient::describe("not_allowed_token_type"), Tone::Error));
        }
        Ok(cred)
    }

    /// C# `SlackConnect_Click` 後段（呼叫前停用「連線」按鈕、顯示 [`SlackSettings::CONNECTING`]，結束後啟用）：
    /// 檢查 Bot Token（拿工作區名稱）、App-Level Token，送測試訊息，存金鑰，SlackBridge 重新連線。
    /// 成功回存好的金鑰（畫面切成已連線、顯示 [`SlackSettings::account_text`]）與狀態；失敗回狀態。
    pub async fn connect(
        cred: SlackCredentials,
        slack: Option<Rc<dyn SlackLink>>,
    ) -> Result<(SlackCredentials, SlackStatus), SlackStatus> {
        Self::connect_with(&SlackClient::new(), cred, slack).await
    }

    pub(crate) async fn connect_with(
        client: &SlackClient,
        mut cred: SlackCredentials,
        slack: Option<Rc<dyn SlackLink>>,
    ) -> Result<(SlackCredentials, SlackStatus), SlackStatus> {
        let result: Result<(), String> = async {
            cred.team = client.auth_test(&cred.bot_token).await.map_err(|e| e.message())?;
            client.open_socket_url(&cred.app_token).await.map_err(|e| e.message())?;
            send_test(client, &cred).await.map_err(|e| e.message())?;
            SlackCredentialStore::save(&cred)?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                if let Some(slack) = slack {
                    slack.restart();
                }
                log::info!("Slack 已連線（{}，私訊給 {}）", cred.team, cred.user_id);
                let status =
                    SlackStatus::new("已連線，到 Slack 左邊「應用程式」的 HayakuBar 看看有沒有收到測試訊息", Tone::Run);
                Ok((cred, status))
            }
            Err(message) => {
                log::error!("Slack 連線失敗: {message}");
                Err(SlackStatus::new(format!("連線失敗：{message}"), Tone::Error))
            }
        }
    }

    /// C# `SlackTest_Click`（呼叫前停用「送測試訊息」按鈕、顯示 [`SlackSettings::SENDING`]，結束後啟用）。
    pub async fn send_test() -> TestOutcome {
        Self::send_test_with(&SlackClient::new()).await
    }

    pub(crate) async fn send_test_with(client: &SlackClient) -> TestOutcome {
        let cred = SlackCredentialStore::load();
        if cred.is_empty() {
            return TestOutcome::NotConfigured;
        }
        match send_test(client, &cred).await {
            Ok(()) => TestOutcome::Done(SlackStatus::new("已送出測試訊息", Tone::Run)),
            Err(err) => {
                log::error!("Slack 測試訊息失敗: {err}");
                TestOutcome::Done(SlackStatus::new(format!("傳送失敗：{}", err.message()), Tone::Error))
            }
        }
    }

    /// C# `SlackLogout_Click`（使用者在確認對話框按了 OK 之後）：刪掉金鑰、SlackBridge 重新讀（變成沒設定）。
    /// 之後畫面清空成員 ID 欄位、切回建立 App 的步驟、隱藏狀態。
    pub fn disconnect(slack: Option<&dyn SlackLink>) {
        SlackCredentialStore::clear();
        if let Some(slack) = slack {
            slack.restart();
        }
        log::info!("Slack 已中斷連線");
    }
}

/// C# `SendTestAsync`：開私訊、送一則測試訊息。
async fn send_test(client: &SlackClient, cred: &SlackCredentials) -> Result<(), SlackError> {
    let channel = client.open_dm(&cred.bot_token, &cred.user_id).await?;
    let text = format!("✓ HayakuBar 已連上 Slack（測試訊息，{}）", Local::now().format("%-m/%-d %H:%M"));
    client.post_message(&cred.bot_token, &channel, &text, None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{MockServer, Reply, TempApp, block_on};

    #[test]
    fn 檢查輸入() {
        assert_eq!(
            Err(SlackStatus::new("請填 Bot Token、App-Level Token 和成員 ID", Tone::Error)),
            SlackSettings::validate("xoxb-1", " ", "U1")
        );
        assert_eq!(
            "金鑰種類放錯了：Bot Token 是 xoxb- 開頭，App-Level Token 是 xapp- 開頭",
            SlackSettings::validate("xapp-1", "xoxb-1", "U1").unwrap_err().text
        );
        let cred = SlackSettings::validate(" xoxb-1 ", "xapp-1\n", " U1").unwrap();
        assert_eq!(
            ("xoxb-1", "xapp-1", "U1"),
            (cred.bot_token.as_str(), cred.app_token.as_str(), cred.user_id.as_str())
        );
    }

    #[test]
    fn 帳號說明() {
        let mut cred = SlackCredentials { user_id: "U1".into(), team: "公司".into(), ..Default::default() };
        assert_eq!("公司 · 私訊給 U1 · 金鑰用 Windows 加密存在這台電腦", SlackSettings::account_text(&cred));
        cred.team.clear();
        assert_eq!("私訊給 U1 · 金鑰用 Windows 加密存在這台電腦", SlackSettings::account_text(&cred));
    }

    #[test]
    fn 連線_檢查兩個金鑰_送測試訊息_存檔() {
        let app = TempApp::new();
        let server = MockServer::start(|req| match req.path.as_str() {
            "/api/auth.test" => Reply::json(200, r#"{"ok":true,"team":"公司"}"#),
            "/api/apps.connections.open" => Reply::json(200, r#"{"ok":true,"url":"wss://x"}"#),
            "/api/conversations.open" => Reply::json(200, r#"{"ok":true,"channel":{"id":"D1"}}"#),
            _ => Reply::json(200, r#"{"ok":true,"ts":"1.2"}"#),
        });
        let client = SlackClient::with_base_url(format!("{}/api/", server.base_url));
        let cred = SlackSettings::validate("xoxb-1", "xapp-1", "U1").unwrap();
        let (saved, status) = block_on(SlackSettings::connect_with(&client, cred, None)).unwrap();
        assert_eq!("公司", saved.team);
        assert_eq!(Tone::Run, status.tone);
        assert_eq!("已連線，到 Slack 左邊「應用程式」的 HayakuBar 看看有沒有收到測試訊息", status.text);
        let reqs = server.requests();
        let paths: Vec<&str> = reqs.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(
            vec!["/api/auth.test", "/api/apps.connections.open", "/api/conversations.open", "/api/chat.postMessage"],
            paths
        );
        assert_eq!(Some("Bearer xapp-1"), reqs[1].headers.get("authorization").map(String::as_str));
        assert!(reqs[3].body.contains("✓ HayakuBar 已連上 Slack（測試訊息，"), "{}", reqs[3].body);
        assert!(app.dir.path().join("slack.bin").is_file());
        assert_eq!(Some(saved), SlackSettings::load_account());

        // 送測試訊息用存好的金鑰
        assert_eq!(
            TestOutcome::Done(SlackStatus::new("已送出測試訊息", Tone::Run)),
            block_on(SlackSettings::send_test_with(&client))
        );

        SlackSettings::disconnect(None);
        assert_eq!(None, SlackSettings::load_account());
        assert_eq!(TestOutcome::NotConfigured, block_on(SlackSettings::send_test_with(&client)));
    }

    #[test]
    fn 連線失敗_顯示原因_不存檔() {
        let app = TempApp::new();
        let server = MockServer::start(|_| Reply::json(200, r#"{"ok":false,"error":"invalid_auth"}"#));
        let client = SlackClient::with_base_url(format!("{}/api/", server.base_url));
        let cred = SlackSettings::validate("xoxb-1", "xapp-1", "U1").unwrap();
        let err = block_on(SlackSettings::connect_with(&client, cred, None)).unwrap_err();
        assert_eq!(SlackStatus::new("連線失敗：auth.test：金鑰不對，請重新複製", Tone::Error), err);
        assert!(!app.dir.path().join("slack.bin").exists());
    }

    #[test]
    fn 複製app設定的狀態() {
        assert_eq!(Tone::Muted, SlackSettings::copy_manifest_status(true).tone);
        assert_eq!("複製失敗，剪貼簿被別的程式占用，再試一次", SlackSettings::copy_manifest_status(false).text);
    }
}
