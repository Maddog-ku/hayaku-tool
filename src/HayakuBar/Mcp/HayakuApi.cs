using System.Text.Json;
using HayakuBar.Core;

namespace HayakuBar.Mcp;

public sealed record TimeEntry(
    string Id,
    string Task,
    string Title,
    DateTimeOffset? StartedAt,
    DateTimeOffset? EndedAt,
    int? DurationMinutes,
    bool Running,
    string? StopReason);

public sealed record WorkSegment(TimeOnly Start, TimeOnly End);

public sealed record DaySchedule(
    DateOnly Date,
    IReadOnlyList<WorkSegment> Working,
    int ExpectedMinutes,
    bool OnLeave,
    string? Holiday);

public sealed record TaskItem(string Key, string Title, string Project, string Status);

public sealed record UserInfo(string Username, string DisplayName);

/// <summary>把需要的 Hayaku MCP 工具包成強型別方法。</summary>
public sealed class HayakuApi(McpClient mcp)
{
    static string Iso(DateOnly d) => d.ToString("yyyy-MM-dd");

    public async Task<UserInfo> WhoAmIAsync(CancellationToken ct)
    {
        var r = await mcp.CallToolAsync("whoami", null, ct);
        var username = Str(r, "username") ?? "";
        return new UserInfo(username, Str(r, "displayName") is { Length: > 0 } d ? d : username);
    }

    public async Task<DaySchedule> GetScheduleAsync(DateOnly day, CancellationToken ct)
    {
        var r = await mcp.CallToolAsync("get_my_schedule", new { from = Iso(day), to = Iso(day) }, ct);
        if (r.TryGetProperty("days", out var days) && days.ValueKind == JsonValueKind.Array && days.GetArrayLength() > 0)
            return ParseDay(days[0], day);
        return new DaySchedule(day, [], 0, false, null);
    }

    public async Task<List<TimeEntry>> ListTimeEntriesAsync(DateOnly from, DateOnly to, CancellationToken ct)
    {
        var r = await mcp.CallToolAsync("list_time_entries", new { from = Iso(from), to = Iso(to) }, ct);
        return ParseEntries(r, "entries");
    }

    public async Task<List<TimeEntry>> RunningTimersAsync(CancellationToken ct)
    {
        var r = await mcp.CallToolAsync("get_running_timers", null, ct);
        return ParseEntries(r, "running");
    }

    public Task StartTimerAsync(string task, CancellationToken ct) =>
        mcp.CallToolAsync("start_timer", new { task }, ct);

    /// <summary>task 為 null 時暫停所有正在計時的。</summary>
    public Task PauseTimerAsync(string? task, CancellationToken ct) =>
        task is null ? mcp.CallToolAsync("pause_timer", null, ct) : mcp.CallToolAsync("pause_timer", new { task }, ct);

    public async Task<List<TaskItem>> MyTasksAsync(string tab, CancellationToken ct)
    {
        var r = await mcp.CallToolAsync("my_tasks", new { tab }, ct);
        var list = new List<TaskItem>();
        if (r.TryGetProperty("tasks", out var tasks) && tasks.ValueKind == JsonValueKind.Array)
            foreach (var t in tasks.EnumerateArray())
                if (Str(t, "task") is { } key)
                    list.Add(new TaskItem(key, Str(t, "title") ?? "", Str(t, "project") ?? "", Str(t, "status") ?? ""));
        return list;
    }

    public async Task<List<string>> ProjectKeysAsync(CancellationToken ct)
    {
        var r = await mcp.CallToolAsync("list_projects", new { includeArchived = true }, ct);
        var list = new List<string>();
        if (r.TryGetProperty("projects", out var projects) && projects.ValueKind == JsonValueKind.Array)
            foreach (var p in projects.EnumerateArray())
                if (Str(p, "key") is { } key)
                    list.Add(key);
        return list;
    }

    internal static DaySchedule ParseDay(JsonElement d, DateOnly fallback)
    {
        var date = Str(d, "date") is { } ds && DateOnly.TryParse(ds, out var parsed) ? parsed : fallback;
        var expected = d.TryGetProperty("expected", out var e) && e.ValueKind == JsonValueKind.Object
                       && e.TryGetProperty("minutes", out var m) && m.TryGetInt32(out var mins) ? mins : 0;
        var onLeave = d.TryGetProperty("onLeave", out var l) && l.ValueKind == JsonValueKind.True;
        string? holiday = null;
        if (d.TryGetProperty("holiday", out var h))
            holiday = h.ValueKind switch
            {
                JsonValueKind.String => h.GetString(),
                JsonValueKind.Object => Str(h, "name") ?? "假日",
                _ => null,
            };
        return new DaySchedule(date, WorkMath.ParseSegments(Str(d, "working")), expected, onLeave, holiday);
    }

    internal static List<TimeEntry> ParseEntries(JsonElement r, string property)
    {
        var list = new List<TimeEntry>();
        if (!r.TryGetProperty(property, out var arr) || arr.ValueKind != JsonValueKind.Array)
            return list;
        foreach (var e in arr.EnumerateArray())
        {
            var task = Str(e, "task");
            if (task is null)
                continue;
            int? duration = e.TryGetProperty("duration", out var d) && d.ValueKind == JsonValueKind.Object
                            && d.TryGetProperty("minutes", out var m) && m.TryGetInt32(out var mins) ? mins : null;
            list.Add(new TimeEntry(
                Str(e, "id") ?? Guid.NewGuid().ToString(),
                task,
                Str(e, "title") ?? "",
                Date(e, "startedAt"),
                Date(e, "endedAt"),
                duration,
                e.TryGetProperty("running", out var run) && run.ValueKind == JsonValueKind.True,
                Str(e, "stopReason")));
        }
        return list;
    }

    static string? Str(JsonElement e, string name) =>
        e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    static DateTimeOffset? Date(JsonElement e, string name) =>
        Str(e, name) is { } s && DateTimeOffset.TryParse(s, out var d) ? d : null;
}
