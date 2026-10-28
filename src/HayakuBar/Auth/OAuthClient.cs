using System.Diagnostics;
using System.Net.Http.Json;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Web;
using HayakuBar.Infra;

namespace HayakuBar.Auth;

public sealed class AuthRequiredException(string message) : Exception(message);

/// <summary>
/// 以「已連線的應用程式」身分登入 Hayaku：OAuth 2.1 授權碼 + PKCE，動態註冊 public client，
/// 回呼走本機 127.0.0.1。拿到的 token 對象是 /mcp。
/// </summary>
public sealed class OAuthClient
{
    public const int CallbackPort = 47824;
    const string Scope = "hayaku:read hayaku:write";

    readonly HttpClient _http;
    readonly string _baseUrl;
    readonly SemaphoreSlim _lock = new(1, 1);
    AuthData _data;
    JsonElement? _meta;

    public OAuthClient(HttpClient http, string baseUrl)
    {
        _http = http;
        _baseUrl = baseUrl.TrimEnd('/');
        _data = TokenStore.Load();
    }

    public string Resource => _baseUrl + "/mcp";

    public bool HasSession => !string.IsNullOrEmpty(_data.AccessToken) || !string.IsNullOrEmpty(_data.RefreshToken);

    public async Task<string> GetAccessTokenAsync(CancellationToken ct)
    {
        await _lock.WaitAsync(ct);
        try
        {
            if (!string.IsNullOrEmpty(_data.AccessToken) && _data.ExpiresAt > DateTimeOffset.UtcNow.AddSeconds(60))
                return _data.AccessToken;
            if (await RefreshCoreAsync(ct))
                return _data.AccessToken!;
            throw new AuthRequiredException("需要登入 Hayaku");
        }
        finally
        {
            _lock.Release();
        }
    }

    /// <summary>伺服器回 401 時呼叫：強制用 refresh token 換新的。</summary>
    public async Task<string> ForceRefreshAsync(CancellationToken ct)
    {
        await _lock.WaitAsync(ct);
        try
        {
            if (await RefreshCoreAsync(ct))
                return _data.AccessToken!;
            _data.AccessToken = null;
            TokenStore.Save(_data);
            throw new AuthRequiredException("登入已失效，請重新登入");
        }
        finally
        {
            _lock.Release();
        }
    }

    public async Task LoginAsync(CancellationToken ct)
    {
        var meta = await MetaAsync(ct);
        var redirect = $"http://127.0.0.1:{CallbackPort}/callback";
        if (string.IsNullOrEmpty(_data.ClientId) || _data.RedirectUri != redirect)
            await RegisterAsync(meta, redirect, ct);

        var verifier = Base64Url(RandomNumberGenerator.GetBytes(32));
        var challenge = Base64Url(SHA256.HashData(Encoding.ASCII.GetBytes(verifier)));
        var state = Base64Url(RandomNumberGenerator.GetBytes(16));
        var result = new TaskCompletionSource<(string? Code, string? Error)>(TaskCreationOptions.RunContinuationsAsynchronously);

        using var server = new MiniHttpServer(CallbackPort, req =>
        {
            if (req.Path != "/callback")
                return Task.FromResult(new MiniResponse(404, "text/plain", "not found"));
            var q = HttpUtility.ParseQueryString(req.Query);
            if (q["state"] != state)
                return Task.FromResult(Page("登入失敗", "state 不符，請回到 HayakuBar 重新登入。"));
            var error = q["error"];
            result.TrySetResult((q["code"], error is null ? null : $"{error} {q["error_description"]}".Trim()));
            return Task.FromResult(error is null
                ? Page("登入完成", "可以關閉這個分頁，回到 HayakuBar。")
                : Page("登入失敗", error));
        });
        try
        {
            server.Start();
        }
        catch (Exception ex)
        {
            throw new InvalidOperationException($"本機連接埠 {CallbackPort} 被占用，無法接收登入結果", ex);
        }

        var url = Str(meta, "authorization_endpoint") + "?" + Query(new()
        {
            ["response_type"] = "code",
            ["client_id"] = _data.ClientId!,
            ["redirect_uri"] = redirect,
            ["scope"] = Scope,
            ["state"] = state,
            ["code_challenge"] = challenge,
            ["code_challenge_method"] = "S256",
            ["resource"] = Resource,
        });
        Process.Start(new ProcessStartInfo(url) { UseShellExecute = true });

        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(ct);
        timeout.CancelAfter(TimeSpan.FromMinutes(5));
        var (code, err) = await result.Task.WaitAsync(timeout.Token);
        if (err is not null || string.IsNullOrEmpty(code))
            throw new InvalidOperationException($"授權失敗：{err ?? "沒有收到授權碼"}");

        var form = new Dictionary<string, string>
        {
            ["grant_type"] = "authorization_code",
            ["code"] = code,
            ["redirect_uri"] = redirect,
            ["client_id"] = _data.ClientId!,
            ["code_verifier"] = verifier,
            ["resource"] = Resource,
        };
        using var res = await _http.PostAsync(Str(meta, "token_endpoint"), new FormUrlEncodedContent(form), ct);
        var body = await res.Content.ReadAsStringAsync(ct);
        if (!res.IsSuccessStatusCode)
        {
            if (body.Contains("invalid_client"))
            {
                // 伺服器端已撤銷這個應用程式，下次重新註冊
                _data = new AuthData();
                TokenStore.Save(_data);
            }
            throw new InvalidOperationException($"換取 token 失敗（{(int)res.StatusCode}）：{body}");
        }
        ApplyToken(body);
        Log.Info("登入成功");
    }

