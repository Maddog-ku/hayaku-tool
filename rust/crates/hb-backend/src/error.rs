//! hb-backend 的錯誤（對應 C# 會丟出的幾種例外）。`Display` 就是 C# 的 `ex.Message`。

use std::collections::VecDeque;
use std::sync::Mutex;

/// C# 例外種類對照：
/// - `AuthRequired`：`AuthRequiredException`（需要重新登入）
/// - `Mcp`：`McpException`（伺服器回錯誤、工具失敗）
/// - `Http`：`HttpRequestException` / `TaskCanceledException`（連不上、逾時、HTTP 狀態碼不是成功）
/// - `InvalidOperation`：`InvalidOperationException`（登入流程的錯誤，例如「授權失敗：…」）
/// - `Other`：其他（回應不是 JSON、存檔失敗…）
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    AuthRequired(String),
    #[error("{0}")]
    Mcp(String),
    #[error("{0}")]
    Http(String),
    #[error("{0}")]
    InvalidOperation(String),
    #[error("{0}")]
    Other(String),
}

/// 錯誤的種類（不含訊息），給 [`error_kind`] 用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    AuthRequired,
    Mcp,
    Http,
    InvalidOperation,
    Other,
}

impl Error {
    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::AuthRequired(_) => ErrorKind::AuthRequired,
            Error::Mcp(_) => ErrorKind::Mcp,
            Error::Http(_) => ErrorKind::Http,
            Error::InvalidOperation(_) => ErrorKind::InvalidOperation,
            Error::Other(_) => ErrorKind::Other,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Error::AuthRequired(m) | Error::Mcp(m) | Error::Http(m) | Error::InvalidOperation(m) | Error::Other(m) => m,
        }
    }
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Self {
        Error::Http(http_message(&err))
    }
}

/// reqwest 錯誤連同底層原因（例如「連線被拒」）一起寫出來。
pub(crate) fn http_message(err: &reqwest::Error) -> String {
    let mut text = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(inner) = source {
        let s = inner.to_string();
        if !text.contains(&s) {
            text.push_str(": ");
            text.push_str(&s);
        }
        source = inner.source();
    }
    text
}

// ───────────── McpTransport 的錯誤種類 ─────────────
//
// hb-api 的 `McpTransport` 只回傳 `McpError(String)`，C# 那邊會依例外型別分開處理
// （`catch (AuthRequiredException)` 讓畫面顯示「未登入」、`catch (McpException)` 只接工具錯誤…）。
// 這裡記下最近轉成 `McpError` 的錯誤種類，讓上層可以用 `error_kind(&err)` 查回來。

/// 需要重新登入時的兩種訊息（C# `AuthRequiredException` 只會帶這兩種）。
pub const AUTH_REQUIRED_MESSAGES: [&str; 2] = ["需要登入 Hayaku", "登入已失效，請重新登入"];

const RECENT_MAX: usize = 64;
static RECENT: Mutex<VecDeque<(String, ErrorKind)>> = Mutex::new(VecDeque::new());

/// 把錯誤轉成 `McpError`，並記下種類。
pub(crate) fn to_mcp_error(err: Error) -> hb_api::McpError {
    let kind = err.kind();
    let message = match err {
        Error::AuthRequired(m) | Error::Mcp(m) | Error::Http(m) | Error::InvalidOperation(m) | Error::Other(m) => m,
    };
    if kind != ErrorKind::Mcp
        && kind != ErrorKind::AuthRequired
        && let Ok(mut recent) = RECENT.lock()
    {
        recent.retain(|(m, _)| m != &message);
        recent.push_back((message.clone(), kind));
        while recent.len() > RECENT_MAX {
            recent.pop_front();
        }
    }
    hb_api::McpError(message)
}

/// `McpTransport` 回傳的錯誤原本是哪一種 C# 例外。
/// 需要登入的看訊息判斷；連線／其他錯誤查最近的紀錄；都不是就當成 `McpException`。
pub fn error_kind(err: &hb_api::McpError) -> ErrorKind {
    if AUTH_REQUIRED_MESSAGES.contains(&err.0.as_str()) {
        return ErrorKind::AuthRequired;
    }
    if let Ok(recent) = RECENT.lock()
        && let Some((_, kind)) = recent.iter().rev().find(|(m, _)| m == &err.0)
    {
        return *kind;
    }
    ErrorKind::Mcp
}

/// 是不是需要重新登入（C# `catch (AuthRequiredException)`）。
pub fn is_auth_required(err: &hb_api::McpError) -> bool {
    error_kind(err) == ErrorKind::AuthRequired
}
