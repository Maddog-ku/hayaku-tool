//! 對應 C# `Auth/OAuthClient.cs`：以「已連線的應用程式」身分登入 Hayaku：OAuth 2.1 授權碼 + PKCE，
//! 動態註冊 public client，回呼走本機 127.0.0.1。拿到的 token 對象是 /mcp。
//!
//! 對外的 async 方法回傳的 future 可以在任何執行器（例如 `slint::spawn_local`）await，內部在 `rt` 的 tokio 上跑。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{TimeDelta, Utc};
use rand::RngCore as _;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::token_store::{self, AuthData};
use crate::error::{Error, http_message};
use crate::infra::app_paths::AppPaths;
use crate::infra::mini_http_server::{MiniHttpServer, MiniRequest, MiniResponse};
use crate::infra::web;
use crate::rt;

/// 本機接收登入結果的連接埠。
pub const CALLBACK_PORT: u16 = 47824;
const SCOPE: &str = "hayaku:read hayaku:write";

type Opener = Box<dyn Fn(&str) -> anyhow::Result<()> + Send + Sync>;
/// 回呼收到的 (code, error)
type CallbackResult = (Option<String>, Option<String>);
type CallbackSender = Mutex<Option<tokio::sync::oneshot::Sender<CallbackResult>>>;

/// OAuth 登入。`Clone` 很便宜（共用同一份狀態）。
#[derive(Clone)]
pub struct OAuthClient {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    http: reqwest::Client,
    base_url: String,
    /// C# `SemaphoreSlim _lock`：取 token／refresh 一次只跑一個
    lock: tokio::sync::Mutex<()>,
    data: Mutex<AuthData>,
    meta: Mutex<Option<Value>>,
    store: PathBuf,
    callback_port: u16,
    open_browser: Opener,
}

impl OAuthClient {
    /// 建立並讀取 %APPDATA%\HayakuBar\auth.bin 的登入資料。
    pub fn new(http: reqwest::Client, base_url: &str) -> Self {
        Self::with_store(http, base_url, &AppPaths::auth())
    }

    /// 同 [`new`](Self::new)，登入資料存在指定的檔案（測試用）。
    pub fn with_store(http: reqwest::Client, base_url: &str, store: &Path) -> Self {
        Self::build(http, base_url, store, CALLBACK_PORT, Box::new(|url| Ok(web::shell_open(url)?)))
    }

    pub(crate) fn build(
        http: reqwest::Client,
        base_url: &str,
        store: &Path,
        callback_port: u16,
        open_browser: Opener,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                base_url: base_url.trim_end_matches('/').to_owned(),
                lock: tokio::sync::Mutex::new(()),
                data: Mutex::new(token_store::load_from(store)),
                meta: Mutex::new(None),
                store: store.to_path_buf(),
                callback_port,
                open_browser,
            }),
        }
    }

    /// token 的對象（`{baseUrl}/mcp`）。
    pub fn resource(&self) -> String {
        self.inner.resource()
    }

    /// 有存登入資料（access token 或 refresh token）。
    pub fn has_session(&self) -> bool {
        let d = self.inner.data();
        !is_empty(&d.access_token) || !is_empty(&d.refresh_token)
    }

    /// 取得有效的 access token（快過期就用 refresh token 換新的）；都沒有時回傳「需要登入 Hayaku」。
    pub fn get_access_token(&self) -> impl Future<Output = Result<String, Error>> + Send + 'static {
        let inner = self.inner.clone();
        rt::bg(async move { inner.get_access_token().await })
    }

    /// 伺服器回 401 時呼叫：強制用 refresh token 換新的。
    pub fn force_refresh(&self) -> impl Future<Output = Result<String, Error>> + Send + 'static {
        let inner = self.inner.clone();
        rt::bg(async move { inner.force_refresh().await })
    }

    /// 開瀏覽器登入，等使用者授權（最多 5 分鐘）。
    pub fn login(&self) -> impl Future<Output = Result<(), Error>> + Send + 'static {
        let inner = self.inner.clone();
        rt::bg(async move { inner.login().await })
    }

    /// 登出：撤銷 refresh token（失敗只寫紀錄），清掉本機的 token。
    pub fn logout(&self) -> impl Future<Output = ()> + Send + 'static {
        let inner = self.inner.clone();
        rt::bg(async move { inner.logout().await })
    }
}

