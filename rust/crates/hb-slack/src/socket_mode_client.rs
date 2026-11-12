//! 對應 C# `Slack/SocketModeClient.cs`：Slack Socket Mode。HayakuBar 主動用 WebSocket 連到 Slack，
//! Slack 把按鈕、表單的互動從這條連線送過來，不需要對外開放的網址。
//! 每則訊息都要在 3 秒內回 envelope_id（先回再處理）；Slack 會不定時要求重連（disconnect），
//! 斷線就等一下再連（5 秒起跳，最多 5 分鐘）。
//!
//! 連線迴圈在 hb-backend 的 tokio 執行環境跑（C# `Task.Run`）；`problem()`、狀態事件可以從任何執行緒用，
//! 事件在 tokio 執行緒觸發，要更新畫面的人自己轉回 UI 執行緒（SlackBridge 用 channel 轉）。

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hb_backend::rt;
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::slack_client::{SlackClient, SlackError};

/// 拿 Socket Mode 連線網址（每次連線都要重拿；C# `Func<CancellationToken, Task<string>>`）。
pub type OpenUrl = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<String, SlackError>> + Send>> + Send + Sync>;

/// 收到按鈕、表單（interactive 的 payload）；在 tokio 執行緒呼叫（C# `Func<JsonElement, Task>`，外面包 `Task.Run`）。
pub type OnInteractive = Arc<dyn Fn(Value) + Send + Sync>;

type Handler = Arc<dyn Fn() + Send + Sync>;

const MIN_DELAY: Duration = Duration::from_secs(5);
const MAX_DELAY: Duration = Duration::from_secs(5 * 60);
/// C# `ClientWebSocket.Options.KeepAliveInterval`（.NET 定時送沒人要求的 Pong）。
const KEEP_ALIVE: Duration = Duration::from_secs(30);

struct Shared {
    open_url: OpenUrl,
    on_interactive: OnInteractive,
    /// 連線狀態：None＝已連上；其他是給人看的說明（連線中、斷線原因）。
    problem: Mutex<Option<String>>,
    status_changed: Mutex<Vec<Handler>>,
    min_delay: Duration,
    max_delay: Duration,
}

impl Shared {
    /// 改狀態；`cancelled` 是這次連線迴圈的取消旗標，已經 Stop 的迴圈不能再改（Stop 已經寫了「未連線」）。
    fn set_problem(&self, problem: Option<&str>, cancelled: Option<&AtomicBool>) {
        {
            let mut current = self.problem.lock().unwrap_or_else(|e| e.into_inner());
            if cancelled.is_some_and(|c| c.load(Ordering::SeqCst)) || current.as_deref() == problem {
                return;
            }
            *current = problem.map(str::to_owned);
        }
        let handlers: Vec<Handler> = self.status_changed.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for h in handlers {
            h();
        }
    }
}

struct Run {
    cancelled: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

pub struct SocketModeClient {
    shared: Arc<Shared>,
    run: Mutex<Option<Run>>,
}

impl SocketModeClient {
    pub fn new(open_url: OpenUrl, on_interactive: OnInteractive) -> Self {
        Self::with_delays(open_url, on_interactive, MIN_DELAY, MAX_DELAY)
    }

    /// 測試用：換掉重連的等待時間。
    pub(crate) fn with_delays(open_url: OpenUrl, on_interactive: OnInteractive, min: Duration, max: Duration) -> Self {
        Self {
            shared: Arc::new(Shared {
                open_url,
                on_interactive,
                problem: Mutex::new(Some("連線中…".to_owned())),
                status_changed: Mutex::new(Vec::new()),
                min_delay: min,
                max_delay: max,
            }),
            run: Mutex::new(None),
        }
    }

