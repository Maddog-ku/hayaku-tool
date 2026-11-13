//! 測試共用：本機假 Slack 伺服器（只聽 127.0.0.1 的隨機埠，不連真的 Slack）、會真的等待的 block_on、暫存的 AppPaths。

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Wake, Waker};

use hb_backend::infra::AppPaths;
use hb_backend::rt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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
    pub body: String,
}

impl Reply {
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self { status, body: body.into() }
    }
}

type Handler = Arc<dyn Fn(&Recorded) -> Reply + Send + Sync>;

pub struct MockServer {
    pub base_url: String,
    requests: Arc<Mutex<Vec<Recorded>>>,
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
                    let head = format!(
                        "HTTP/1.1 {} X\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        reply.status,
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

/// 在目前執行緒等 future 跑完（不是 tokio：模擬 UI 執行緒上的 `slint::spawn_local`）。
pub fn block_on<F: Future>(fut: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park_timeout(std::time::Duration::from_millis(50)),
        }
    }
}

static APP_DIR_LOCK: Mutex<()> = Mutex::new(());

/// 把 AppPaths 改到暫存資料夾（不碰真正的 %APPDATA%\HayakuBar）；結束時還原。
pub struct TempApp {
    _lock: MutexGuard<'static, ()>,
    pub dir: tempfile::TempDir,
}

impl TempApp {
    pub fn new() -> Self {
        let lock = APP_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        AppPaths::set_dir(Some(dir.path()));
        Self { _lock: lock, dir }
    }
}

impl Drop for TempApp {
    fn drop(&mut self) {
        AppPaths::set_dir(None);
    }
}
