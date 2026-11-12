//! hb-core 的錯誤（對應 C# 在 Core 裡丟出或往外傳的幾種例外）。`Display` 就是 C# 的 `ex.Message`。

use hb_api::McpError;
use hb_backend::{ErrorKind, error_kind};

/// - `Mcp`：HayakuApi 回來的錯誤（`McpException`；實際種類用 [`CoreError::kind`] 查，可能是需要登入、連不上…）
/// - `InvalidOperation`：`InvalidOperationException`（例「這筆申請已經處理過或被撤回了」、EIP 頁面改版）
/// - `EipLogin`：`EipLoginException`（EIP 帳密錯誤、還沒設定帳密）
/// - `Http`：連不上（EIP 的 `HttpRequestException`／`TaskCanceledException`）
/// - `Other`：其他例外
#[derive(Debug, Clone, thiserror::Error)]
pub enum CoreError {
    #[error("{0}")]
    Mcp(#[from] McpError),
    #[error("{0}")]
    InvalidOperation(String),
    #[error("{0}")]
    EipLogin(String),
    #[error("{0}")]
    Http(String),
    #[error("{0}")]
    Other(String),
}

impl CoreError {
    /// C# 丟 `new McpException(message)` 的地方。
    pub fn mcp(message: impl Into<String>) -> Self {
        CoreError::Mcp(McpError(message.into()))
    }

    pub fn message(&self) -> &str {
        match self {
            CoreError::Mcp(e) => &e.0,
            CoreError::InvalidOperation(m) | CoreError::EipLogin(m) | CoreError::Http(m) | CoreError::Other(m) => m,
        }
    }

    /// 對應的 C# 例外種類。
    pub fn kind(&self) -> ErrorKind {
        match self {
            CoreError::Mcp(e) => error_kind(e),
            CoreError::InvalidOperation(_) => ErrorKind::InvalidOperation,
            CoreError::Http(_) => ErrorKind::Http,
            CoreError::EipLogin(_) | CoreError::Other(_) => ErrorKind::Other,
        }
    }

    /// `catch (AuthRequiredException)`。
    pub fn is_auth_required(&self) -> bool {
        self.kind() == ErrorKind::AuthRequired
    }

    /// `catch (McpException)`。
    pub fn is_mcp(&self) -> bool {
        self.kind() == ErrorKind::Mcp
    }

    /// `ex is McpException or HttpRequestException or TaskCanceledException`。
    pub fn is_transient(&self) -> bool {
        matches!(self.kind(), ErrorKind::Mcp | ErrorKind::Http)
    }

    /// `ex is HttpRequestException or TaskCanceledException`。
    pub fn is_http(&self) -> bool {
        self.kind() == ErrorKind::Http
    }
}

/// `catch (McpException)`：HayakuApi 的錯誤只接工具錯誤（需要登入、連不上的往外傳）。
pub fn is_mcp(e: &McpError) -> bool {
    error_kind(e) == ErrorKind::Mcp
}

/// `ex is McpException or HttpRequestException or TaskCanceledException`。
pub fn is_transient(e: &McpError) -> bool {
    matches!(error_kind(e), ErrorKind::Mcp | ErrorKind::Http)
}

impl From<hb_eip::EipError> for CoreError {
    fn from(err: hb_eip::EipError) -> Self {
        match err {
            hb_eip::EipError::Login(m) => CoreError::EipLogin(m),
            hb_eip::EipError::Http(m) => CoreError::Http(m),
            hb_eip::EipError::InvalidOperation(m) => CoreError::InvalidOperation(m),
        }
    }
}
