using System.Diagnostics;

namespace HayakuBar.Infra;

public static class Web
{
    /// <summary>用預設瀏覽器（或檔案總管）開啟。</summary>
    public static void Open(string target)
    {
        try
        {
            Process.Start(new ProcessStartInfo(target) { UseShellExecute = true });
        }
        catch (Exception ex)
        {
            Log.Error($"開啟 {target} 失敗", ex);
        }
    }
}
