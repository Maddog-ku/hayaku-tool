//! 對應 C# `Eip/EipClient.cs`。
//!
//! 公司 EIP（eip.yanray.com.tw，PHP 網站）。沒有 API，用帳密登入表單拿 PHPSESSID，之後帶 cookie 抓頁面。
//! 登入分兩段：先送 account、password（加上登入頁的隱藏欄位 csrf_token、state），
//! 再送「登入身分選擇」表單（頁面本來會自動送出，帶 person_no＝工號）。
//!
//! 網路 IO 在 hb-backend 的 tokio 執行環境跑，對外的 future 可以在任何執行器（`slint::spawn_local`）await；
//! 丟棄 future 就是取消（C# 的 CancellationToken）。網頁編碼照 Content-Type 的 charset，沒寫就是 UTF-8（跟 .NET 一樣）。

use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use chrono::NaiveDateTime;
use hb_backend::config::IndexMap;
use regex::Regex;
use url::Url;

use crate::html::html_decode;

/// 登入結果：帳密錯誤是 ok = false。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EipLoginResult {
    pub ok: bool,
    pub message: String,
}

impl EipLoginResult {
    fn new(ok: bool, message: impl Into<String>) -> Self {
        Self { ok, message: message.into() }
    }
}

/// EIP「假單」清單的一列。kind 是性質（請假、銷假、補假、黃卡），type 是假別（特休、事假…），subtotal 是小計（1天0小時）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EipLeave {
    pub no: String,
    pub person_no: String,
    pub kind: String,
    pub r#type: String,
    pub reason: String,
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
    pub status: String,
    pub id: Option<String>,
    pub subtotal: String,
}

impl EipLeave {
    /// C# 建構子（subtotal 預設空字串）。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        no: impl Into<String>,
        person_no: impl Into<String>,
        kind: impl Into<String>,
        r#type: impl Into<String>,
        reason: impl Into<String>,
        start: NaiveDateTime,
        end: NaiveDateTime,
        status: impl Into<String>,
        id: Option<String>,
    ) -> Self {
        Self {
            no: no.into(),
            person_no: person_no.into(),
            kind: kind.into(),
            r#type: r#type.into(),
            reason: reason.into(),
            start,
            end,
            status: status.into(),
            id,
            subtotal: String::new(),
        }
    }
}

/// EipClient 的錯誤（`Display` 就是 C# 的 `ex.Message`）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EipError {
    /// C# `EipLoginException`：帳密錯誤。
    #[error("{0}")]
    Login(String),
    /// C# `HttpRequestException` / `TaskCanceledException`：連不上、逾時、HTTP 狀態碼不是成功。
    #[error("{0}")]
    Http(String),
    /// C# `InvalidOperationException`：頁面格式跟預期不同。
    #[error("{0}")]
    InvalidOperation(String),
}

impl From<reqwest::Error> for EipError {
    fn from(err: reqwest::Error) -> Self {
        // 連同底層原因（例如「連線被拒」）一起寫出來
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
        EipError::Http(text)
    }
}

pub const BASE_URI: &str = "https://eip.yanray.com.tw/";
const LOGIN_PATH: &str = "plug/home/php/login.php?get_url=";
const LEAVE_LIST_PATH: &str = "plug/absentNext/php/leave_list.php";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) HayakuBar";

/// 表單欄位（照加入順序送出，同名的後面蓋前面，跟 C# Dictionary 一樣）。
pub type FormFields = IndexMap<String, String>;

#[derive(Default)]
struct State {
    logged_in: bool,
    person_no: Option<String>,
}

struct Inner {
    http: reqwest::Client,
    base: Url,
    state: Mutex<State>,
}

/// EIP 連線（含 cookie）。可以 clone，clone 出來的共用同一個 session。
#[derive(Clone)]
pub struct EipClient {
    inner: Arc<Inner>,
}

impl Default for EipClient {
    fn default() -> Self {
        Self::new()
    }
}

impl EipClient {
    /// 假單清單的狀態篩選（absent_state）。
    pub const STATE_PENDING: i32 = 0;
    pub const STATE_APPROVED: i32 = 1;
    pub const STATE_REJECTED: i32 = 2;

    pub fn new() -> Self {
        Self::with_base(BASE_URI)
    }

