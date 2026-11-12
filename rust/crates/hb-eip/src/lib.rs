//! EIP 請假：登入、讀假單、LeavePlanner（對應 C# `Eip/`）。
//!
//! - [`EipClient`]：登入公司 EIP（PHP 網站、cookie、兩段登入）、讀假單（`EipClient.cs`）。
//! - [`EipCredentials`]／[`EipCredentialStore`]：DPAPI 加密的帳密 eip.bin（`EipCredentials.cs`）。
//! - [`EipSyncState`]：eip-sync.json（`EipSyncState.cs`）。
//! - [`LeavePlanner`]：假單 → Hayaku 請假的規則（`LeavePlanner.cs`）。

pub mod eip_client;
pub mod eip_credentials;
pub mod eip_sync_state;
mod html;
pub mod leave_planner;
pub mod net_time;

pub use eip_client::{EipClient, EipError, EipLeave, EipLoginResult, FormFields};
pub use eip_credentials::{EipCredentialStore, EipCredentials};
pub use eip_sync_state::{EipSyncState, SyncedLeave};
pub use leave_planner::{LeavePiece, LeavePlanner};

#[cfg(test)]
mod tests;