    public async Task LogoutAsync()
    {
        try
        {
            if (!string.IsNullOrEmpty(_data.RefreshToken) && !string.IsNullOrEmpty(_data.ClientId))
            {
                var meta = await MetaAsync(CancellationToken.None);
                var form = new Dictionary<string, string>
                {
                    ["token"] = _data.RefreshToken,
                    ["token_type_hint"] = "refresh_token",
                    ["client_id"] = _data.ClientId,
                };
                using var res = await _http.PostAsync(Str(meta, "revocation_endpoint"), new FormUrlEncodedContent(form));
            }
        }
        catch (Exception ex)
        {
            Log.Error("撤銷 token 失敗", ex);
        }
        _data.AccessToken = null;
        _data.RefreshToken = null;
        TokenStore.Save(_data);
    }

    async Task<bool> RefreshCoreAsync(CancellationToken ct)
    {
        if (string.IsNullOrEmpty(_data.RefreshToken) || string.IsNullOrEmpty(_data.ClientId))
            return false;
        var meta = await MetaAsync(ct);
        var form = new Dictionary<string, string>
        {
            ["grant_type"] = "refresh_token",
            ["refresh_token"] = _data.RefreshToken,
            ["client_id"] = _data.ClientId,
            ["resource"] = Resource,
        };
        using var res = await _http.PostAsync(Str(meta, "token_endpoint"), new FormUrlEncodedContent(form), ct);
        var body = await res.Content.ReadAsStringAsync(ct);
        if (!res.IsSuccessStatusCode)
        {
            Log.Error($"refresh 失敗（{(int)res.StatusCode}）：{body}");
            if ((int)res.StatusCode is 400 or 401)
            {
                _data.AccessToken = null;
                _data.RefreshToken = null;
                TokenStore.Save(_data);
            }
            return false;
        }
        ApplyToken(body);
        return true;
    }

    async Task RegisterAsync(JsonElement meta, string redirect, CancellationToken ct)
    {
        var payload = new Dictionary<string, object>
        {
            ["client_name"] = "HayakuBar 工時小工具",
            ["redirect_uris"] = new[] { redirect },
            ["grant_types"] = new[] { "authorization_code", "refresh_token" },
            ["response_types"] = new[] { "code" },
            ["token_endpoint_auth_method"] = "none",
            ["application_type"] = "native",
            ["scope"] = Scope,
        };
        using var res = await _http.PostAsJsonAsync(Str(meta, "registration_endpoint"), payload, ct);
        var body = await res.Content.ReadAsStringAsync(ct);
        if (!res.IsSuccessStatusCode)
            throw new InvalidOperationException($"註冊應用程式失敗（{(int)res.StatusCode}）：{body}");
        using var doc = JsonDocument.Parse(body);
        _data = new AuthData
        {
            ClientId = doc.RootElement.GetProperty("client_id").GetString(),
            RedirectUri = redirect,
        };
        TokenStore.Save(_data);
        Log.Info($"已註冊應用程式 {_data.ClientId}");
    }

    void ApplyToken(string body)
    {
        using var doc = JsonDocument.Parse(body);
        var root = doc.RootElement;
        _data.AccessToken = root.GetProperty("access_token").GetString();
        if (root.TryGetProperty("refresh_token", out var rt) && rt.ValueKind == JsonValueKind.String)
            _data.RefreshToken = rt.GetString();
        var seconds = root.TryGetProperty("expires_in", out var e) && e.TryGetInt32(out var s) ? s : 3600;
        _data.ExpiresAt = DateTimeOffset.UtcNow.AddSeconds(seconds);
        TokenStore.Save(_data);
    }

    async Task<JsonElement> MetaAsync(CancellationToken ct)
    {
        if (_meta is { } cached)
            return cached;
        var json = await _http.GetStringAsync(_baseUrl + "/.well-known/oauth-authorization-server", ct);
        using var doc = JsonDocument.Parse(json);
        var meta = doc.RootElement.Clone();
        _meta = meta;
        return meta;
    }

    static string Str(JsonElement e, string name) =>
        e.TryGetProperty(name, out var v) && v.GetString() is { } s ? s : throw new InvalidOperationException($"OAuth 設定缺少 {name}");

    static string Query(Dictionary<string, string> values) =>
        string.Join("&", values.Select(kv => $"{Uri.EscapeDataString(kv.Key)}={Uri.EscapeDataString(kv.Value)}"));

    static string Base64Url(byte[] bytes) =>
        Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');

    static MiniResponse Page(string title, string message) => new(200, "text/html",
        $"<!doctype html><html lang=\"zh-Hant-TW\"><meta charset=\"utf-8\"><title>{title}</title>" +
        "<body style=\"font-family:system-ui,'Microsoft JhengHei';background:#f1eee8;color:#2b2925;display:grid;place-items:center;height:100vh;margin:0\">" +
        $"<div style=\"text-align:center\"><h2>{HttpUtility.HtmlEncode(title)}</h2><p>{HttpUtility.HtmlEncode(message)}</p></div></body></html>");
}
