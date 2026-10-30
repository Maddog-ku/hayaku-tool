using System.Text.Json;

namespace HayakuBar.Mcp;

/// <summary>
/// 統計表的一列（get_stats 的 rows）：依專案時 Key 是專案代碼；依人時是帳號。
/// 依專案才有預計、累計、任務數與完成率；依人才有應出勤、工作天、有計時天數與指派、完成數。兩邊都有實際工時與逾期。
/// </summary>
public sealed record StatsRow(
    string Key,
    string Name,
    int ActualMinutes,
    int EstimatedMinutes,
    int TotalLoggedMinutes,
    int ExpectedMinutes,
    int Workdays,
    int ActiveDays,
    int Assigned,
    int Done,
    int Overdue,
    double? CompletionRate);

/// <summary>某列在某個區間（日、週、月）的工時；依人時是那個人在各專案照實相加（同時計時會重複算）。</summary>
public sealed record StatsCell(string Key, string Bucket, int Minutes);

/// <summary>
/// 一次統計（get_stats）：Perspective 是 project（依專案）或 assignee（依執行者）；Buckets 是區間的名稱（日期或週、月的起日）。
/// ActualMinutes：依專案時用伺服器的合計；依人時把每人的實際（已經是時間聯集）加起來。
/// </summary>
public sealed record StatsReport(
    string Perspective,
    DateOnly? From,
    DateOnly? To,
    string Granularity,
    IReadOnlyList<string> Buckets,
    IReadOnlyList<StatsRow> Rows,
    IReadOnlyList<StatsCell> Cells,
    int ActualMinutes,
    int EstimatedMinutes,
    int ExpectedMinutes,
    int Done,
    int TaskTotal,
    int Overdue)
{
    public bool ByProject => Perspective == "project";
}

/// <summary>統計（跟網頁統計頁同一套數字）。</summary>
public sealed partial class HayakuApi
{
    /// <summary>perspective：project 或 assignee；preset（thisWeek、lastWeek、thisMonth…）或 from/to 擇一。</summary>
    public async Task<StatsReport> StatsAsync(string perspective, string? preset, DateOnly? from, DateOnly? to, string granularity,
        CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["perspective"] = perspective, ["granularity"] = granularity };
        if (from is { } f && to is { } t)
        {
            args["from"] = Iso(f);
            args["to"] = Iso(t);
        }
        else
            args["preset"] = preset ?? "thisMonth";
        return ParseStats(await Mcp.CallToolAsync("get_stats", args, ct));
    }

    internal static StatsReport ParseStats(JsonElement r)
    {
        static int Minutes(JsonElement e, string name) =>
            e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.Object
            && v.TryGetProperty("minutes", out var m) && m.TryGetInt32(out var mins) ? mins : 0;
        static int Int(JsonElement e, string name) =>
            e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) && v.TryGetInt32(out var i) ? i : 0;

        var perspective = Str(r, "perspective") ?? "project";
        var byProject = perspective == "project";
        var range = r.TryGetProperty("range", out var rg) && rg.ValueKind == JsonValueKind.Object ? rg : default;
        var buckets = range.ValueKind == JsonValueKind.Object && range.TryGetProperty("buckets", out var b) && b.ValueKind == JsonValueKind.Array
            ? b.EnumerateArray().Where(x => x.ValueKind == JsonValueKind.String).Select(x => x.GetString()!).ToList()
            : [];
        var projectNames = Items(r, "projects").Where(p => Str(p, "key") is not null)
            .GroupBy(p => Str(p, "key")!).ToDictionary(g => g.Key, g => Str(g.First(), "name") ?? g.Key);
        var userNames = Items(r, "users").Where(u => Str(u, "username") is not null)
            .GroupBy(u => Str(u, "username")!).ToDictionary(g => g.Key, g => Str(g.First(), "displayName") ?? g.Key);
        string NameOf(string key) => byProject ? projectNames.GetValueOrDefault(key, key) : userNames.GetValueOrDefault(key, key);

        var rows = new List<StatsRow>();
        foreach (var row in Items(r, "rows"))
        {
            var key = Str(row, byProject ? "project" : "user") ?? "";
            var counts = row.TryGetProperty("taskCounts", out var tc) && tc.ValueKind == JsonValueKind.Object
                ? tc.EnumerateObject().Where(p => p.Value.TryGetInt32(out _)).ToDictionary(p => p.Name, p => p.Value.GetInt32())
                : new Dictionary<string, int>();
            var total = byProject ? counts.Values.Sum() : Int(row, "assignedCount");
            var done = byProject ? counts.GetValueOrDefault("DONE") : Int(row, "doneCount");
            rows.Add(new StatsRow(
                key,
                NameOf(key),
                Minutes(row, "actual"),
                Minutes(row, "estimated"),
                Minutes(row, "totalLogged"),
                Minutes(row, "expected"),
                Int(row, "workdays"),
                Int(row, "activeDays"),
                total,
                done,
                Int(row, "overdueCount"),
                row.TryGetProperty("completionRate", out var cr) && cr.TryGetDouble(out var rate) ? rate : null));
        }
        rows = rows.OrderByDescending(x => x.ActualMinutes).ThenBy(x => x.Name, StringComparer.CurrentCulture).ToList();

        var cells = Items(r, "cells")
            .Select(c => (Key: Str(c, byProject ? "project" : "user") ?? "", Bucket: Str(c, "bucket") ?? "", Minutes: Minutes(c, "duration")))
            .GroupBy(c => (c.Key, c.Bucket))
            .Select(g => new StatsCell(g.Key.Key, g.Key.Bucket, g.Sum(c => c.Minutes)))
            .ToList();

        var summary = r.TryGetProperty("summary", out var s) && s.ValueKind == JsonValueKind.Object ? s : default;
        var hasSummary = summary.ValueKind == JsonValueKind.Object;
        var summaryCounts = hasSummary && summary.TryGetProperty("taskCounts", out var sc) && sc.ValueKind == JsonValueKind.Object
            ? sc.EnumerateObject().Where(p => p.Value.TryGetInt32(out _)).ToDictionary(p => p.Name, p => p.Value.GetInt32())
            : null;
        return new StatsReport(
            perspective,
            Day(range, "from"),
            Day(range, "to"),
            range.ValueKind == JsonValueKind.Object ? Str(range, "granularity") ?? "day" : "day",
            buckets,
            rows,
            cells,
            hasSummary ? Minutes(summary, "actual") : rows.Sum(x => x.ActualMinutes),
            hasSummary ? Minutes(summary, "estimated") : rows.Sum(x => x.EstimatedMinutes),
            rows.Sum(x => x.ExpectedMinutes),
            summaryCounts?.GetValueOrDefault("DONE") ?? rows.Sum(x => x.Done),
            summaryCounts?.Values.Sum() ?? rows.Sum(x => x.Assigned),
            hasSummary ? Int(summary, "overdueCount") : rows.Sum(x => x.Overdue));
    }
}
