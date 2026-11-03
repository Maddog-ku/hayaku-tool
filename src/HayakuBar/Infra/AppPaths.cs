namespace HayakuBar.Infra;

public static class AppPaths
{
    public static string Dir { get; } = CreateDir();

    public static string Config => Path.Combine(Dir, "config.json");
    public static string State => Path.Combine(Dir, "state.json");
    public static string Auth => Path.Combine(Dir, "auth.bin");
    /// <summary>EIP 帳密（DPAPI 加密）。</summary>
    public static string EipAuth => Path.Combine(Dir, "eip.bin");
    /// <summary>EIP 請假同步的狀態（上次同步、由同步建立的請假）。</summary>
    public static string EipSync => Path.Combine(Dir, "eip-sync.json");
    /// <summary>Slack App 的金鑰與要傳給誰（DPAPI 加密）。</summary>
    public static string SlackAuth => Path.Combine(Dir, "slack.bin");
    /// <summary>傳到 Slack 的訊息（訊息編號對應哪個通知、處理了沒）。</summary>
    public static string SlackState => Path.Combine(Dir, "slack.json");
    /// <summary>紀錄資料夾，一天一個檔（20261029093301.log）。</summary>
    public static string Logs => Path.Combine(Dir, "logs");

    static string CreateDir()
    {
        var dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "HayakuBar");
        Directory.CreateDirectory(dir);
        return dir;
    }
}