fn is_empty(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(str::is_empty)
}

fn other(err: impl std::fmt::Display) -> Error {
    Error::Other(err.to_string())
}

/// .NET `JsonElement.GetProperty` 找不到時的訊息。
const KEY_NOT_FOUND: &str = "The given key was not present in the dictionary.";

impl Inner {
    fn data(&self) -> std::sync::MutexGuard<'_, AuthData> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn resource(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    fn save(&self) -> Result<(), Error> {
        let data = self.data().clone();
        token_store::save_to(&self.store, &data).map_err(other)
    }

    pub(crate) async fn get_access_token(&self) -> Result<String, Error> {
        let _guard = self.lock.lock().await;
        {
            let d = self.data();
            if let Some(token) = d.access_token.as_ref().filter(|t| !t.is_empty())
                && d.expires_at > Utc::now() + TimeDelta::seconds(60)
            {
                return Ok(token.clone());
            }
        }
        if self.refresh_core().await? {
            return Ok(self.data().access_token.clone().unwrap_or_default());
        }
        Err(Error::AuthRequired("需要登入 Hayaku".into()))
    }

    pub(crate) async fn force_refresh(&self) -> Result<String, Error> {
        let _guard = self.lock.lock().await;
        if self.refresh_core().await? {
            return Ok(self.data().access_token.clone().unwrap_or_default());
        }
        self.data().access_token = None;
        self.save()?;
        Err(Error::AuthRequired("登入已失效，請重新登入".into()))
    }

    async fn login(&self) -> Result<(), Error> {
        let meta = self.meta().await?;
        let redirect = format!("http://127.0.0.1:{}/callback", self.callback_port);
        let need_register = {
            let d = self.data();
            is_empty(&d.client_id) || d.redirect_uri.as_deref() != Some(redirect.as_str())
        };
        if need_register {
            self.register(&meta, &redirect).await?;
        }

        let verifier = base64_url(&random_bytes(32));
        let challenge = base64_url(&Sha256::digest(verifier.as_bytes()));
        let state = base64_url(&random_bytes(16));
        let (tx, rx) = tokio::sync::oneshot::channel::<CallbackResult>();
        let tx = Arc::new(Mutex::new(Some(tx)));

        let expected_state = state.clone();
        let mut server = MiniHttpServer::new(self.callback_port, move |req: MiniRequest| {
            let response = callback(&req, &expected_state, &tx);
            async move { Ok(response) }
        });
        if server.start().is_err() {
            return Err(Error::InvalidOperation(format!("本機連接埠 {} 被占用，無法接收登入結果", self.callback_port)));
        }

        let result = self.login_with_server(&meta, &redirect, &verifier, &challenge, &state, rx).await;
        server.stop().await;
        result
    }

    async fn login_with_server(
        &self,
        meta: &Value,
        redirect: &str,
        verifier: &str,
        challenge: &str,
        state: &str,
        rx: tokio::sync::oneshot::Receiver<CallbackResult>,
    ) -> Result<(), Error> {
        let client_id = self.data().client_id.clone().unwrap_or_default();
        let resource = self.resource();
        let url = format!(
            "{}?{}",
            str_prop(meta, "authorization_endpoint")?,
            query(&[
                ("response_type", "code"),
                ("client_id", &client_id),
                ("redirect_uri", redirect),
                ("scope", SCOPE),
                ("state", state),
                ("code_challenge", challenge),
                ("code_challenge_method", "S256"),
                ("resource", &resource),
            ])
        );
        (self.open_browser)(&url).map_err(other)?;

        let (code, err) = match tokio::time::timeout(Duration::from_secs(5 * 60), rx).await {
            Ok(Ok(r)) => r,
            // 逾時（C# TaskCanceledException）
            _ => return Err(Error::Other("A task was canceled.".into())),
        };
        let code = match (err, code) {
            (None, Some(code)) if !code.is_empty() => code,
            (err, _) => {
                return Err(Error::InvalidOperation(format!(
                    "授權失敗：{}",
                    err.unwrap_or_else(|| "沒有收到授權碼".into())
                )));
            }
        };

        let form = [
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier),
            ("resource", resource.as_str()),
        ];
        let res = self.http.post(str_prop(meta, "token_endpoint")?).form(&form).send().await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            if body.contains("invalid_client") {
                // 伺服器端已撤銷這個應用程式，下次重新註冊
                *self.data() = AuthData::default();
                self.save()?;
            }
            return Err(Error::InvalidOperation(format!("換取 token 失敗（{}）：{body}", status.as_u16())));
        }
        self.apply_token(&body)?;
        log::info!("登入成功");
        Ok(())
    }

    async fn logout(&self) {
        let revoke = async {
            let (refresh, client_id) = {
                let d = self.data();
                (d.refresh_token.clone().unwrap_or_default(), d.client_id.clone().unwrap_or_default())
            };
            if !refresh.is_empty() && !client_id.is_empty() {
                let meta = self.meta().await?;
                let form = [
                    ("token", refresh.as_str()),
                    ("token_type_hint", "refresh_token"),
                    ("client_id", client_id.as_str()),
                ];
                let _ = self.http.post(str_prop(&meta, "revocation_endpoint")?).form(&form).send().await?;
            }
            Ok::<_, Error>(())
        };
        if let Err(err) = revoke.await {
            log::error!("撤銷 token 失敗: {err}");
        }
        {
            let mut d = self.data();
            d.access_token = None;
            d.refresh_token = None;
        }
        if let Err(err) = self.save() {
            log::error!("儲存登入資料失敗: {err}");
        }
    }

    async fn refresh_core(&self) -> Result<bool, Error> {
        let (refresh, client_id) = {
            let d = self.data();
            if is_empty(&d.refresh_token) || is_empty(&d.client_id) {
                return Ok(false);
            }
            (d.refresh_token.clone().unwrap_or_default(), d.client_id.clone().unwrap_or_default())
        };
        let meta = self.meta().await?;
        let resource = self.resource();
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.as_str()),
            ("client_id", client_id.as_str()),
            ("resource", resource.as_str()),
        ];
        let res = self.http.post(str_prop(&meta, "token_endpoint")?).form(&form).send().await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            log::error!("refresh 失敗（{}）：{body}", status.as_u16());
            if matches!(status.as_u16(), 400 | 401) {
                {
                    let mut d = self.data();
                    d.access_token = None;
                    d.refresh_token = None;
                }
                self.save()?;
            }
            return Ok(false);
        }
        self.apply_token(&body)?;
        Ok(true)
    }

    async fn register(&self, meta: &Value, redirect: &str) -> Result<(), Error> {
        let payload = serde_json::json!({
            "client_name": "HayakuBar 工時小工具",
            "redirect_uris": [redirect],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "application_type": "native",
            "scope": SCOPE,
        });
        let res = self
            .http
            .post(str_prop(meta, "registration_endpoint")?)
            .header(reqwest::header::CONTENT_TYPE, "application/json; charset=utf-8")
            .body(serde_json::to_vec(&payload).map_err(other)?)
            .send()
            .await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            return Err(Error::InvalidOperation(format!("註冊應用程式失敗（{}）：{body}", status.as_u16())));
        }
        let doc: Value = serde_json::from_str(&body).map_err(other)?;
        let client_id = match doc.get("client_id") {
            None => return Err(Error::Other(KEY_NOT_FOUND.into())),
            Some(v) => v.as_str().map(str::to_owned),
        };
        *self.data() =
            AuthData { client_id: client_id.clone(), redirect_uri: Some(redirect.to_owned()), ..AuthData::default() };
        self.save()?;
        log::info!("已註冊應用程式 {}", client_id.unwrap_or_default());
        Ok(())
    }

    fn apply_token(&self, body: &str) -> Result<(), Error> {
        let root: Value = serde_json::from_str(body).map_err(other)?;
        let access = match root.get("access_token") {
            None => return Err(Error::Other(KEY_NOT_FOUND.into())),
            Some(v) => v.as_str().map(str::to_owned),
        };
        let seconds =
            root.get("expires_in").and_then(Value::as_i64).filter(|s| i32::try_from(*s).is_ok()).unwrap_or(3600);
        {
            let mut d = self.data();
            d.access_token = access;
            if let Some(rt) = root.get("refresh_token").and_then(Value::as_str) {
                d.refresh_token = Some(rt.to_owned());
            }
            d.expires_at = (Utc::now() + TimeDelta::seconds(seconds)).fixed_offset();
        }
        self.save()
    }

    async fn meta(&self) -> Result<Value, Error> {
        if let Some(cached) = self.meta.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(cached);
        }
        let res = self.http.get(format!("{}/.well-known/oauth-authorization-server", self.base_url)).send().await?;
        let status = res.status();
        if !status.is_success() {
            // C# GetStringAsync 的 HttpRequestException 訊息
            return Err(Error::Http(format!(
                "Response status code does not indicate success: {} ({}).",
                status.as_u16(),
                status.canonical_reason().unwrap_or("")
            )));
        }
        let json = res.text().await.map_err(|e| Error::Http(http_message(&e)))?;
        let meta: Value = serde_json::from_str(&json).map_err(other)?;
        *self.meta.lock().unwrap_or_else(|e| e.into_inner()) = Some(meta.clone());
        Ok(meta)
    }
}

