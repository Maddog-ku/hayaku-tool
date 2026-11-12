//! 全域 tokio 執行環境（C# 沒有對應檔：.NET 的 async/await 自帶執行緒池；Rust 版網路 IO 都在這裡跑）。
//!
//! UI 執行緒跑 Slint 事件迴圈（非 tokio）；要做網路 IO 時用 [`bg`] 把 future 丟到這裡的 tokio 執行緒池，
//! 回傳的 future 可以在任何執行器（例如 `slint::spawn_local`）裡 await。

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};

use tokio::runtime::{Handle, Runtime};
use tokio::task::JoinHandle;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// 全域的多執行緒 runtime（第一次用到時建立）。
pub fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("hb-tokio")
            .build()
            .expect("無法建立 tokio 執行環境")
    })
}

/// 全域 runtime 的 handle。
pub fn handle() -> &'static Handle {
    runtime().handle()
}

/// 把 future 丟到 tokio 跑，回傳一個可以在任何執行器 await 的 future（結果就是原 future 的結果）。
///
/// - 回傳的 future 被丟棄（呼叫端取消）時，tokio 那邊的工作也一起取消（等同 C# 的 CancellationToken）。
/// - 工作 panic 時，在 await 的那一端重新 panic（跟 C# 例外往外丟一樣）。
pub fn bg<F>(fut: F) -> Bg<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    Bg { handle: Some(runtime().spawn(fut)) }
}

/// 丟到 tokio 跑、不等結果（fire-and-forget，等同 C# 的 `_ = XxxAsync()`）。
pub fn spawn<F>(fut: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    runtime().spawn(fut)
}

/// 在目前執行緒同步等 future 跑完（只給 main、診斷模式這種不在 runtime 裡的地方用）。
pub fn block_on<F: Future>(fut: F) -> F::Output {
    runtime().block_on(fut)
}

/// [`bg`] 回傳的 future。
pub struct Bg<T> {
    handle: Option<JoinHandle<T>>,
}

impl<T> Future for Bg<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let Some(handle) = self.handle.as_mut() else {
            panic!("Bg 已經完成後又被 poll");
        };
        match Pin::new(handle).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.handle = None;
                match result {
                    Ok(value) => Poll::Ready(value),
                    Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
                    Err(err) => panic!("背景工作被取消：{err}"),
                }
            }
        }
    }
}

impl<T> Drop for Bg<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bg_可以在非tokio執行器await() {
        // 用最簡單的自製 block_on（不是 tokio）等 bg 的結果
        let value = simple_block_on(bg(async {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            42
        }));
        assert_eq!(42, value);
    }

    fn simple_block_on<F: Future>(fut: F) -> F::Output {
        use std::sync::Arc;
        use std::task::{Wake, Waker};
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
                Poll::Pending => std::thread::park(),
            }
        }
    }
}
