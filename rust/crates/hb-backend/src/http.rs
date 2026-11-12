//! 共用的 HttpClient（對應 C# `App.xaml.cs` 建立的 `HttpClient`：逾時 20 秒、User-Agent `HayakuBar/1.0`）。

use std::time::Duration;

/// App 用的 HTTP client（OAuthClient、McpClient 共用）。跟 .NET HttpClient 一樣會保留 cookie。
pub fn app_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent("HayakuBar/1.0")
        .cookie_store(true)
        .build()
        .unwrap_or_else(|err| {
            log::error!("建立 HttpClient 失敗: {err}");
            reqwest::Client::new()
        })
}
