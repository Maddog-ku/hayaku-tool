//! 測試共用：假的 MCP transport、不靠執行環境的 block_on、JSON 文字轉 Value。

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use hb_api::{BoxFuture, McpError, McpTransport};
use serde_json::Value;

pub fn j(s: &str) -> Value {
    serde_json::from_str(s).expect("test json")
}

/// 記下每次呼叫的工具與參數；回應先找依工具名稱設定的，沒有再照順序拿 `reply` 排好的，都沒有回 null。
#[derive(Default)]
pub struct FakeTransport {
    calls: Mutex<Vec<(String, Value)>>,
    replies: Mutex<VecDeque<Value>>,
    by_tool: Mutex<HashMap<String, Result<Value, String>>>,
}

impl FakeTransport {
    pub fn reply(&self, v: Value) {
        self.replies.lock().unwrap().push_back(v);
    }

    pub fn on(&self, tool: &str, v: Value) {
        self.by_tool.lock().unwrap().insert(tool.to_owned(), Ok(v));
    }

    pub fn fail(&self, tool: &str, message: &str) {
        self.by_tool.lock().unwrap().insert(tool.to_owned(), Err(message.to_owned()));
    }

    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }

    pub fn tools(&self) -> Vec<String> {
        self.calls().into_iter().map(|c| c.0).collect()
    }
}

impl McpTransport for FakeTransport {
    fn call_tool<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, McpError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((name.to_owned(), arguments));
            if let Some(r) = self.by_tool.lock().unwrap().get(name) {
                return r.clone().map_err(McpError);
            }
            Ok(self.replies.lock().unwrap().pop_front().unwrap_or(Value::Null))
        })
    }

    fn call_tool_raw<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, McpError>> {
        self.call_tool(name, arguments)
    }
}

/// 不靠 async 執行環境跑完一個 future（假 transport 不會真的等待）。
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut f = std::pin::pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..1_000_000 {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
    panic!("future 沒有完成");
}
