//! McpClient 的測試（用本機假伺服器，不連真的 Hayaku）。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hb_api::{McpError, McpTransport};
use serde_json::{Value, json};

use super::McpClient;
use crate::auth::OAuthClient;
use crate::auth::oauth_client::tests::{meta_reply, store_with};
use crate::error::{Error, ErrorKind, error_kind, is_auth_required};
use crate::rt;
use crate::test_support::{MockServer, Recorded, Reply};

fn rpc_method(req: &Recorded) -> (String, Value) {
    let body: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
    (body["method"].as_str().unwrap_or("").to_owned(), body)
}

fn result(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

/// 已登入（token 還有一小時）的 McpClient。
fn client(server: &MockServer, dir: &std::path::Path) -> McpClient {
    let store = store_with(dir, Some("at-1"), Some("rt-1"), 3600);
    let http = crate::http::app_client();
    let auth = OAuthClient::with_store(http.clone(), &server.base_url, &store);
    McpClient::new(http, auth, &format!("{}/", server.base_url))
}

#[test]
fn call_tool_先initialize_帶標頭_回傳structured_content() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| {
        let (method, body) = rpc_method(req);
        match method.as_str() {
            "initialize" => Reply::json(200, result(&body["id"], json!({ "protocolVersion": "2025-06-18" }))),
            _ => Reply::json(
                200,
                result(
                    &body["id"],
                    json!({ "content": [{ "type": "text", "text": "ignored" }], "structuredContent": { "user": "harry" } }),
                ),
            ),
        }
    });
    let mcp = client(&server, dir.path());
    assert_eq!(json!({ "user": "harry" }), rt::block_on(mcp.call_tool("whoami", Value::Null)).unwrap());
    // 第二次不再 initialize
    rt::block_on(mcp.call_tool("whoami", json!({ "a": 1 }))).unwrap();

    let reqs = server.requests();
    assert_eq!(3, reqs.len());
    let (m0, b0) = rpc_method(&reqs[0]);
    assert_eq!("initialize", m0);
    assert_eq!(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "HayakuBar", "version": "1.0.0" } } }),
        b0
    );
    let (_, b1) = rpc_method(&reqs[1]);
    assert_eq!(
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "whoami", "arguments": {} } }),
        b1
    );
    let (_, b2) = rpc_method(&reqs[2]);
    assert_eq!(json!({ "name": "whoami", "arguments": { "a": 1 } }), b2["params"]);
    let r = &reqs[1];
    assert_eq!("POST", r.method);
    assert_eq!("/mcp", r.path);
    assert_eq!("Bearer at-1", r.headers["authorization"]);
    assert_eq!("application/json, text/event-stream", r.headers["accept"]);
    assert_eq!("2025-06-18", r.headers["mcp-protocol-version"]);
    assert_eq!("application/json; charset=utf-8", r.headers["content-type"]);
    assert_eq!("HayakuBar/1.0", r.headers["user-agent"]);
}

#[test]
fn call_tool_文字內容_json就解析_不是就當字串_沒有是null() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| {
        let (_, body) = rpc_method(req);
        let name = body["params"]["name"].as_str().unwrap_or("");
        let r = match name {
            "json" => {
                json!({ "content": [{ "type": "image", "data": "x" }, { "type": "text", "text": "[1,{\"a\":\"中\"}]" }] })
            }
            "plain" => json!({ "content": [{ "type": "text", "text": "已完成" }] }),
            "empty" => json!({ "content": [] }),
            "fail" => {
                json!({ "isError": true, "content": [{ "type": "text", "text": "找不到任務" }] })
            }
            "fail2" => json!({ "isError": true }),
            _ => json!({}),
        };
        Reply::json(200, result(&body["id"], r))
    });
    let mcp = client(&server, dir.path());
    assert_eq!(json!([1, { "a": "中" }]), rt::block_on(mcp.call_tool("json", Value::Null)).unwrap());
    assert_eq!(json!("已完成"), rt::block_on(mcp.call_tool("plain", Value::Null)).unwrap());
    assert_eq!(Value::Null, rt::block_on(mcp.call_tool("empty", Value::Null)).unwrap());
    assert_eq!(Err(Error::Mcp("找不到任務".into())), rt::block_on(mcp.call_tool("fail", Value::Null)));
    assert_eq!(Err(Error::Mcp("fail2 失敗".into())), rt::block_on(mcp.call_tool("fail2", Value::Null)));
    // raw：整個結果
    let raw = rt::block_on(mcp.call_tool_raw("json", Value::Null)).unwrap();
    assert_eq!("image", raw["content"][0]["type"]);
    assert_eq!(Err(Error::Mcp("找不到任務".into())), rt::block_on(mcp.call_tool_raw("fail", Value::Null)));
}