/// 處理 /callback：state 對得上才把結果交給登入流程。
fn callback(req: &MiniRequest, state: &str, tx: &CallbackSender) -> MiniResponse {
    if req.path != "/callback" {
        return MiniResponse::new(404, "text/plain", "not found");
    }
    let q = parse_query(&req.query);
    if q.get("state") != Some(state) {
        return page("登入失敗", "state 不符，請回到 HayakuBar 重新登入。");
    }
    let error = q.get("error").map(str::to_owned);
    let detail = error.as_ref().map(|e| format!("{e} {}", q.get("error_description").unwrap_or("")).trim().to_owned());
    if let Some(tx) = tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = tx.send((q.get("code").map(str::to_owned), detail));
    }
    match error {
        None => page("登入完成", "可以關閉這個分頁，回到 HayakuBar。"),
        Some(e) => page("登入失敗", &e),
    }
}

/// C# `HttpUtility.ParseQueryString`：解碼（`+` 是空白），同名的值用逗號串起來；沒有 `=` 的片段不算有名字。
pub(crate) struct Query(Vec<(String, String)>);

impl Query {
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub(crate) fn parse_query(query: &str) -> Query {
    let query = query.strip_prefix('?').unwrap_or(query);
    let mut items: Vec<(String, String)> = Vec::new();
    for segment in query.split('&') {
        if !segment.contains('=') {
            continue;
        }
        let Some((k, v)) = url::form_urlencoded::parse(segment.as_bytes()).next() else {
            continue;
        };
        match items.iter_mut().find(|(name, _)| name.as_str() == k) {
            Some((_, existing)) => {
                existing.push(',');
                existing.push_str(&v);
            }
            None => items.push((k.into_owned(), v.into_owned())),
        }
    }
    Query(items)
}

fn str_prop(meta: &Value, name: &str) -> Result<String, Error> {
    meta.get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::InvalidOperation(format!("OAuth 設定缺少 {name}")))
}

/// `Uri.EscapeDataString` 串成查詢字串。
fn query(values: &[(&str, &str)]) -> String {
    values
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; n];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

fn base64_url(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// C# `HttpUtility.HtmlEncode`。
fn html_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '&' => out.push_str("&amp;"),
            '\u{a0}'..='\u{ff}' => out.push_str(&format!("&#{};", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

fn page(title: &str, message: &str) -> MiniResponse {
    MiniResponse::new(
        200,
        "text/html",
        format!(
            "<!doctype html><html lang=\"zh-Hant-TW\"><meta charset=\"utf-8\"><title>{title}</title>\
             <body style=\"font-family:system-ui,'Microsoft JhengHei';background:#f1eee8;color:#2b2925;display:grid;place-items:center;height:100vh;margin:0\">\
             <div style=\"text-align:center\"><h2>{}</h2><p>{}</p></div></body></html>",
            html_encode(title),
            html_encode(message)
        ),
    )
}

#[cfg(test)]
pub(crate) mod tests;