    /// 連到別的網址（測試用本機假伺服器）。
    pub fn with_base(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .cookie_store(true)
            .timeout(Duration::from_secs(20))
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_else(|err| {
                log::error!("建立 EIP HttpClient 失敗: {err}");
                reqwest::Client::new()
            });
        let base = Url::parse(base).or_else(|_| Url::parse(BASE_URI)).expect("固定的網址");
        Self { inner: Arc::new(Inner { http, base, state: Mutex::new(State::default()) }) }
    }

    pub fn logged_in(&self) -> bool {
        self.inner.state().logged_in
    }

    /// 登入後的工號（例 Y2798），用來只挑自己的假單。
    pub fn person_no(&self) -> Option<String> {
        self.inner.state().person_no.clone()
    }

    /// 用帳密登入；帳密錯誤回 ok = false，連不上 EIP 回錯誤。
    pub async fn login(&self, cred: &crate::EipCredentials) -> Result<EipLoginResult, EipError> {
        let inner = self.inner.clone();
        let cred = cred.clone();
        hb_backend::rt::bg(async move { inner.login(&cred).await }).await
    }

    /// 某一年、某個狀態的假單（自己的，主管會連部屬的一起列出，用 person_no 過濾）。
    pub async fn list_leaves(
        &self,
        year: i32,
        state: i32,
        cred: &crate::EipCredentials,
    ) -> Result<Vec<EipLeave>, EipError> {
        let inner = self.inner.clone();
        let cred = cred.clone();
        hb_backend::rt::bg(async move { inner.list_leaves(year, state, &cred).await }).await
    }

    // ───────────── 解析（給測試用） ─────────────

    /// 表單裡的隱藏欄位（name → value），value 照原樣送回去，跟瀏覽器一樣。
    pub fn hidden_fields(html: &str) -> FormFields {
        inputs(html, false)
    }

    /// 「登入身分選擇」表單要送的欄位：隱藏欄位＋已勾選的 radio。
    pub fn identity_fields(html: &str) -> FormFields {
        inputs(html, true)
    }

    /// 頁面上有密碼欄位就是還在登入頁（登入失敗或 session 過期）。
    pub fn is_login_form(html: &str) -> bool {
        INPUT_TAG
            .find_iter(html)
            .any(|m| attributes(m.as_str()).get("type").is_some_and(|t| t.eq_ignore_ascii_case("password")))
    }

    /// session 已經失效、要重新登入：回的是登入頁，或整頁只有一段 script 把頁面導去 login.php
    /// （alert('請登入'); location.href='/plug/home/php/login.php?…'）。
    /// 登入後的正常假單頁裡也有導去 login.php 的 script，所以要「除了 script 沒有別的內容」才算。
    pub fn needs_login(html: &str) -> bool {
        Self::is_login_form(html)
            || (LOGIN_REDIRECT.is_match(html) && SCRIPT_BLOCK.replace_all(html, "").trim().is_empty())
    }

    /// PHP 常用 alert('…') 回錯誤訊息。
    pub fn alert_text(html: &str) -> Option<String> {
        let m = ALERT.captures(html)?;
        let msg = m.name("a").or_else(|| m.name("b"))?.as_str().trim();
        (!msg.is_empty()).then(|| msg.to_owned())
    }

    /// 解析「假單」清單（leave_list.php）的表格；欄位用表頭文字對應，沒有起訖時間的列（黃卡）略過。
    pub fn parse_leave_list(html: &str) -> Vec<EipLeave> {
        let mut list = Vec::new();
        let mut headers: Option<Vec<String>> = None;
        for row in ROW.captures_iter(html) {
            let (Some(all), Some(inner)) = (row.get(0), row.name("row")) else { continue };
            let cells: Vec<&str> =
                CELL.captures_iter(inner.as_str()).filter_map(|c| c.name("cell").map(|m| m.as_str())).collect();
            let Some(headers) = headers.as_ref() else {
                let texts: Vec<String> = cells.iter().map(|c| text(c)).collect();
                if texts.iter().any(|t| t == "單號") && texts.iter().any(|t| t == "請假時間(起)") {
                    headers = Some(texts);
                }
                continue;
            };
            let col = |name: &str| -> String {
                match headers.iter().position(|h| h == name) {
                    Some(i) if i < cells.len() => text(cells[i]),
                    _ => String::new(),
                }
            };

            let (Some(start), Some(end)) = (parse_time(&col("請假時間(起)")), parse_time(&col("請假時間(迄)")))
            else {
                continue;
            };
            if end <= start {
                continue;
            }
            let id = ROW_ID.captures(all.as_str()).and_then(|m| m.name("id")).map(|m| m.as_str().to_owned());
            list.push(EipLeave {
                no: col("單號"),
                person_no: col("工號"),
                kind: col("類別"),
                r#type: col("假別"),
                reason: col("請假原因"),
                start,
                end,
                status: col("狀態"),
                id,
                subtotal: col("小計"),
            });
        }
        list
    }
}