    /// 連線狀態：None＝已連上；其他是給人看的說明（連線中、斷線原因）。
    pub fn problem(&self) -> Option<String> {
        self.shared.problem.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 狀態變了（C# `StatusChanged`）；在 tokio 執行緒或呼叫 Stop 的執行緒觸發。
    pub fn on_status_changed(&self, f: impl Fn() + Send + Sync + 'static) {
        self.shared.status_changed.lock().unwrap_or_else(|e| e.into_inner()).push(Arc::new(f));
    }

    pub fn start(&self) {
        let mut run = self.run.lock().unwrap_or_else(|e| e.into_inner());
        if run.is_some() {
            return;
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let handle = rt::spawn(run_loop(self.shared.clone(), cancelled.clone()));
        *run = Some(Run { cancelled, handle });
    }

    pub fn stop(&self) {
        let run = self.run.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(run) = run {
            // C# 取消 CancellationToken：迴圈停在哪個 await 就在那裡結束，WebSocket 一起關掉
            run.cancelled.store(true, Ordering::SeqCst);
            run.handle.abort();
        }
        self.shared.set_problem(Some("未連線"), None);
    }

    /// C# `Dispose()`＝`Stop()`。
    pub fn dispose(&self) {
        self.stop();
    }
}

impl Drop for SocketModeClient {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, thiserror::Error)]
enum LoopError {
    #[error(transparent)]
    Slack(#[from] SlackError),
    #[error(transparent)]
    Ws(#[from] tokio_tungstenite::tungstenite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

async fn run_loop(shared: Arc<Shared>, cancelled: Arc<AtomicBool>) {
    let is_cancelled = || cancelled.load(Ordering::SeqCst);
    let mut delay = shared.min_delay;
    while !is_cancelled() {
        shared.set_problem(Some("連線中…"), Some(&cancelled));
        let result = connect_and_receive(&shared, &cancelled, &mut delay).await;
        if let Err(err) = result {
            if is_cancelled() {
                return;
            }
            log::error!("Slack 連線中斷: {err}");
            let text = match &err {
                LoopError::Slack(SlackError::Slack(se)) => format!("連不上：{}", SlackClient::describe(&se.error)),
                other => format!("連不上：{other}"),
            };
            shared.set_problem(Some(&text), Some(&cancelled));
        }
        if is_cancelled() {
            return;
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(shared.max_delay);
    }
}

async fn connect_and_receive(shared: &Shared, cancelled: &AtomicBool, delay: &mut Duration) -> Result<(), LoopError> {
    let url = (shared.open_url)().await?;
    let (ws, _) = tokio_tungstenite::connect_async(url.as_str()).await?;
    receive(ws, shared, cancelled, delay).await
}

/// 一直收到 Slack 要求重連或連線斷掉為止。
async fn receive<S>(
    mut ws: tokio_tungstenite::WebSocketStream<S>,
    shared: &Shared,
    cancelled: &AtomicBool,
    delay: &mut Duration,
) -> Result<(), LoopError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut keep_alive = tokio::time::interval_at(tokio::time::Instant::now() + KEEP_ALIVE, KEEP_ALIVE);
    while !cancelled.load(Ordering::SeqCst) {
        let next = tokio::select! {
            m = ws.next() => m,
            _ = keep_alive.tick() => {
                ws.send(Message::Pong(Default::default())).await?;
                continue;
            }
        };
        let data = match next {
            None => return Ok(()),
            Some(m) => match m? {
                Message::Text(t) => t.as_bytes().to_vec(),
                Message::Binary(b) => b.to_vec(),
                Message::Close(_) => return Ok(()),
                _ => continue,
            },
        };
        let root: Value = serde_json::from_slice(&data)?;
        let r#type = root.get("type").and_then(Value::as_str);
        if r#type == Some("hello") {
            *delay = shared.min_delay;
            shared.set_problem(None, Some(cancelled));
            log::info!("Slack Socket Mode 已連線");
            continue;
        }
        if r#type == Some("disconnect") {
            log::info!("Slack 要求重連（{}）", root.get("reason").and_then(Value::as_str).unwrap_or(""));
            return Ok(());
        }
        if let Some(id) = root.get("envelope_id").and_then(Value::as_str) {
            ws.send(Message::text(json!({ "envelope_id": id }).to_string())).await?;
        }
        if r#type == Some("interactive")
            && let Some(payload) = root.get("payload")
        {
            let on_interactive = shared.on_interactive.clone();
            let copy = payload.clone();
            // C# Task.Run 包起來：處理時出錯不影響收下一則
            tokio::spawn(async move {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| on_interactive(copy))).is_err() {
                    log::error!("處理 Slack 按鈕失敗");
                }
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use tokio::net::TcpListener;

    /// 本機假的 Socket Mode 伺服器：每次連線送 hello、一則 interactive，等 ack 後送 disconnect；ack 記下來。
    fn fake_server() -> (String, mpsc::Receiver<String>, Arc<Mutex<u32>>) {
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std_listener.set_nonblocking(true).unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = {
            let _g = rt::handle().enter();
            TcpListener::from_std(std_listener).unwrap()
        };
        let (tx, rx) = mpsc::channel();
        let connections = Arc::new(Mutex::new(0u32));
        let count = connections.clone();
        rt::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let tx = tx.clone();
                *count.lock().unwrap() += 1;
                let n = *count.lock().unwrap();
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    ws.send(Message::text(r#"{"type":"hello","num_connections":1}"#)).await.unwrap();
                    let payload = format!(
                        r#"{{"envelope_id":"e{n}","type":"interactive","payload":{{"type":"block_actions","user":{{"id":"U1"}}}}}}"#
                    );
                    ws.send(Message::text(payload)).await.unwrap();
                    while let Some(Ok(m)) = ws.next().await {
                        if let Message::Text(t) = m {
                            tx.send(t.to_string()).unwrap();
                            ws.send(Message::text(r#"{"type":"disconnect","reason":"refresh_requested"}"#))
                                .await
                                .unwrap();
                        }
                    }
                });
            }
        });
        (format!("ws://127.0.0.1:{port}"), rx, connections)
    }

    #[test]
    fn 連上後回ack_收到按鈕_要求重連就重連() {
        let (url, acks, connections) = fake_server();
        let (got_tx, got_rx) = mpsc::channel::<Value>();
        let got_tx = Mutex::new(got_tx);
        let open_url: OpenUrl = Arc::new(move || {
            let url = url.clone();
            Box::pin(async move { Ok(url) })
        });
        let client = SocketModeClient::with_delays(
            open_url,
            Arc::new(move |v| {
                let _ = got_tx.lock().unwrap().send(v);
            }),
            Duration::from_millis(50),
            Duration::from_millis(200),
        );
        assert_eq!(Some("連線中…".to_owned()), client.problem());
        client.start();
        let wait = Duration::from_secs(10);
        assert_eq!(r#"{"envelope_id":"e1"}"#, acks.recv_timeout(wait).unwrap());
        assert_eq!(Some("U1"), got_rx.recv_timeout(wait).unwrap()["user"]["id"].as_str());
        // disconnect 後等一下會重連，拿新的網址再連一次
        assert_eq!(r#"{"envelope_id":"e2"}"#, acks.recv_timeout(wait).unwrap());
        assert!(*connections.lock().unwrap() >= 2);
        client.stop();
        assert_eq!(Some("未連線".to_owned()), client.problem());
    }

    #[test]
    fn 拿不到網址_顯示slack錯誤的說明() {
        let open_url: OpenUrl = Arc::new(|| {
            Box::pin(async {
                Err(crate::slack_client::SlackException::new("apps.connections.open", "invalid_auth").into())
            })
        });
        let client =
            SocketModeClient::with_delays(open_url, Arc::new(|_| {}), Duration::from_secs(60), Duration::from_secs(60));
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        client.on_status_changed(move || {
            let _ = tx.lock().unwrap().send(());
        });
        client.start();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while client.problem().as_deref() != Some("連不上：金鑰不對，請重新複製") {
            assert!(std::time::Instant::now() < deadline, "{:?}", client.problem());
            let _ = rx.recv_timeout(Duration::from_millis(200));
        }
        client.dispose();
        assert_eq!(Some("未連線".to_owned()), client.problem());
    }
}
