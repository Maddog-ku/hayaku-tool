namespace HayakuBar.Infra;

public static class AppPaths
{
    public static string Dir { get; } = CreateDir();

    public static string Config => Path.Combine(Dir, "config.json");
    public static string State => Path.Combine(Dir, "state.json");
    public static string Auth => Path.Combine(Dir, "auth.bin");
    /// <summary>紀錄資料夾，一天一個檔（20261029093301.log）。</summary>
    public static string Logs => Path.Combine(Dir, "logs");

    static string CreateDir()
    {
        var dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "HayakuBar");
        Directory.CreateDirectory(dir);
        return dir;
    }
}
