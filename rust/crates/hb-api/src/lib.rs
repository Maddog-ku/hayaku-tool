//! Hayaku MCP 工具的強型別包裝（對應 C# 的 `Mcp/HayakuApi*.cs`）與純函式工具（`Core/WorkMath.cs`、`Core/KeyFinder.cs`）。
//!
//! 這個 crate 不碰網路：真正送 JSON-RPC 的是 `hb-backend` 的 `McpClient`，它實作下面的 [`McpTransport`]。

pub mod api;
pub mod json;
pub mod key_finder;
pub mod models;
pub mod transport;
pub mod work_math;

pub use api::HayakuApi;
pub use key_finder::KeyFinder;
pub use models::*;
pub use transport::{BoxFuture, McpError, McpTransport};
pub use work_math::{DayPhase, DayPhaseKind, WorkMath};
