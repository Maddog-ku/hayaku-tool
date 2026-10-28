namespace HayakuBar.Infra;

public static class Log
{
    static readonly object Gate = new();

    public static void Info(string message) => Write("INFO", message);

    public static void Error(string message, Exception? ex = null) =>
        Write("ERROR", ex is null ? message : $"{message}: {ex}");

    static void Write(string level, string message)
    {
        try
        {
            lock (Gate)
            {
                var file = new FileInfo(AppPaths.Log);
                if (file.Exists && file.Length > 2_000_000)
                    File.Move(AppPaths.Log, AppPaths.Log + ".1", overwrite: true);
                File.AppendAllText(AppPaths.Log, $"{DateTime.Now:yyyy-MM-dd HH:mm:ss} [{level}] {message}{Environment.NewLine}");
            }
        }
        catch
        {
            // 記錄失敗不能影響主流程
        }
    }
}
