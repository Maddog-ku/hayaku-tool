//! 對應 C# `Slack/SlackClient.cs`：Slack Web API（https://slack.com/api/）。
//! Bot Token 用來開私訊、傳訊息；App-Level Token 只用來拿 Socket Mode 的連線網址。
//!
//! 對外的 async 方法回傳的 future 可以在任何執行器（`slint::spawn_local`）await：內部用 `hb_backend::rt::bg` 丟到 tokio 跑。
//! C# 的 `CancellationToken` 參數拿掉了：future 被丟棄時，tokio 那邊的請求一起取消。

use std::time::Duration;

use hb_backend::infra::json_file;
use hb_backend::rt;
use serde_json::{Value, json};

/// Slack Web API 回 ok:false 時的錯誤（C# `SlackException`）；`error` 是 Slack 的錯誤代碼（invalid_auth…）。
/// 訊息（C# `ex.Message`）是「方法：說明」。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{method}：{}", SlackClient::describe(.error))]
pub struct SlackException {
    pub method: String,
    pub error: String,
}

impl SlackException {
    pub fn new(method: impl Into<String>, error: impl Into<String>) -> Self {
        Self { method: method.into(), error: error.into() }
    }
}

/// SlackClient 的錯誤。`Display` 就是 C# 的 `ex.Message`。
/// - `Slack`：`SlackException`（Slack 回 ok:false、429）
/// - `Http`：`HttpRequestException`／`TaskCanceledException`（連不上、逾時、HTTP 狀態碼不是成功）
/// - `Other`：其他（回應不是 JSON…）
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SlackError {
    #[error(transparent)]
    Slack(#[from] SlackException),
    #[error("{0}")]
    Http(String),
    #[error("{0}")]
    Other(String),
}

impl SlackError {
    /// 給人看的訊息（C# `ex.Message`）。
    pub fn message(&self) -> String {
        self.to_string()
    }

    /// Slack 的錯誤代碼（只有 `SlackException` 有）。
    pub fn slack_error(&self) -> Option<&str> {
        match self {
            SlackError::Slack(e) => Some(&e.error),
            _ => None,
        }
    }
}

const BASE_URL: &str = "https://slack.com/api/";

/// Slack Web API。`clone()` 很便宜（共用同一個 reqwest client）。
#[derive(Clone)]
pub struct SlackClient {
    http: reqwest::Client,
    base_url: String,
}

impl Default for SlackClient {
    fn default() -> Self {
        Self::new()
    }
}

impl SlackClient {
    /// 在 Slack「Create New App › From a manifest」貼上的 App 設定：私訊權限、Socket Mode、按鈕互動。
    pub const MANIFEST: &'static str = r#"{
  "display_information": {
    "name": "HayakuBar",
    "description": "把 Hayaku 的通知、審核、測試結果傳到 Slack，可以直接按已讀、核准、駁回。"
  },
  "features": {
    "app_home": {
      "home_tab_enabled": false,
      "messages_tab_enabled": true,
      "messages_tab_read_only_enabled": true
    },
    "bot_user": {
      "display_name": "HayakuBar",
      "always_online": false
    }
  },
  "oauth_config": {
    "scopes": {
      "bot": ["chat:write", "im:write"]
    }
  },
  "settings": {
    "interactivity": { "is_enabled": true },
    "org_deploy_enabled": false,
    "socket_mode_enabled": true,
    "token_rotation_enabled": false
  }
}"#;

    pub const APPS_URL: &'static str = "https://api.slack.com/apps?new_app=1";

    /// C# `new HttpClient { BaseAddress = https://slack.com/api/, Timeout = 20 秒 }`。
    pub fn new() -> Self {
        Self::with_base_url(BASE_URL)
    }

    /// 換 API 網址（測試用本機假伺服器）；結尾要有 `/`。
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().unwrap_or_else(|err| {
            log::error!("建立 Slack HttpClient 失敗: {err}");
            reqwest::Client::new()
        });
        Self { http, base_url: base_url.into() }
    }

    /// 檢查 Bot Token，回工作區名稱。
    pub async fn auth_test(&self, bot_token: &str) -> Result<String, SlackError> {
        let r = self.call("auth.test", bot_token, None).await?;
        Ok(r.get("team").and_then(Value::as_str).unwrap_or("").to_owned())
    }

    /// 用 App-Level Token 拿 Socket Mode 的 WebSocket 網址（每次連線都要重拿）。
    pub async fn open_socket_url(&self, app_token: &str) -> Result<String, SlackError> {
        let r = self.call("apps.connections.open", app_token, None).await?;
        r.get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| SlackException::new("apps.connections.open", "no_url").into())
    }

    /// 開（或拿到已經有的）跟這個人的私訊頻道。
    pub async fn open_dm(&self, bot_token: &str, user_id: &str) -> Result<String, SlackError> {
        let r = self.call("conversations.open", bot_token, Some(json!({ "users": user_id }))).await?;
        r.get("channel")
            .and_then(|c| c.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| SlackException::new("conversations.open", "channel_not_found").into())
    }

    /// 傳訊息，回訊息的 ts（之後改訊息要用）。text 是通知列與無法顯示 blocks 時的文字；blocks 是 None 就不送這個欄位。
    pub async fn post_message(
        &self,
        bot_token: &str,
        channel: &str,
        text: &str,
        blocks: Option<Value>,
    ) -> Result<String, SlackError> {
        let mut body = json!({ "channel": channel, "text": text });
        if let Some(b) = blocks {
            body["blocks"] = b;
        }
        body["unfurl_links"] = json!(false);
        let r = self.call("chat.postMessage", bot_token, Some(body)).await?;
        Ok(r.get("ts").and_then(Value::as_str).unwrap_or("").to_owned())
    }

    /// 改已經傳出去的訊息（處理完把按鈕換成結果）。
    pub async fn update_message(
        &self,
        bot_token: &str,
        channel: &str,
        ts: &str,
        text: &str,
        blocks: Option<Value>,
    ) -> Result<(), SlackError> {
        let mut body = json!({ "channel": channel, "ts": ts, "text": text });
        if let Some(b) = blocks {
            body["blocks"] = b;
        }
        self.call("chat.update", bot_token, Some(body)).await.map(|_| ())
    }

    /// 跳出表單（駁回、退回要填原因）；trigger_id 只有 3 秒有效。
    pub async fn open_view(&self, bot_token: &str, trigger_id: &str, view: Value) -> Result<(), SlackError> {
        self.call("views.open", bot_token, Some(json!({ "trigger_id": trigger_id, "view": view }))).await.map(|_| ())
    }

    /// 送出 POST，在 tokio 上跑；body 是 None 時送空的表單（C# `FormUrlEncodedContent([])`）。
    async fn call(&self, method: &str, token: &str, body: Option<Value>) -> Result<Value, SlackError> {
        let http = self.http.clone();
        let url = format!("{}{method}", self.base_url);
        let method = method.to_owned();
        let token = token.to_owned();
        rt::bg(async move {
            let request = http.post(url).bearer_auth(token);
            let request = match body {
                None => request.form(&[] as &[(&str, &str)]),
                Some(b) => {
                    // C# BodyOptions：不寫 null、中文不跳脫、不縮排
                    let text = json_file::to_string_compact(&b).map_err(|e| SlackError::Other(e.to_string()))?;
                    request.header(reqwest::header::CONTENT_TYPE, "application/json; charset=utf-8").body(text)
                }
            };
            let response = request.send().await.map_err(http_error)?;
            let status = response.status();
            if status.as_u16() == 429 {
                return Err(SlackException::new(&method, "ratelimited").into());
            }
            if !status.is_success() {
                // C# EnsureSuccessStatusCode 的 HttpRequestException 訊息
                return Err(SlackError::Http(format!(
                    "Response status code does not indicate success: {} ({}).",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or("")
                )));
            }
            let text = response.text().await.map_err(http_error)?;
            let doc: Value = serde_json::from_str(&text).map_err(|e| SlackError::Other(e.to_string()))?;
            Ok(Self::check(&method, doc)?)
        })
        .await
    }

    /// Slack 一律回 HTTP 200，成功與否看 ok；失敗時回帶錯誤代碼的 [`SlackException`]。
    pub fn check(method: &str, r: Value) -> Result<Value, SlackException> {
        if r.get("ok").and_then(Value::as_bool) == Some(true) {
            return Ok(r);
        }
        let error = r.get("error").and_then(Value::as_str).unwrap_or("unknown");
        Err(SlackException::new(method, error))
    }

    /// 常見的錯誤代碼換成看得懂的說明；不認得的直接顯示代碼。
    pub fn describe(error: &str) -> String {
        match error {
            "invalid_auth" | "not_authed" => "金鑰不對，請重新複製",
            "token_revoked" | "token_expired" | "account_inactive" => {
                "金鑰已失效（App 被移除或金鑰被撤銷），請重新產生"
            }
            "not_allowed_token_type" => "金鑰種類放錯了：Bot Token 是 xoxb- 開頭，App-Level Token 是 xapp- 開頭",
            "missing_scope" => "App 少了權限，請用「複製 App 設定」的內容更新 App 後重新安裝",
            "user_not_found" | "channel_not_found" => "找不到這個成員 ID，請在 Slack 個人檔案重新複製",
            "cannot_dm_bot" => "成員 ID 是機器人，請填你自己的",
            "ratelimited" => "Slack 暫時限制傳送次數，等一下再試",
            other => other,
        }
        .to_owned()
    }
}

