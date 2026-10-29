using System.Globalization;

namespace HayakuBar.Infra;

/// <summary>
/// 紀錄寫在 logs\ 裡，一天一個檔，檔名是那個檔第一筆的時間（20261029093301.log），依日期就能找到。
/// 同一天超過 5MB 會再開一個新檔；舊檔由 Housekeeping 保留 2 個月。
/// </summary>
public static class Log
{
    internal const long MaxBytes = 5_000_000;
    const string NameFormat = "yyyyMMddHHmmss";

    static readonly object Gate = new();
    static string? _path;
    static DateOnly _day;

    public static void Info(string message) => Write("INFO", message);

    public static void Error(string message, Exception? ex = null) =>
        Write("ERROR", ex is null ? message : $"{message}: {ex}");

    static void Write(string level, string message)
    {
        try
        {
            lock (Gate)
            {
                var now = DateTime.Now;
                var today = DateOnly.FromDateTime(now);
                if (_path is null || _day != today || new FileInfo(_path) is { Exists: true, Length: > MaxBytes })
                {
                    _path = PickFile(AppPaths.Logs, now, reuseToday: _path is null);
                    _day = today;
                }
                File.AppendAllText(_path, $"{now:yyyy-MM-dd HH:mm:ss} [{level}] {message}{Environment.NewLine}");
            }
        }
        catch
        {
            // 記錄失敗不能影響主流程
        }
    }

    /// <summary>
    /// 要寫哪個檔。reuseToday（程式剛啟動）：今天已經有、還沒滿的檔就接著寫；否則用現在的時間開新檔。
    /// </summary>
    internal static string PickFile(string logsDir, DateTime now, bool reuseToday)
    {
        Directory.CreateDirectory(logsDir);
        if (reuseToday)
        {
            var latest = new DirectoryInfo(logsDir).EnumerateFiles($"{now:yyyyMMdd}*.log")
                .Where(f => f.Length <= MaxBytes).MaxBy(f => f.Name, StringComparer.Ordinal);
            if (latest is not null)
                return latest.FullName;
        }
        return FreePath(logsDir, now);
    }

    /// <summary>logs\20261029093301.log；同一秒已經有了就加 -2、-3…。</summary>
    internal static string FreePath(string logsDir, DateTime at)
    {
        var path = Path.Combine(logsDir, $"{at.ToString(NameFormat, CultureInfo.InvariantCulture)}.log");
        for (var n = 2; File.Exists(path); n++)
            path = Path.Combine(logsDir, $"{at.ToString(NameFormat, CultureInfo.InvariantCulture)}-{n}.log");
        return path;
    }

    /// <summary>跟寫紀錄互斥地執行（搬移、刪除紀錄檔時用）。</summary>
    internal static void Exclusive(Action action)
    {
        lock (Gate)
            action();
    }

    /// <summary>紀錄檔第一行的時間（"2026-10-28 09:39:24 [INFO] …"）；沒有檔案或讀不懂回傳 null。</summary>
    internal static DateTime? FirstTime(string path)
    {
        try
        {
            using var reader = new StreamReader(new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete));
            var line = reader.ReadLine();
            return line is { Length: >= 19 } && DateTime.TryParseExact(line[..19], "yyyy-MM-dd HH:mm:ss",
                CultureInfo.InvariantCulture, DateTimeStyles.None, out var at) ? at : null;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            return null;
        }
    }
}
