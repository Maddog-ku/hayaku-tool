using System.Text.Encodings.Web;
using System.Text.Json;
using HayakuBar.Auth;
using HayakuBar.Config;
using HayakuBar.Mcp;

namespace HayakuBar.Infra;

/// <summary>
/// 診斷用：HayakuBar.exe --mcp-call &lt;工具名稱&gt; &lt;參數 JSON&gt; &lt;輸出檔&gt;
/// 用目前的登入呼叫一支 Hayaku MCP 工具，把結果寫到檔案後結束（不顯示任何視窗）。
/// 工具名稱填 tools/list 時列出所有工具與參數說明。
/// 請先關掉正在執行的 HayakuBar，避免兩個程序同時更新 token。
/// </summary>
public static class Diagnostics
{
    public static async Task<int> McpCallAsync(string[] args)
    {
        if (args.Length < 4)
            return 2;
        var (tool, argsJson, output) = (args[1], args[2], args[3]);
        try
        {
            var config = AppConfig.Load();
            using var http = new HttpClient { Timeout = TimeSpan.FromSeconds(30) };
            var auth = new OAuthClient(http, config.BaseUrl);
            var mcp = new McpClient(http, auth, config.BaseUrl);
            object? arguments = string.IsNullOrWhiteSpace(argsJson) || argsJson == "{}"
                ? null
                : JsonSerializer.Deserialize<JsonElement>(argsJson);
            var result = tool == "tools/list"
                ? await mcp.ListToolsAsync(CancellationToken.None)
                : await mcp.CallToolAsync(tool, arguments, CancellationToken.None);
            var options = new JsonSerializerOptions { WriteIndented = true, Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping };
            await File.WriteAllTextAsync(output, JsonSerializer.Serialize(result, options));
            return 0;
        }
        catch (Exception ex)
        {
            await File.WriteAllTextAsync(output, $"ERROR: {ex}");
            return 1;
        }
    }
}