impl Inner {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn url(&self, path: &str) -> Result<Url, EipError> {
        self.base.join(path).map_err(|e| EipError::Http(e.to_string()))
    }

    async fn get_string(&self, path: &str) -> Result<String, EipError> {
        let res = self.http.get(self.url(path)?).send().await?.error_for_status()?;
        Ok(res.text().await?)
    }

    async fn post_form(&self, path: &str, fields: &FormFields) -> Result<String, EipError> {
        let pairs: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let res = self.http.post(self.url(path)?).form(&pairs).send().await?.error_for_status()?;
        Ok(res.text().await?)
    }

    async fn login(&self, cred: &crate::EipCredentials) -> Result<EipLoginResult, EipError> {
        self.state().logged_in = false;
        let form = self.get_string(LOGIN_PATH).await?;
        let mut fields = EipClient::hidden_fields(&form);
        if !fields.contains_key("csrf_token") {
            return Ok(EipLoginResult::new(false, "EIP 登入頁的格式跟預期不同，可能改版了"));
        }
        fields.insert("account".into(), cred.account.clone());
        fields.insert("password".into(), cred.password.clone());

        let mut html = self.post_form(LOGIN_PATH, &fields).await?;
        if EipClient::is_login_form(&html) {
            let alert = EipClient::alert_text(&html);
            log::info!("EIP 登入失敗{}", alert.as_ref().map(|a| format!("：{a}")).unwrap_or_default());
            return Ok(EipLoginResult::new(false, alert.unwrap_or_else(|| "帳號或密碼錯誤".into())));
        }

        // 第二段：登入身分選擇（瀏覽器會自動送出）
        let identity = EipClient::identity_fields(&html);
        if let Some(person_no) = identity.get("person_no") {
            self.state().person_no = Some(person_no.clone());
            html = self.post_form(LOGIN_PATH, &identity).await?;
            if EipClient::is_login_form(&html) {
                return Ok(EipLoginResult::new(
                    false,
                    EipClient::alert_text(&html).unwrap_or_else(|| "EIP 登入身分選擇失敗".into()),
                ));
            }
        }
        let person_no = {
            let mut state = self.state();
            state.logged_in = true;
            state.person_no.clone()
        };
        log::info!("EIP 登入成功（工號 {}）", person_no.as_deref().unwrap_or("未知"));
        Ok(EipLoginResult::new(true, "登入成功"))
    }

    async fn list_leaves(
        &self,
        year: i32,
        state: i32,
        cred: &crate::EipCredentials,
    ) -> Result<Vec<EipLeave>, EipError> {
        let mut fields = FormFields::new();
        fields.insert("years".into(), year.to_string());
        fields.insert("selectDep".into(), String::new());
        fields.insert("selectPerson".into(), String::new());
        fields.insert("absent_state".into(), state.to_string());
        fields.insert("btnSearch".into(), String::new());
        let html = self.with_login(LEAVE_LIST_PATH, &fields, cred).await?;
        if !html.contains("請假時間(起)") {
            return Err(EipError::InvalidOperation("EIP 假單頁的格式跟預期不同，可能改版了".into()));
        }
        let leaves = EipClient::parse_leave_list(&html);
        let me = self.state().person_no.clone();
        Ok(match me.filter(|m| !m.is_empty()) {
            Some(me) => leaves.into_iter().filter(|l| l.person_no.to_uppercase() == me.to_uppercase()).collect(),
            None => leaves,
        })
    }

