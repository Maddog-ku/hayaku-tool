using HayakuBar.Infra;

namespace HayakuBar.Mcp;

/// <summary>
/// 統計送到 Hayaku /mcp 的請求數（含 initialize 與 token 失效後的重送），每小時寫一行到紀錄，
/// 用來確認同步打了多少請求。
/// </summary>
public static class McpStats
{
    static readonly TimeSpan Window = TimeSpan.FromHours(1);
    static readonly Lock Sync = new();
    static readonly Dictionary<string, int> Counts = new(StringComparer.Ordinal);
    static DateTime _since = DateTime.Now;

    /// <summary>記一次請求（tools/call 用工具名稱，其他用方法名稱）；滿一小時就寫統計並重新計算。</summary>
    public static void Record(string name)
    {
        string? line;
        lock (Sync)
        {
            Counts[name] = Counts.GetValueOrDefault(name) + 1;
            line = DateTime.Now - _since >= Window ? TakeLocked() : null;
        }
        if (line is not null)
            Log.Info(line);
    }

    /// <summary>把目前累計的寫進紀錄（例如結束程式時）；沒有請求就不寫。</summary>
    public static void Flush()
    {
        string? line;
        lock (Sync)
            line = Counts.Count > 0 ? TakeLocked() : null;
        if (line is not null)
            Log.Info(line);
    }

    static string TakeLocked()
    {
        var now = DateTime.Now;
        var line = Format(Counts, now - _since);
        Counts.Clear();
        _since = now;
        return line;
    }

    /// <summary>"MCP 請求（過去 60 分鐘）：共 412 次 · list_time_entries 120、get_running_timers 120…"（多的排前面）。</summary>
    public static string Format(IReadOnlyDictionary<string, int> counts, TimeSpan elapsed)
    {
        var total = counts.Values.Sum();
        var detail = counts.OrderByDescending(p => p.Value).ThenBy(p => p.Key, StringComparer.Ordinal)
            .Select(p => $"{p.Key} {p.Value}");
        return $"MCP 請求（過去 {Math.Max(1, (int)Math.Round(elapsed.TotalMinutes))} 分鐘）：共 {total} 次 · {string.Join("、", detail)}";
    }
}
