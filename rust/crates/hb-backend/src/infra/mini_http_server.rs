//! 對應 C# `Infra/MiniHttpServer.cs`：只聽 127.0.0.1 的極簡 HTTP 伺服器。
//! 不用 HttpListener 是因為它在非系統管理員下需要 URL ACL。用在 OAuth 回呼與 Claude Code hooks。
//!
//! 連線在 `hb_backend::rt` 的 tokio 執行環境處理；`start` 可以在任何執行緒呼叫。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use crate::rt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiniRequest {
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiniResponse {
    pub status: u16,
    pub content_type: String,
    pub body: String,
}

impl MiniResponse {
    pub fn new(status: u16, content_type: &str, body: impl Into<String>) -> Self {
        Self { status, content_type: content_type.to_owned(), body: body.into() }
    }
}

pub type HandlerFuture = Pin<Box<dyn Future<Output = anyhow::Result<MiniResponse>> + Send>>;
pub type Handler = Arc<dyn Fn(MiniRequest) -> HandlerFuture + Send + Sync>;

pub struct MiniHttpServer {
    port: u16,
    handler: Handler,
    stop_tx: watch::Sender<bool>,
    done_rx: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl MiniHttpServer {
    pub fn new<F, Fut>(port: u16, handler: F) -> Self
    where
        F: Fn(MiniRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = anyhow::Result<MiniResponse>> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |req| Box::pin(handler(req)) as HandlerFuture);
        let (stop_tx, _) = watch::channel(false);
        Self { port, handler, stop_tx, done_rx: None }
    }

    /// 開始聽 127.0.0.1:port；連接埠被占用時回傳錯誤。
    pub fn start(&mut self) -> std::io::Result<()> {
        let std_listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, self.port))?;
        std_listener.set_nonblocking(true)?;
        let listener = {
            let _guard = rt::handle().enter();
            TcpListener::from_std(std_listener)?
        };
        let handler = self.handler.clone();
        let stop_rx = self.stop_tx.subscribe();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        self.done_rx = Some(done_rx);
        rt::spawn(async move {
            accept_loop(listener, handler, stop_rx).await;
            let _ = done_tx.send(());
        });
        Ok(())
    }

    /// 停止並等到連接埠真的釋放（在 async 流程裡用；之後同一個連接埠可以馬上再開）。
    pub async fn stop(mut self) {
        let _ = self.stop_tx.send(true);
        if let Some(done) = self.done_rx.take() {
            let _ = tokio::time::timeout(Duration::from_secs(2), done).await;
        }
    }

    /// 停止（C# `Dispose`）；不等連接埠釋放。
    pub fn dispose(&mut self) {
        let _ = self.stop_tx.send(true);
    }
}

impl Drop for MiniHttpServer {
    fn drop(&mut self) {
        self.dispose();
    }
}

async fn accept_loop(listener: TcpListener, handler: Handler, mut stop_rx: watch::Receiver<bool>) {
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let accepted = tokio::select! {
            r = listener.accept() => r,
            _ = stop_rx.changed() => break,
        };
        let Ok((client, _)) = accepted else {
            break;
        };
        tokio::spawn(handle(client, handler.clone()));
    }
}

/// 對方提早斷線、逾時、伺服器停止：忽略（C# 只有 IO／取消／Socket 例外，Rust 這裡不會有其他種類）。
#[derive(Debug)]
struct Quiet;

impl From<std::io::Error> for Quiet {
    fn from(_: std::io::Error) -> Self {
        Quiet
    }
}

async fn handle(mut client: TcpStream, handler: Handler) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let work = async {
        let request = match tokio::time::timeout_at(deadline, read_request(&mut client)).await {
            Ok(r) => r?,
            Err(_) => return Err(Quiet),
        };
        let Some(request) = request else {
            return Ok(());
        };

        // 處理器 panic 也當成處理失敗
        let response = match tokio::spawn((handler)(request)).await {
            Ok(Ok(r)) => r,
            Ok(Err(err)) => {
                log::error!("HTTP 處理失敗: {err}");
                MiniResponse::new(500, "text/plain", "error")
            }
            Err(err) => {
                log::error!("HTTP 處理失敗: {err}");
                MiniResponse::new(500, "text/plain", "error")
            }
        };

        let body = response.body.as_bytes();
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.status,
            if response.status == 200 { "OK" } else { "Error" },
            response.content_type,
            body.len()
        );
        let write = async {
            client.write_all(head.as_bytes()).await?;
            client.write_all(body).await?;
            client.flush().await?;
            let _ = client.shutdown().await;
            Ok::<_, std::io::Error>(())
        };
        match tokio::time::timeout_at(deadline, write).await {
            Ok(r) => r.map_err(Quiet::from),
            Err(_) => Err(Quiet),
        }
    };
    // 停止時不中斷進行中的連線（最多 10 秒），讓瀏覽器收得到「登入完成／失敗」的頁面
    let _: Result<(), Quiet> = work.await;
}