#[test]
fn sse回應與jsonrpc錯誤() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| {
        let (method, body) = rpc_method(req);
        if method == "tools/list" {
            return Reply::sse(format!("event: message\ndata: {}\n\n", result(&body["id"], json!({ "tools": [] }))));
        }
        Reply::json(
            200,
            json!({ "jsonrpc": "2.0", "id": body["id"], "error": { "code": -32602, "message": "參數錯誤" } })
                .to_string(),
        )
    });
    let mcp = client(&server, dir.path());
    assert_eq!(json!({ "tools": [] }), rt::block_on(mcp.list_tools()).unwrap());
    // initialize 失敗（JSON-RPC 錯誤）不影響；tools/call 的錯誤往外丟
    assert_eq!(Err(Error::Mcp("參數錯誤".into())), rt::block_on(mcp.call_tool("x", Value::Null)));
    assert_eq!(3, server.requests().len());
}

#[test]
fn http錯誤_取message並截斷() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| {
        let (method, body) = rpc_method(req);
        if method == "initialize" {
            return Reply::json(200, result(&body["id"], json!({})));
        }
        if body["params"]["name"] == "long" {
            return Reply::json(502, "x".repeat(300));
        }
        Reply::json(500, r#"{"message":"伺服器壞了"}"#)
    });
    let mcp = client(&server, dir.path());
    assert_eq!(Err(Error::Mcp("Hayaku 回應 500：伺服器壞了".into())), rt::block_on(mcp.call_tool("x", Value::Null)));
    assert_eq!(
        Err(Error::Mcp(format!("Hayaku 回應 502：{}…", "x".repeat(200)))),
        rt::block_on(mcp.call_tool("long", Value::Null))
    );
}

#[test]
fn 回401_強制refresh後重送() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        "/token" => Reply::json(200, r#"{"access_token":"at-2","expires_in":3600}"#),
        _ => {
            let (_, body) = rpc_method(req);
            if req.headers["authorization"] == "Bearer at-1" {
                Reply::json(401, "{}")
            } else {
                Reply::json(200, result(&body["id"], json!({ "structuredContent": { "ok": true } })))
            }
        }
    });
    let mcp = client(&server, dir.path());
    assert_eq!(json!({ "ok": true }), rt::block_on(mcp.call_tool_raw("x", Value::Null)).unwrap()["structuredContent"]);
    let paths: Vec<String> = server.requests().iter().map(|r| r.path.clone()).collect();
    assert_eq!(vec!["/mcp", "/.well-known/oauth-authorization-server", "/token", "/mcp"], paths);
}

#[test]
fn 一直401_需要重新登入_initialize也不吞掉() {
    let dir = tempfile::tempdir().unwrap();
    let server = MockServer::start(|req| match req.path.as_str() {
        "/.well-known/oauth-authorization-server" => meta_reply(req),
        "/token" => Reply::json(200, r#"{"access_token":"at-2"}"#),
        _ => Reply::json(401, "{}"),
    });
    let mcp = client(&server, dir.path());
    assert_eq!(
        Err(Error::AuthRequired("登入已失效，請重新登入".into())),
        rt::block_on(mcp.call_tool("x", Value::Null))
    );
    // initialize 就失敗，不會送 tools/call
    let calls = server.requests().iter().filter(|r| r.path == "/mcp").count();
    assert_eq!(2, calls);
}

#[test]
fn 透過mcp_transport_錯誤種類查得回來() {
    let dir = tempfile::tempdir().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let server = MockServer::start(move |req| {
        let (method, body) = rpc_method(req);
        c.fetch_add(1, Ordering::SeqCst);
        match method.as_str() {
            "initialize" => Reply::json(200, result(&body["id"], json!({}))),
            _ if body["params"]["name"] == "bad" => Reply::json(200, "<html>not json</html>"),
            _ => Reply::json(
                200,
                result(&body["id"], json!({ "isError": true, "content": [{ "type": "text", "text": "工具失敗" }] })),
            ),
        }
    });
    let mcp = client(&server, dir.path());
    let transport: &dyn McpTransport = &mcp;
    let err = rt::block_on(transport.call_tool("x", Value::Null)).unwrap_err();
    assert_eq!("工具失敗", err.0);
    assert_eq!(ErrorKind::Mcp, error_kind(&err));
    let err = rt::block_on(transport.call_tool("bad", Value::Null)).unwrap_err();
    assert_eq!(ErrorKind::Other, error_kind(&err));
    assert!(is_auth_required(&McpError("需要登入 Hayaku".into())));

    // 連不上：Http
    let mcp2 = McpClient::new(
        crate::http::app_client(),
        OAuthClient::with_store(
            crate::http::app_client(),
            "http://127.0.0.1:9",
            &store_with(dir.path(), Some("t"), None, 3600),
        ),
        "http://127.0.0.1:9",
    );
    let transport: &dyn McpTransport = &mcp2;
    let err = rt::block_on(transport.call_tool_raw("x", Value::Null)).unwrap_err();
    assert_eq!(ErrorKind::Http, error_kind(&err));
}
