//! `HayakuApi` 與 `McpClient` 之間的介面（對應 C# `McpClient.CallToolAsync` / `CallToolRawAsync`）。
//! 由 hb-backend 的 `McpClient` 實作；測試可以自己實作一個假的。

use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 對應 C# `McpException`：伺服器回了錯誤、工具執行失敗（isError）、或 HTTP／連線失敗。
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct McpError(pub String);

pub trait McpTransport: Send + Sync {
    /// `tools/call`，回傳工具結果解析後的 JSON（C# `CallToolAsync`：content[0].text 是 JSON 就解析，否則包成字串）。
    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        arguments: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, McpError>>;

    /// `tools/call`，回傳原始 result（C# `CallToolRawAsync`）。
    fn call_tool_raw<'a>(
        &'a self,
        name: &'a str,
        arguments: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, McpError>>;
}