    /// 還沒登入就先登入；session 過期（回登入頁，或只回一段導去登入頁的 script）時重新登入一次再試。
    async fn with_login(
        &self,
        path: &str,
        fields: &FormFields,
        cred: &crate::EipCredentials,
    ) -> Result<String, EipError> {
        if !self.state().logged_in && !self.login(cred).await?.ok {
            return Err(EipError::Login("EIP 帳號或密碼錯誤".into()));
        }
        let html = self.post_form(path, fields).await?;
        if !EipClient::needs_login(&html) {
            return Ok(html);
        }
        if !self.login(cred).await?.ok {
            return Err(EipError::Login("EIP 帳號或密碼錯誤".into()));
        }
        self.post_form(path, fields).await
    }
}

// ───────────── regex ─────────────

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("固定的 regex")
}

static INPUT_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)<input\b[^>]*>"));

/// C# 三個選項共用一個名稱 v；Rust 分成 v1～v3。
static ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)\b(?P<name>[\w-]+)\s*=\s*(?:'(?P<v1>[^']*)'|"(?P<v2>[^"]*)"|(?P<v3>[^\s>]+))"#));

static CHECKED: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\schecked\b"));

/// C# `alert\s*\(\s*(['"])(?<msg>.*?)\1\s*\)`：反向參考改成兩個選項。
static ALERT: LazyLock<Regex> = LazyLock::new(|| re(r#"(?is)alert\s*\(\s*(?:'(?P<a>.*?)'|"(?P<b>.*?)")\s*\)"#));

static SCRIPT_BLOCK: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<script\b[^>]*>.*?</script>"));

static LOGIN_REDIRECT: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)location(?:\.href)?\s*(?:=|\.(?:replace|assign)\s*\()\s*['"][^'"]*login\.php"#));

static ROW: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<tr\b[^>]*>(?P<row>.*?)</tr>"));

static CELL: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)<t[dh]\b[^>]*>(?P<cell>.*?)</t[dh]>"));

static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]+>"));

static ROW_ID: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)[?&]ID=(?P<id>\d+)"));

/// 標籤的屬性（名稱轉小寫，同名取第一個，值做 HtmlDecode）。
fn attributes(tag: &str) -> IndexMap<String, String> {
    let mut map = IndexMap::new();
    for m in ATTRIBUTE.captures_iter(tag) {
        let Some(name) = m.name("name") else { continue };
        let key = name.as_str().to_lowercase();
        if map.contains_key(&key) {
            continue;
        }
        let value = m.name("v1").or_else(|| m.name("v2")).or_else(|| m.name("v3")).map_or("", |v| v.as_str());
        map.insert(key, html_decode(value));
    }
    map
}

fn text(html: &str) -> String {
    html_decode(&TAG.replace_all(html, "")).trim().to_owned()
}

fn inputs(html: &str, radios: bool) -> FormFields {
    let mut fields = FormFields::new();
    for m in INPUT_TAG.find_iter(html) {
        let a = attributes(m.as_str());
        let ty = a.get("type").map(|t| t.to_lowercase());
        let wanted = ty.as_deref() == Some("hidden")
            || (radios && ty.as_deref() == Some("radio") && CHECKED.is_match(m.as_str()));
        if wanted && let Some(name) = a.get("name").filter(|n| !n.is_empty()) {
            fields.insert(name.clone(), a.get("value").cloned().unwrap_or_default());
        }
    }
    fields
}

/// `DateTime.TryParseExact(text, ["yyyy-MM-dd HH:mm", "yyyy-MM-dd HH:mm:ss", "yyyy/MM/dd HH:mm"])`。
fn parse_time(text: &str) -> Option<NaiveDateTime> {
    static EXACT: LazyLock<Regex> = LazyLock::new(|| {
        re(
            r"^(?:[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}(?::[0-9]{2})?|[0-9]{4}/[0-9]{2}/[0-9]{2} [0-9]{2}:[0-9]{2})$",
        )
    });
    if !EXACT.is_match(text) {
        return None;
    }
    ["%Y-%m-%d %H:%M", "%Y-%m-%d %H:%M:%S", "%Y/%m/%d %H:%M"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(text, f).ok())
}