/// C# `Encoding.ASCII.GetString`：非 ASCII 的位元組變成 '?'。
fn ascii(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if b < 0x80 { b as char } else { '?' }).collect()
}

fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

async fn read_request(stream: &mut TcpStream) -> Result<Option<MiniRequest>, Quiet> {
    let mut buffer = vec![0u8; 8192];
    let mut head: Vec<u8> = Vec::new();
    let header_end;
    loop {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            return Ok(None);
        }
        head.extend_from_slice(&buffer[..n]);
        if head.len() > 64 * 1024 {
            return Ok(None);
        }
        if let Some(end) = find_header_end(&head) {
            header_end = end;
            break;
        }
    }

    let text = ascii(&head[..header_end]);
    let lines: Vec<&str> = text.split("\r\n").collect();
    let first: Vec<&str> = lines[0].split(' ').collect();
    if first.len() < 2 {
        return Ok(None);
    }

    let mut content_length: i64 = 0;
    for line in lines.iter().skip(1) {
        if let Some(colon) = line.find(':')
            && colon > 0
            && line[..colon].trim().eq_ignore_ascii_case("Content-Length")
        {
            // int.TryParse 失敗時是 0
            content_length = line[colon + 1..].trim().parse::<i32>().map(i64::from).unwrap_or(0);
        }
    }
    if !(0..=5_000_000).contains(&content_length) {
        return Ok(None);
    }
    let content_length = content_length as usize;

    let mut body: Vec<u8> = head[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..n]);
    }

    let target = first[1];
    let body_text = String::from_utf8_lossy(&body[..body.len().min(content_length)]).into_owned();
    let (path, query) = match target.find('?') {
        Some(q) => (&target[..q], &target[q + 1..]),
        None => (target, ""),
    };
    Ok(Some(MiniRequest {
        method: first[0].to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        body: body_text,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }

    fn raw(port: u16, request: &[u8]) -> String {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.write_all(request).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out
    }

    #[test]
    fn 收到請求_回傳處理器的回應() {
        let port = free_port();
        let mut server = MiniHttpServer::new(port, |req: MiniRequest| async move {
            Ok(match req.path.as_str() {
                "/health" => MiniResponse::new(200, "text/plain", "ok"),
                "/hook" => {
                    MiniResponse::new(200, "application/json", format!("{}|{}|{}", req.method, req.query, req.body))
                }
                "/boom" => anyhow::bail!("壞了"),
                _ => MiniResponse::new(404, "text/plain", "not found"),
            })
        });
        server.start().unwrap();

        let ok = raw(port, b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(ok.starts_with("HTTP/1.1 200 OK\r\n"), "{ok}");
        assert!(ok.contains("Content-Type: text/plain; charset=utf-8\r\n"));
        assert!(ok.contains("Content-Length: 2\r\nConnection: close\r\n\r\nok"));

        let body = "{\"a\":\"中文\"}";
        let req = format!("POST /hook?x=1 HTTP/1.1\r\ncontent-length: {}\r\n\r\n{body}", body.len());
        let hook = raw(port, req.as_bytes());
        assert!(hook.ends_with(&format!("POST|x=1|{body}")), "{hook}");

        assert!(raw(port, b"GET /nope HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404 Error\r\n"));
        let boom = raw(port, b"GET /boom HTTP/1.1\r\n\r\n");
        assert!(boom.starts_with("HTTP/1.1 500 Error\r\n") && boom.ends_with("error"), "{boom}");

        rt::block_on(server.stop());
        // 停止後連接埠釋放，可以再開
        let mut again = MiniHttpServer::new(port, |_| async { Ok(MiniResponse::new(200, "text/plain", "again")) });
        again.start().unwrap();
        assert!(raw(port, b"GET / HTTP/1.1\r\n\r\n").ends_with("again"));
    }

    #[test]
    fn 連接埠被占用_start回傳錯誤() {
        let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = busy.local_addr().unwrap().port();
        let mut server = MiniHttpServer::new(port, |_| async { Ok(MiniResponse::new(200, "text/plain", "")) });
        assert!(server.start().is_err());
    }
}
