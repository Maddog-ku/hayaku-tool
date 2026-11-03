using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;

namespace HayakuBar.Slack;

/// <summary>Slack Web API 回 ok:false 時丟出；Error 是 Slack 的錯誤代碼（invalid_auth…）。</summary>
public sealed class SlackException(string method, string error) : Exception($"{method}：{SlackClient.Describe(error)}")
{
    public string Error { get; } = error;
}

/// <summary>
/// Slack Web API（https://slack.com/api/）。Bot Token 用來開私訊、傳訊息；App-Level Token 只用來拿 Socket Mode 的連線網址。
/// </summary>
public sealed class SlackClient : IDisposable
{
    static readonly Uri BaseUri = new("https://slack.com/api/");

    /// <summary>在 Slack「Create New App › From a manifest」貼上的 App 設定：私訊權限、Socket Mode、按鈕互動。</summary>
    public const string Manifest = """
        {
          "display_information": {
            "name": "HayakuBar",
            "description": "把 Hayaku 的通知、審核、測試結果傳到 Slack，可以直接按已讀、核准、駁回。"
          },
          "features": {
            "app_home": {
              "home_tab_enabled": false,
              "messages_tab_enabled": true,
              "messages_tab_read_only_enabled": true
            },
            "bot_user": {
              "display_name": "HayakuBar",
              "always_online": false
            }
          },
          "oauth_config": {
            "scopes": {
              "bot": ["chat:write", "im:write"]
            }
          },
          "settings": {
            "interactivity": { "is_enabled": true },
            "org_deploy_enabled": false,
            "socket_mode_enabled": true,
            "token_rotation_enabled": false
          }
        }
        """;

    public const string AppsUrl = "https://api.slack.com/apps?new_app=1";

    readonly HttpClient _http = new() { BaseAddress = BaseUri, Timeout = TimeSpan.FromSeconds(20) };

    /// <summary>檢查 Bot Token，回工作區名稱。</summary>
    public async Task<string> AuthTestAsync(string botToken, CancellationToken ct)
    {
        var r = await CallAsync("auth.test", botToken, null, ct);
        return r.TryGetProperty("team", out var team) ? team.GetString() ?? "" : "";
    }

    /// <summary>用 App-Level Token 拿 Socket Mode 的 WebSocket 網址（每次連線都要重拿）。</summary>
    public async Task<string> OpenSocketUrlAsync(string appToken, CancellationToken ct)
    {
        var r = await CallAsync("apps.connections.open", appToken, null, ct);
        return r.GetProperty("url").GetString() ?? throw new SlackException("apps.connections.open", "no_url");
    }

    /// <summary>開（或拿到已經有的）跟這個人的私訊頻道。</summary>
    public async Task<string> OpenDmAsync(string botToken, string userId, CancellationToken ct)
    {
        var r = await CallAsync("conversations.open", botToken, new { users = userId }, ct);
        return r.GetProperty("channel").GetProperty("id").GetString() ?? throw new SlackException("conversations.open", "channel_not_found");
    }

    /// <summary>傳訊息，回訊息的 ts（之後改訊息要用）。text 是通知列與無法顯示 blocks 時的文字。</summary>
    public async Task<string> PostMessageAsync(string botToken, string channel, string text, object? blocks, CancellationToken ct)
    {
        var r = await CallAsync("chat.postMessage", botToken, new { channel, text, blocks, unfurl_links = false }, ct);
        return r.GetProperty("ts").GetString() ?? "";
    }

    /// <summary>改已經傳出去的訊息（處理完把按鈕換成結果）。</summary>
    public Task UpdateMessageAsync(string botToken, string channel, string ts, string text, object? blocks, CancellationToken ct) =>
        CallAsync("chat.update", botToken, new { channel, ts, text, blocks }, ct);

    /// <summary>跳出表單（駁回、退回要填原因）；triggerId 只有 3 秒有效。</summary>
    public Task OpenViewAsync(string botToken, string triggerId, object view, CancellationToken ct) =>
        CallAsync("views.open", botToken, new { trigger_id = triggerId, view }, ct);

    async Task<JsonElement> CallAsync(string method, string token, object? body, CancellationToken ct)
    {
        using var request = new HttpRequestMessage(HttpMethod.Post, method)
        {
            Content = body is null
                ? new FormUrlEncodedContent([])
                : new StringContent(JsonSerializer.Serialize(body, BodyOptions), Encoding.UTF8, "application/json"),
        };
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
        using var response = await _http.SendAsync(request, ct);
        if ((int)response.StatusCode == 429)
            throw new SlackException(method, "ratelimited");
        response.EnsureSuccessStatusCode();
        using var doc = JsonDocument.Parse(await response.Content.ReadAsStringAsync(ct));
        return Check(method, doc.RootElement.Clone());
    }

    static readonly JsonSerializerOptions BodyOptions = new()
    {
        DefaultIgnoreCondition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull,
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    };

    /// <summary>Slack 一律回 HTTP 200，成功與否看 ok；失敗時丟出帶錯誤代碼的例外。</summary>
    internal static JsonElement Check(string method, JsonElement r)
    {
        if (r.TryGetProperty("ok", out var ok) && ok.ValueKind == JsonValueKind.True)
            return r;
        var error = r.TryGetProperty("error", out var e) ? e.GetString() ?? "unknown" : "unknown";
        throw new SlackException(method, error);
    }

    /// <summary>常見的錯誤代碼換成看得懂的說明；不認得的直接顯示代碼。</summary>
    public static string Describe(string error) => error switch
    {
        "invalid_auth" or "not_authed" => "金鑰不對，請重新複製",
        "token_revoked" or "token_expired" or "account_inactive" => "金鑰已失效（App 被移除或金鑰被撤銷），請重新產生",
        "not_allowed_token_type" => "金鑰種類放錯了：Bot Token 是 xoxb- 開頭，App-Level Token 是 xapp- 開頭",
        "missing_scope" => "App 少了權限，請用「複製 App 設定」的內容更新 App 後重新安裝",
        "user_not_found" or "channel_not_found" => "找不到這個成員 ID，請在 Slack 個人檔案重新複製",
        "cannot_dm_bot" => "成員 ID 是機器人，請填你自己的",
        "ratelimited" => "Slack 暫時限制傳送次數，等一下再試",
        _ => error,
    };

    public void Dispose() => _http.Dispose();
}
