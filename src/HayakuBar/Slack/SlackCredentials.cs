using System.Security.Cryptography;
using System.Text.Json;
using HayakuBar.Infra;

namespace HayakuBar.Slack;

/// <summary>Slack App 的兩組金鑰，和要把私訊傳給誰（成員 ID）。Team 只是顯示用。</summary>
public sealed class SlackCredentials
{
    /// <summary>Bot User OAuth Token（xoxb-），用來傳訊息、改訊息。</summary>
    public string BotToken { get; set; } = "";

    /// <summary>App-Level Token（xapp-，connections:write），用來連 Socket Mode 收按鈕。</summary>
    public string AppToken { get; set; } = "";

    /// <summary>自己的 Slack 成員 ID（U 開頭）。</summary>
    public string UserId { get; set; } = "";

    public string Team { get; set; } = "";

    [System.Text.Json.Serialization.JsonIgnore]
    public bool IsEmpty => BotToken.Length == 0 || AppToken.Length == 0 || UserId.Length == 0;
}

/// <summary>Slack 金鑰用 Windows DPAPI（目前使用者）加密後存檔，別的帳號或電腦解不開。</summary>
public static class SlackCredentialStore
{
    static readonly byte[] Entropy = "HayakuBar.Slack.v1"u8.ToArray();

    public static SlackCredentials Load()
    {
        try
        {
            if (!File.Exists(AppPaths.SlackAuth))
                return new SlackCredentials();
            var raw = ProtectedData.Unprotect(File.ReadAllBytes(AppPaths.SlackAuth), Entropy, DataProtectionScope.CurrentUser);
            return JsonSerializer.Deserialize<SlackCredentials>(raw, JsonFile.Options) ?? new SlackCredentials();
        }
        catch (Exception ex)
        {
            Log.Error("讀取 Slack 金鑰失敗", ex);
            return new SlackCredentials();
        }
    }

    public static void Save(SlackCredentials data)
    {
        var raw = JsonSerializer.SerializeToUtf8Bytes(data, JsonFile.Options);
        File.WriteAllBytes(AppPaths.SlackAuth, ProtectedData.Protect(raw, Entropy, DataProtectionScope.CurrentUser));
    }

    public static void Clear()
    {
        try
        {
            File.Delete(AppPaths.SlackAuth);
        }
        catch (Exception ex)
        {
            Log.Error("刪除 Slack 金鑰失敗", ex);
        }
    }
}
