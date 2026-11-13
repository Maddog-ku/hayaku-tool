//! Claude Code 監看、session 整理、hooks（對應 C# `Claude/`）。
//!
//! - [`ClaudeMonitor`]：監看 ~/.claude/projects 的 session 檔與 hooks 事件（`ClaudeMonitor.cs`），只在 UI 執行緒使用。
//! - [`session_digest`]：一天的對話與 git commit 整理成工作日誌、替計時段產生備註（`SessionDigest.cs`）。
//! - [`shell_edits`]：Bash／PowerShell 指令有沒有在專案裡寫檔（`ShellEdits.cs`）。
//! - [`HookInstaller`]：在 repo 的 .claude/settings.local.json 加上 hooks（`HookInstaller.cs`）。
//! - [`ClaudeEstimator`]：背景跑 `claude -p` 估預計工時（`ClaudeEstimator.cs`）。

pub mod claude_estimator;
pub mod claude_monitor;
pub mod hook_installer;
pub mod session_digest;
pub mod shell_edits;
mod util;

pub use claude_estimator::ClaudeEstimator;
pub use claude_monitor::{ClaudeMonitor, DetectedProject, DiscoveredProject, InvalidPath, KnownKey, RepoActivity};
pub use hook_installer::HookInstaller;
pub use session_digest::{ClaudePrompt, GitCommit};
pub use util::eq_ic;

/// `ProcessStartInfo.CreateNoWindow`：子程序不開主控台視窗。
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(test)]
mod tests;
