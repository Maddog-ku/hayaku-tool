//! 基礎層（對應 C# `Infra/`、`Config/`、`Auth/`、`Mcp/McpClient.cs`、`Mcp/McpStats.cs`）：
//! 檔案路徑與設定、紀錄、清暫存、DPAPI、本機 HTTP 伺服器、OAuth 登入、MCP JSON-RPC 呼叫、tokio 執行環境。
//!
//! 典型用法（對應 `App.xaml.cs`）：
//! ```no_run
//! use hb_backend::{auth::OAuthClient, config::{AppConfig, AppState}, mcp::McpClient};
//! hb_backend::infra::log::init();
//! let config = AppConfig::load();
//! let state = AppState::load();
//! let http = hb_backend::http::app_client();
//! let auth = OAuthClient::new(http.clone(), &config.base_url);
//! let mcp = McpClient::new(http, auth.clone(), &config.base_url); // 實作 hb_api::McpTransport
//! ```

pub mod auth;
pub mod config;
pub mod error;
pub mod http;
pub mod infra;
pub mod mcp;
pub mod rt;

pub use error::{Error, ErrorKind, error_kind, is_auth_required};

#[cfg(test)]
mod test_support;
