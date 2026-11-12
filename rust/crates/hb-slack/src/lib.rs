//! Slack Web API、Socket Mode、SlackBridge（對應 C# `Slack/`，以及 `UI/SettingsWindow.Slack.cs` 不碰畫面的部分）。
//!
//! - [`SlackClient`]：Slack Web API（`SlackClient.cs`）。
//! - [`SocketModeClient`]：WebSocket 收按鈕、表單，斷線自動重連（`SocketModeClient.cs`）。
//! - [`SlackBridge`]：實作 `hb_core::SlackLink`，把通知傳到 Slack、處理 Slack 上的按鈕（`SlackBridge.cs`）。
//! - [`SlackMessages`]：Block Kit 訊息與表單、哪些通知要傳（`SlackMessages.cs`）。
//! - [`SlackCredentials`]／[`SlackCredentialStore`]：DPAPI 加密的 slack.bin（`SlackCredentials.cs`）。
//! - [`SlackState`]／[`SlackPost`]：slack.json，保留 7 天（`SlackState.cs`）。
//! - [`SlackSettings`]：設定頁要用的文字與動作（`SettingsWindow.Slack.cs`）。

pub mod settings;
pub mod slack_bridge;
pub mod slack_client;
pub mod slack_credentials;
pub mod slack_messages;
pub mod slack_state;
pub mod socket_mode_client;

pub use settings::{SlackSettings, SlackStatus, TestOutcome, Tone};
pub use slack_bridge::SlackBridge;
pub use slack_client::{SlackClient, SlackError, SlackException};
pub use slack_credentials::{SlackCredentialStore, SlackCredentials};
pub use slack_messages::{SlackInteraction, SlackMessages};
pub use slack_state::{SlackPost, SlackState};
pub use socket_mode_client::SocketModeClient;

#[cfg(test)]
mod test_support;
