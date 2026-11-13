//! 對應 C# `Infra/`（`GlobalHotkey.cs`、`Native.cs` 在 hb-win）。

pub mod app_paths;
pub mod diagnostics;
pub mod dpapi;
pub mod housekeeping;
mod json_escape_table;
pub mod json_file;
pub mod log;
pub mod mini_http_server;
pub mod startup_registration;
pub mod web;

pub use app_paths::AppPaths;
pub use mini_http_server::{MiniHttpServer, MiniRequest, MiniResponse};
