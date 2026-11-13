//! 對應 C# `Mcp/McpClient.cs`：Hayaku /mcp 的 JSON-RPC 呼叫（無狀態、只接受 POST）。
//!
//! 實作 hb-api 的 [`McpTransport`]；對外的 async 方法回傳的 future 可以在任何執行器 await（內部在 `rt` 的 tokio 上跑）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use hb_api::{BoxFuture, McpError, McpTransport};
use serde_json::{Value, json};

use super::mcp_stats;
use crate::auth::oauth_client::OAuthClient;
use crate::error::{Error, to_mcp_error};
use crate::rt;

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// `Clone` 很便宜（共用同一份狀態）。
#[derive(Clone)]
pub struct McpClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    auth: OAuthClient,
    endpoint: String,
    id: AtomicI32,
    initialized: AtomicBool,
}

impl McpClient {
    pub fn new(http: reqwest::Client, auth: OAuthClient, base_url: &str) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                auth,
                endpoint: format!("{}/mcp", base_url.trim_end_matches('/')),
                id: AtomicI32::new(0),
                initialized: AtomicBool::new(false),
            }),
        }
    }

    /// 呼叫 MCP 工具，回傳工具結果（structuredContent，或把文字內容當 JSON 解析；不是 JSON 就是字串；沒有內容是 null）。
    pub fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> impl Future<Output = Result<Value, Error>> + Send + 'static {
        let inner = self.inner.clone();
        let name = name.to_owned();
        rt::bg(async move { inner.call_tool(&name, arguments).await })
    }

    /// 呼叫 MCP 工具，回傳整個結果（content 陣列與 structuredContent 都在）；圖片這類不是 JSON 的內容（read_file）用。
    pub fn call_tool_raw(
        &self,
        name: &str,
        arguments: Value,
    ) -> impl Future<Output = Result<Value, Error>> + Send + 'static {
        let inner = self.inner.clone();
        let name = name.to_owned();
        rt::bg(async move { inner.call_tool_raw(&name, arguments).await })
    }

    /// 列出伺服器提供的所有工具（診斷用）。
    pub fn list_tools(&self) -> impl Future<Output = Result<Value, Error>> + Send + 'static {
        let inner = self.inner.clone();
        rt::bg(async move { inner.rpc("tools/list", json!({}), None).await })
    }

    pub fn auth(&self) -> &OAuthClient {
        &self.inner.auth
    }
}

impl McpTransport for McpClient {
    fn call_tool<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, McpError>> {
        let fut = McpClient::call_tool(self, name, arguments);
        Box::pin(async move { fut.await.map_err(to_mcp_error) })
    }

    fn call_tool_raw<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, McpError>> {
        let fut = McpClient::call_tool_raw(self, name, arguments);
        Box::pin(async move { fut.await.map_err(to_mcp_error) })
    }
}

fn tool_params(name: &str, arguments: Value) -> Value {
    json!({ "name": name, "arguments": if arguments.is_null() { json!({}) } else { arguments } })
}

