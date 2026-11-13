//! 對應 C# `Infra/Diagnostics.cs`。
//!
//! 診斷用：HayakuBar.exe --mcp-call <工具名稱> <參數 JSON> <輸出檔>
//! 用目前的登入呼叫一支 Hayaku MCP 工具，把結果寫到檔案後結束（不顯示任何視窗）。
//! 工具名稱填 tools/list 時列出所有工具與參數說明。
//! 請先關掉正在執行的 HayakuBar，避免兩個程序同時更新 token。

use std::time::Duration;

use serde_json::Value;

use crate::auth::oauth_client::OAuthClient;
use crate::config::app_config::AppConfig;
use crate::infra::json_file;
use crate::mcp::mcp_client::McpClient;
use crate::rt;

/// 執行 `--mcp-call`，回傳結束代碼（0 成功、1 失敗、2 參數不足）。`args[0]` 是 `--mcp-call`。
/// 同步等到完成（不能在 tokio 執行緒裡呼叫）。
pub fn mcp_call(args: &[String]) -> i32 {
    if args.len() < 4 {
        return 2;
    }
    let (tool, args_json, output) = (args[1].as_str(), args[2].as_str(), args[3].as_str());
    let result = rt::block_on(async {
        let config = AppConfig::load();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .cookie_store(true)
            .build()
            .map_err(|e| anyhow::anyhow!(crate::error::http_message(&e)))?;
        let auth = OAuthClient::new(http.clone(), &config.base_url);
        let mcp = McpClient::new(http, auth, &config.base_url);
        let arguments = if args_json.trim().is_empty() || args_json == "{}" {
            Value::Null
        } else {
            serde_json::from_str::<Value>(args_json)?
        };
        let result =
            if tool == "tools/list" { mcp.list_tools().await? } else { mcp.call_tool(tool, arguments).await? };
        anyhow::Ok(json_file::to_string(&result)?)
    });
    match result {
        Ok(text) => match std::fs::write(output, text) {
            Ok(()) => 0,
            Err(err) => {
                let _ = std::fs::write(output, format!("ERROR: {err}"));
                1
            }
        },
        Err(err) => {
            let _ = std::fs::write(output, format!("ERROR: {err:#}"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 參數不足回傳2() {
        assert_eq!(2, mcp_call(&["--mcp-call".into(), "whoami".into()]));
    }
}
