using System.Security.Cryptography;
using System.Text.Json;
using HayakuBar.Infra;

namespace HayakuBar.Eip;

public sealed class EipCredentials
{
    public string Account { get; set; } = "";
    public string Password { get; set; } = "";

    [System.Text.Json.Serialization.JsonIgnore]
    public bool IsEmpty => Account.Length == 0 || Password.Length == 0;
}

/// <summary>EIP 帳密用 Windows DPAPI（目前使用者）加密後存檔，別的帳號或電腦解不開。</summary>
public static class EipCredentialStore
{
    static readonly byte[] Entropy = "HayakuBar.Eip.v1"u8.ToArray();

    public static EipCredentials Load()
    {
        try
        {
            if (!File.Exists(AppPaths.EipAuth))
                return new EipCredentials();
            var raw = ProtectedData.Unprotect(File.ReadAllBytes(AppPaths.EipAuth), Entropy, DataProtectionScope.CurrentUser);
            return JsonSerializer.Deserialize<EipCredentials>(raw, JsonFile.Options) ?? new EipCredentials();
        }
        catch (Exception ex)
        {
            Log.Error("讀取 EIP 帳密失敗", ex);
            return new EipCredentials();
        }
    }

    public static void Save(EipCredentials data)
    {
        var raw = JsonSerializer.SerializeToUtf8Bytes(data, JsonFile.Options);
        File.WriteAllBytes(AppPaths.EipAuth, ProtectedData.Protect(raw, Entropy, DataProtectionScope.CurrentUser));
    }

    public static void Clear()
    {
        try
        {
            File.Delete(AppPaths.EipAuth);
        }
        catch (Exception ex)
        {
            Log.Error("刪除 EIP 帳密失敗", ex);
        }
    }
}
