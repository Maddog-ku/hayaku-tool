using System.Security.Cryptography;
using System.Text.Json;
using HayakuBar.Infra;

namespace HayakuBar.Auth;

public sealed class AuthData
{
    public string? ClientId { get; set; }
    public string? RedirectUri { get; set; }
    public string? AccessToken { get; set; }
    public string? RefreshToken { get; set; }
    public DateTimeOffset ExpiresAt { get; set; }
}

/// <summary>登入資料用 Windows DPAPI（目前使用者）加密後存檔，別的帳號或電腦解不開。</summary>
public static class TokenStore
{
    static readonly byte[] Entropy = "HayakuBar.v1"u8.ToArray();

    public static AuthData Load()
    {
        try
        {
            if (!File.Exists(AppPaths.Auth))
                return new AuthData();
            var raw = ProtectedData.Unprotect(File.ReadAllBytes(AppPaths.Auth), Entropy, DataProtectionScope.CurrentUser);
            return JsonSerializer.Deserialize<AuthData>(raw, JsonFile.Options) ?? new AuthData();
        }
        catch (Exception ex)
        {
            Log.Error("讀取登入資料失敗", ex);
            return new AuthData();
        }
    }

    public static void Save(AuthData data)
    {
        var raw = JsonSerializer.SerializeToUtf8Bytes(data, JsonFile.Options);
        File.WriteAllBytes(AppPaths.Auth, ProtectedData.Protect(raw, Entropy, DataProtectionScope.CurrentUser));
    }
}
