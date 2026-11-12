//! 測試用的假 Hayaku 伺服器（只聽 127.0.0.1 的隨機埠，不連真的伺服器）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::rt;

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    /// 標頭名稱一律小寫
    pub headers: HashMap<String, String>,
    pub body: String,
}

pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Reply {
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self { status, content_type: "application/json", body: body.into() }
    }

    pub fn sse(body: impl Into<String>) -> Self {
        Self { status: 200, content_type: "text/event-stream", body: body.into() }
    }
}

type Handler = Arc<dyn Fn(&Recorded) -> Reply + Send + Sync>;

pub struct MockServer {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

impl MockServer {
    pub fn start(handler: impl Fn(&Recorded) -> Reply + Send + Sync + 'static) -> Self {
        let handler: Handler = Arc::new(handler);
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std_listener.set_nonblocking(true).unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = {
            let _g = rt::handle().enter();
            tokio::net::TcpListener::from_std(std_listener).unwrap()
        };
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        rt::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let handler = handler.clone();
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let Some(req) = read(&mut stream).await else {
                        return;
                    };
                    recorded.lock().unwrap().push(req.clone());
                    let reply = handler(&req);
                    let reason = if reply.status == 200 { "OK" } else { "Error" };
                    let head = format!(
                        "HTTP/1.1 {} {reason}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.status,
                        reply.content_type,
                        reply.body.len()
                    );
                    let _ = stream.write_all(head.as_bytes()).await;
                    let _ = stream.write_all(reply.body.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { base_url: format!("http://127.0.0.1:{port}"), requests }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

async fn read(stream: &mut tokio::net::TcpStream) -> Option<Recorded> {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    let end = loop {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(p) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
    };
    let head = String::from_utf8_lossy(&data[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let first: Vec<&str> = lines.next()?.split(' ').collect();
    let headers: HashMap<String, String> =
        lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_owned()))).collect();
    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut body = data[end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut buf).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    Some(Recorded {
        method: first.first()?.to_string(),
        path: first.get(1)?.to_string(),
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// 把 x-www-form-urlencoded 解成字典。
pub fn form(body: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes()).into_owned().collect()
}
