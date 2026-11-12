//! 對應 C# `Mcp/McpClient.cs`、`Mcp/McpStats.cs`（`HayakuApi*.cs` 在 hb-api）。

pub mod mcp_client;
pub mod mcp_stats;

pub use mcp_client::McpClient;

#[cfg(test)]
mod tests;