/// reqwest 錯誤 → C# 的 HttpRequestException／TaskCanceledException 訊息（逾時用 .NET 的文字，其他連同底層原因寫出來）。
fn http_error(err: reqwest::Error) -> SlackError {
    if err.is_timeout() {
        return SlackError::Http(
            "The request was canceled due to the configured HttpClient.Timeout of 20 seconds elapsing.".to_owned(),
        );
    }
    let mut text = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(inner) = source {
        let s = inner.to_string();
        if !text.contains(&s) {
            text.push_str(": ");
            text.push_str(&s);
        }
        source = inner.source();
    }
    SlackError::Http(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{MockServer, Reply, block_on};

    fn j(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    // ───────────── SlackTests ─────────────

    #[test]
    fn 回應ok_直接回傳() {
        let r = SlackClient::check("auth.test", j(r#"{"ok":true,"team_id":"T1"}"#)).unwrap();
        assert_eq!(Some("T1"), r["team_id"].as_str());
    }

    #[test]
    fn 回應不ok_丟出錯誤代碼() {
        let ex = SlackClient::check("auth.test", j(r#"{"ok":false,"error":"invalid_auth"}"#)).unwrap_err();
        assert_eq!("invalid_auth", ex.error);
        assert!(ex.to_string().contains("金鑰不對"));
    }

    #[test]
    fn 沒有ok欄位_當成失敗() {
        assert_eq!("unknown", SlackClient::check("x", j("{}")).unwrap_err().error);
    }

    #[test]
    fn 不認得的錯誤代碼_直接顯示() {
        assert_eq!("some_new_error", SlackClient::describe("some_new_error"));
    }

    #[test]
    fn app設定_開了socket_mode與按鈕互動_權限只要私訊() {
        let m = j(SlackClient::MANIFEST);
        assert_eq!(Some(true), m["settings"]["socket_mode_enabled"].as_bool());
        assert_eq!(Some(true), m["settings"]["interactivity"]["is_enabled"].as_bool());
        let scopes: Vec<&str> =
            m["oauth_config"]["scopes"]["bot"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
        assert_eq!(vec!["chat:write", "im:write"], scopes);
        assert_eq!(Some(true), m["features"]["app_home"]["messages_tab_enabled"].as_bool());
    }

    // ───────────── 本機假伺服器 ─────────────

    #[test]
    fn 用bearer_token送json_沒有blocks就不送() {
        let server = MockServer::start(|req| match req.path.as_str() {
            "/api/conversations.open" => Reply::json(200, r#"{"ok":true,"channel":{"id":"D1"}}"#),
            _ => Reply::json(200, r#"{"ok":true,"ts":"111.2"}"#),
        });
        let client = SlackClient::with_base_url(format!("{}/api/", server.base_url));
        let dm = block_on(client.open_dm("xoxb-1", "U1")).unwrap();
        assert_eq!("D1", dm);
        let ts = block_on(client.post_message("xoxb-1", "D1", "測試 <a>", None)).unwrap();
        assert_eq!("111.2", ts);
        let reqs = server.requests();
        assert_eq!("POST", reqs[0].method);
        assert_eq!(Some("Bearer xoxb-1"), reqs[0].headers.get("authorization").map(String::as_str));
        assert_eq!(r#"{"users":"U1"}"#, reqs[0].body);
        assert_eq!("/api/chat.postMessage", reqs[1].path);
        assert_eq!(r#"{"channel":"D1","text":"測試 <a>","unfurl_links":false}"#, reqs[1].body);
        assert!(reqs[1].headers["content-type"].starts_with("application/json"));
    }

    #[test]
    fn 沒有body_送空的表單() {
        let server = MockServer::start(|_| Reply::json(200, r#"{"ok":true,"team":"公司"}"#));
        let client = SlackClient::with_base_url(format!("{}/api/", server.base_url));
        assert_eq!("公司", block_on(client.auth_test("xoxb-1")).unwrap());
        let req = &server.requests()[0];
        assert_eq!("/api/auth.test", req.path);
        assert_eq!("", req.body);
        assert!(req.headers["content-type"].starts_with("application/x-www-form-urlencoded"));
    }

    #[test]
    fn 狀態429_當成ratelimited_ok_false_帶方法與說明() {
        let server = MockServer::start(|req| match req.path.as_str() {
            "/api/chat.update" => Reply::json(429, "{}"),
            "/api/views.open" => Reply::json(500, "{}"),
            _ => Reply::json(200, r#"{"ok":false,"error":"not_allowed_token_type"}"#),
        });
        let client = SlackClient::with_base_url(format!("{}/api/", server.base_url));
        let err = block_on(client.update_message("t", "D1", "1.2", "x", Some(json!([])))).unwrap_err();
        assert_eq!(Some("ratelimited"), err.slack_error());
        assert_eq!("chat.update：Slack 暫時限制傳送次數，等一下再試", err.message());
        let err = block_on(client.open_view("t", "tr", json!({}))).unwrap_err();
        assert_eq!("Response status code does not indicate success: 500 (Internal Server Error).", err.message());
        let err = block_on(client.open_socket_url("xoxb-1")).unwrap_err();
        assert_eq!(
            "apps.connections.open：金鑰種類放錯了：Bot Token 是 xoxb- 開頭，App-Level Token 是 xapp- 開頭",
            err.message()
        );
    }
}
