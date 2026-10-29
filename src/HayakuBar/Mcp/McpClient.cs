using System.Net;
using System.Net.Http.Json;
using System.Text.Json;
using HayakuBar.Auth;
using HayakuBar.Infra;

namespace HayakuBar.Mcp;

public sealed class McpException(string message) : Exception(message);

/// <summary>Hayaku /mcp 的 JSON-RPC 呼叫（無狀態、只接受 POST）。</summary>
public sealed class McpClient
{
    const string ProtocolVersion = "2025-06-18";

    readonly HttpClient _http;
    readonly OAuthClient _auth;
    readonly string _endpoint;
    int _id;
    bool _initialized;

    public McpClient(HttpClient http, OAuthClient auth, string baseUrl)
    {
        _http = http;
        _auth = auth;
        _endpoint = baseUrl.TrimEnd('/') + "/mcp";
    }

    /// <summary>呼叫 MCP 工具，回傳工具結果（structuredContent，或把文字內容當 JSON 解析）。</summary>
    public async Task<JsonElement> CallToolAsync(string name, object? arguments, CancellationToken ct)
    {
        if (!_initialized)
        {
            try
            {
                await RpcAsync("initialize", new
                {
                    protocolVersion = ProtocolVersion,
                    capabilities = new { },
                    clientInfo = new { name = "HayakuBar", version = "1.0.0" },
                }, ct);
            }
            catch (AuthRequiredException)
            {
                throw;
            }
            catch (Exception ex)
            {
                Log.Info($"initialize 失敗（無狀態伺服器可忽略）：{ex.Message}");
            }
            _initialized = true;
        }

        var result = await RpcAsync("tools/call", new { name, arguments = arguments ?? new { } }, ct, statName: name);
        var text = ExtractText(result);
        if (result.TryGetProperty("isError", out var isError) && isError.ValueKind == JsonValueKind.True)
            throw new McpException(text ?? $"{name} 失敗");
        if (result.TryGetProperty("structuredContent", out var structured) && structured.ValueKind == JsonValueKind.Object)
            return structured.Clone();
        if (text is null)
            return default;
        try
        {
            using var doc = JsonDocument.Parse(text);
            return doc.RootElement.Clone();
        }
        catch (JsonException)
        {
            return JsonSerializer.SerializeToElement(text);
        }
    }

    /// <summary>列出伺服器提供的所有工具（診斷用）。</summary>
    public Task<JsonElement> ListToolsAsync(CancellationToken ct) => RpcAsync("tools/list", new { }, ct);

    /// <summary>statName：統計請求數用的名稱（tools/call 傳工具名稱），預設用 method。</summary>
    async Task<JsonElement> RpcAsync(string method, object parameters, CancellationToken ct, string? statName = null)
    {
        for (var attempt = 0; ; attempt++)
        {
            var token = attempt == 0 ? await _auth.GetAccessTokenAsync(ct) : await _auth.ForceRefreshAsync(ct);
            McpStats.Record(statName ?? method);
            using var request = new HttpRequestMessage(HttpMethod.Post, _endpoint);
            request.Headers.Authorization = new("Bearer", token);
            request.Headers.Accept.ParseAdd("application/json");
            request.Headers.Accept.ParseAdd("text/event-stream");
            request.Headers.Add("MCP-Protocol-Version", ProtocolVersion);
            request.Content = JsonContent.Create(new
            {
                jsonrpc = "2.0",
                id = Interlocked.Increment(ref _id),
                method,
                @params = parameters,
            });

            using var response = await _http.SendAsync(request, ct);
            var body = await response.Content.ReadAsStringAsync(ct);
            if (response.StatusCode == HttpStatusCode.Unauthorized)
            {
                if (attempt == 0)
                    continue;
                throw new AuthRequiredException("登入已失效，請重新登入");
            }
            if (!response.IsSuccessStatusCode)
                throw new McpException($"Hayaku 回應 {(int)response.StatusCode}：{Shorten(ErrorMessage(body))}");

            var json = response.Content.Headers.ContentType?.MediaType == "text/event-stream" ? FromSse(body) : body;
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            if (root.TryGetProperty("error", out var error))
                throw new McpException(error.TryGetProperty("message", out var m) ? m.GetString() ?? "MCP 錯誤" : "MCP 錯誤");
            return root.GetProperty("result").Clone();
        }
    }

    static string? ExtractText(JsonElement result)
    {
        if (!result.TryGetProperty("content", out var content) || content.ValueKind != JsonValueKind.Array)
            return null;
        foreach (var item in content.EnumerateArray())
            if (item.TryGetProperty("type", out var type) && type.GetString() == "text")
                return item.GetProperty("text").GetString();
        return null;
    }

    static string FromSse(string body)
    {
        string? last = null;
        foreach (var line in body.Split('\n'))
            if (line.StartsWith("data:") && line.Contains("\"jsonrpc\""))
                last = line[5..].Trim();
        return last ?? throw new McpException("MCP 回應沒有資料");
    }

    static string ErrorMessage(string body)
    {
        try
        {
            using var doc = JsonDocument.Parse(body);
            if (doc.RootElement.TryGetProperty("message", out var m) && m.GetString() is { } s)
                return s;
        }
        catch (JsonException)
        {
        }
        return body;
    }

    static string Shorten(string s) => s.Length > 200 ? s[..200] + "…" : s;
}