impl Inner {
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, Error> {
        if !self.initialized.load(Ordering::SeqCst) {
            let params = json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "HayakuBar", "version": "1.0.0" },
            });
            match self.rpc("initialize", params, None).await {
                Ok(_) => {}
                Err(err @ Error::AuthRequired(_)) => return Err(err),
                Err(err) => log::info!("initialize 失敗（無狀態伺服器可忽略）：{err}"),
            }
            self.initialized.store(true, Ordering::SeqCst);
        }

        let result = self.rpc("tools/call", tool_params(name, arguments), Some(name)).await?;
        let text = extract_text(&result)?;
        if result.get("isError") == Some(&Value::Bool(true)) {
            return Err(Error::Mcp(text.unwrap_or_else(|| format!("{name} 失敗"))));
        }
        if let Some(structured) = result.get("structuredContent").filter(|v| v.is_object()) {
            return Ok(structured.clone());
        }
        let Some(text) = text else {
            return Ok(Value::Null);
        };
        Ok(serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)))
    }

    async fn call_tool_raw(&self, name: &str, arguments: Value) -> Result<Value, Error> {
        let result = self.rpc("tools/call", tool_params(name, arguments), Some(name)).await?;
        if result.get("isError") == Some(&Value::Bool(true)) {
            return Err(Error::Mcp(extract_text(&result)?.unwrap_or_else(|| format!("{name} 失敗"))));
        }
        Ok(result)
    }

    /// stat_name：統計請求數用的名稱（tools/call 傳工具名稱），預設用 method。
    async fn rpc(&self, method: &str, params: Value, stat_name: Option<&str>) -> Result<Value, Error> {
        let auth = &self.auth.inner;
        let mut attempt = 0;
        loop {
            let token = if attempt == 0 { auth.get_access_token().await? } else { auth.force_refresh().await? };
            mcp_stats::record(stat_name.unwrap_or(method));
            let id = self.id.fetch_add(1, Ordering::SeqCst) + 1;
            let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
            let response = self
                .http
                .post(&self.endpoint)
                .bearer_auth(&token)
                .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
                .header("MCP-Protocol-Version", PROTOCOL_VERSION)
                .header(reqwest::header::CONTENT_TYPE, "application/json; charset=utf-8")
                .body(serde_json::to_vec(&body).map_err(|e| Error::Other(e.to_string()))?)
                .send()
                .await?;
            let status = response.status();
            let is_sse = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or("").trim() == "text/event-stream")
                .unwrap_or(false);
            let text = response.text().await?;
            if status == reqwest::StatusCode::UNAUTHORIZED {
                if attempt == 0 {
                    attempt += 1;
                    continue;
                }
                return Err(Error::AuthRequired("登入已失效，請重新登入".into()));
            }
            if !status.is_success() {
                return Err(Error::Mcp(format!("Hayaku 回應 {}：{}", status.as_u16(), shorten(&error_message(&text)))));
            }

            let json = if is_sse { from_sse(&text)? } else { text };
            let root: Value = serde_json::from_str(&json).map_err(|e| Error::Other(e.to_string()))?;
            if let Some(error) = root.get("error") {
                let message = error.get("message").and_then(Value::as_str).unwrap_or("MCP 錯誤");
                return Err(Error::Mcp(message.to_owned()));
            }
            return match root.get("result") {
                Some(result) => Ok(result.clone()),
                None => Err(Error::Other("The given key was not present in the dictionary.".into())),
            };
        }
    }
}

/// content 陣列裡第一個 type 是 text 的內容。
fn extract_text(result: &Value) -> Result<Option<String>, Error> {
    let Some(content) = result.get("content").and_then(Value::as_array) else {
        return Ok(None);
    };
    for item in content {
        if item.get("type").and_then(Value::as_str) == Some("text") {
            return match item.get("text") {
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Ok(None),
                None => Err(Error::Other("The given key was not present in the dictionary.".into())),
            };
        }
    }
    Ok(None)
}

/// SSE 回應：取最後一個帶 jsonrpc 的 data 行。
fn from_sse(body: &str) -> Result<String, Error> {
    let mut last = None;
    for line in body.split('\n') {
        if line.starts_with("data:") && line.contains("\"jsonrpc\"") {
            last = Some(line[5..].trim().to_owned());
        }
    }
    last.ok_or_else(|| Error::Mcp("MCP 回應沒有資料".into()))
}

fn error_message(body: &str) -> String {
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(body)
        && let Some(Value::String(s)) = map.get("message")
    {
        return s.clone();
    }
    body.to_owned()
}

/// 超過 200 個字（UTF-16）截斷加「…」。
fn shorten(s: &str) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    if units.len() > 200 { format!("{}…", String::from_utf16_lossy(&units[..200])) } else { s.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_取最後一個jsonrpc() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"a\":1}}\r\n\ndata: ping\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"a\":2}}\r\n";
        assert_eq!("{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"a\":2}}", from_sse(body).unwrap());
        assert_eq!(Err(Error::Mcp("MCP 回應沒有資料".into())), from_sse("data: ping\n"));
    }

    #[test]
    fn 錯誤訊息_取message_太長截斷() {
        assert_eq!("壞了", error_message("{\"message\":\"壞了\"}"));
        assert_eq!("<html>", error_message("<html>"));
        let long = "字".repeat(250);
        assert_eq!(format!("{}…", "字".repeat(200)), shorten(&long));
        assert_eq!("短", shorten("短"));
    }
}
