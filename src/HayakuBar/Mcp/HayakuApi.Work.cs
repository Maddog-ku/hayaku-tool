using System.Text.Json;
using HayakuBar.Core;

namespace HayakuBar.Mcp;

/// <summary>我的任務列表的一筆（my_tasks）。</summary>
public sealed record TaskSummary(
    string Key,
    string Title,
    string Project,
    string Status,
    string StatusCategory,
    string Priority,
    DateOnly? PlannedStart,
    DateOnly? PlannedEnd,
    bool Timing,
    string Path);

public sealed record PersonRef(string Username, string DisplayName);

/// <summary>任務完整資料（get_task）。</summary>
public sealed record TaskDetail(
    string Key,
    string Title,
    string Status,
    string StatusCategory,
    string Priority,
    IReadOnlyList<PersonRef> Assignees,
    DateOnly? PlannedStart,
    DateOnly? PlannedEnd,
    DateOnly? ActualStart,
    DateOnly? ActualEnd,
    double? EstimatedHours,
    string LoggedText,
    string? Note,
    string ApprovalState,
    IReadOnlyList<(string Key, string Title)> Path,
    IReadOnlyList<(string Key, string Title, string Status)> Children,
    int PendingChangeRequests);

public sealed record StatusOption(string Name, string Category, bool IsDefault);

public sealed record ProjectInfo(string Key, string Name, string Role)
{
    public override string ToString() => Name;
}

public sealed record ProjectDetail(string Key, string Name, IReadOnlyList<PersonRef> Members, IReadOnlyList<StatusOption> Statuses);

public sealed record SearchHit(string Key, string Title, string Project);

public sealed record ReminderFull(
    string Id,
    string Text,
    DateTimeOffset? At,
    string? Task,
    string? RepeatText,
    bool Paused,
    bool Editable);

public sealed record LeaveItem(string Id, PersonRef User, string Period, DateTimeOffset StartAt, DateTimeOffset EndAt, string LostText);

public sealed record TeamTiming(string Task, string Title, string Project, DateTimeOffset? Since, bool CanOpen);

public sealed record TeamMember(PersonRef User, string? JobTitle, bool Online, bool OnLeave, IReadOnlyList<string> Leaves, IReadOnlyList<TeamTiming> Timing);

public sealed record WorkLogEntry(string Task, int Minutes, IReadOnlyList<(DateTimeOffset Start, DateTimeOffset End)> Ranges);

public sealed record WorkLog(DateOnly Date, int TotalMinutes, IReadOnlyList<WorkLogEntry> Entries, string Markdown);

public sealed record ChatMessage(string Id, PersonRef Author, string Body, DateTimeOffset CreatedAt, bool Deleted, string? PollQuestion);

public sealed record StatsSummary(int ActualMinutes, int ExpectedMinutes, int ActiveDays, int Workdays, int Assigned, int Done, int Overdue,
    IReadOnlyList<(string Project, int Minutes)> ByProject);

/// <summary>任務、提醒、請假、團隊、工作日誌、聊天、統計。</summary>
public sealed partial class HayakuApi
{
    // ───────────── 任務 ─────────────

    public async Task<List<TaskSummary>> MyTaskListAsync(string tab, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("my_tasks", new { tab }, ct);
        return ParseTaskSummaries(r);
    }

    internal static List<TaskSummary> ParseTaskSummaries(JsonElement r)
    {
        var list = new List<TaskSummary>();
        foreach (var t in Items(r, "tasks"))
        {
            if (Str(t, "task") is not { } key)
                continue;
            var path = t.TryGetProperty("path", out var p) && p.ValueKind == JsonValueKind.Array
                ? string.Join(" › ", p.EnumerateArray().Select(x => Str(x, "title")).Where(x => x is not null))
                : "";
            list.Add(new TaskSummary(key, Str(t, "title") ?? "", Str(t, "project") ?? "", Str(t, "status") ?? "",
                Str(t, "statusCategory") ?? "", Str(t, "priority") ?? "", Day(t, "plannedStart"), Day(t, "plannedEnd"),
                t.TryGetProperty("timing", out var timing) && timing.ValueKind == JsonValueKind.True, path));
        }
        return list;
    }

    public async Task<TaskDetail> GetTaskAsync(string task, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("get_task", new { task }, ct);
        return ParseTaskDetail(r);
    }

    internal static TaskDetail ParseTaskDetail(JsonElement r)
    {
        var assignees = Items(r, "assignees").Select(Person).Where(p => p is not null).Select(p => p!).ToList();
        var path = Items(r, "path").Select(x => (Str(x, "task") ?? "", Str(x, "title") ?? "")).ToList();
        var children = Items(r, "children").Select(x => (Str(x, "task") ?? "", Str(x, "title") ?? "", Str(x, "status") ?? "")).ToList();
        double? estimated = null;
        if (r.TryGetProperty("estimated", out var est))
            estimated = est.ValueKind switch
            {
                JsonValueKind.Number => est.GetDouble(),
                JsonValueKind.Object when est.TryGetProperty("minutes", out var m) && m.TryGetDouble(out var mins) => Math.Round(mins / 60, 2),
                _ => null,
            };
        var logged = r.TryGetProperty("logged", out var lg) && lg.ValueKind == JsonValueKind.Object ? Str(lg, "display") ?? "" : "";
        return new TaskDetail(
            Str(r, "task") ?? "",
            Str(r, "title") ?? "",
            Str(r, "status") ?? "",
            Str(r, "statusCategory") ?? "",
            Str(r, "priority") ?? "",
            assignees,
            Day(r, "plannedStart"),
            Day(r, "plannedEnd"),
            Day(r, "actualStart"),
            Day(r, "actualEnd"),
            estimated,
            logged,
            Str(r, "note"),
            Str(r, "approvalState") ?? "",
            path,
            children,
            Items(r, "pendingChangeRequests").Count());
    }

    /// <summary>部分更新任務；fields 的值為 null 代表清空。回傳伺服器的說明文字（例如送出變更申請）。</summary>
    public async Task<string?> UpdateTaskAsync(string task, IDictionary<string, object?> fields, CancellationToken ct)
    {
        var args = new Dictionary<string, object?>(fields) { ["task"] = task };
        var r = await Mcp.CallToolAsync("update_task", args, ct);
        return Message(r);
    }

    public async Task<string> CreateTaskAsync(IDictionary<string, object?> fields, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("create_task", fields, ct);
        return Str(r, "task") ?? FirstStr(r, "key", "number") ?? "";
    }

    public async Task<List<SearchHit>> SearchTasksAsync(string query, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("search_tasks", new { query }, ct);
        return Items(r, "tasks")
            .Where(t => Str(t, "task") is not null)
            .Select(t => new SearchHit(Str(t, "task")!, Str(t, "title") ?? "", Str(t, "project") ?? ""))
            .ToList();
    }

    public async Task<List<ProjectInfo>> ListProjectsAsync(CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_projects", null, ct);
        return Items(r, "projects")
            .Where(p => Str(p, "key") is not null)
            .Select(p => new ProjectInfo(Str(p, "key")!, Str(p, "name") ?? "", Str(p, "role") ?? ""))
            .ToList();
    }

    public async Task<ProjectDetail> GetProjectAsync(string project, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("get_project", new { project }, ct);
        var members = Items(r, "members").Select(Person).Where(p => p is not null).Select(p => p!).ToList();
        var statuses = Items(r, "statuses")
            .Where(s => Str(s, "name") is not null)
            .Select(s => new StatusOption(Str(s, "name")!, Str(s, "category") ?? "",
                s.TryGetProperty("isDefault", out var d) && d.ValueKind == JsonValueKind.True))
            .ToList();
        return new ProjectDetail(Str(r, "key") ?? project, Str(r, "name") ?? project, members, statuses);
    }

    // ───────────── 提醒 ─────────────

    public async Task<List<ReminderFull>> ListRemindersFullAsync(string tab, string? task, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["tab"] = tab };
        if (task is not null)
            args["task"] = task;
        var r = await Mcp.CallToolAsync("list_reminders", args, ct);
        var list = new List<ReminderFull>();
        foreach (var t in Items(r, "reminders", "items"))
        {
            if (Str(t, "id") is not { } id)
                continue;
            string? repeat = null;
            var paused = false;
            if (t.TryGetProperty("repeat", out var rep) && rep.ValueKind == JsonValueKind.Object)
            {
                repeat = RepeatText(rep);
                paused = Str(rep, "pausedAt") is not null;
            }
            list.Add(new ReminderFull(
                id,
                FirstStr(t, "message", "label", "text", "title") ?? "提醒",
                FirstDate(t, "nextAt", "remindAt", "fireAt", "lastFiredAt", "at"),
                FirstKey(t, "task", "taskCode"),
                repeat,
                paused,
                !(t.TryGetProperty("editable", out var editable) && editable.ValueKind == JsonValueKind.False)));
        }
        return list;
    }

    /// <summary>重複規則 → 「每週四 09:00」這類文字（weekdays：0=週一…6=週日）。</summary>
    internal static string RepeatText(JsonElement rep)
    {
        var time = Str(rep, "timeOfDay") ?? "";
        if (!rep.TryGetProperty("rule", out var rule) || rule.ValueKind != JsonValueKind.Object)
            return $"重複 {time}".Trim();
        int Int(string name) => rule.TryGetProperty(name, out var v) && v.TryGetInt32(out var i) ? i : 0;
        var interval = Math.Max(1, Int("interval"));
        var names = "一二三四五六日";
        var text = Str(rule, "freq") switch
        {
            "DAILY" when rule.TryGetProperty("workdaysOnly", out var w) && w.ValueKind == JsonValueKind.True => "每個工作日",
            "DAILY" => interval == 1 ? "每天" : $"每 {interval} 天",
            "WEEKLY" => (interval == 1 ? "每週" : $"每 {interval} 週") + string.Join("、",
                Items(rule, "weekdays").Where(d => d.TryGetInt32(out var n) && n is >= 0 and <= 6).Select(d => names[d.GetInt32()].ToString())),
            "MONTHLY" when rule.TryGetProperty("lastDay", out _) => "每月最後一天",
            "MONTHLY" when Int("monthDay") > 0 => $"每月 {Int("monthDay")} 日",
            "MONTHLY" => "每月",
            _ => "重複",
        };
        return $"{text} {time}".Trim();
    }

    public Task CreateReminderAsync(IDictionary<string, object?> args, CancellationToken ct) =>
        Mcp.CallToolAsync("create_reminder", args, ct);

    public Task UpdateReminderAsync(IDictionary<string, object?> args, CancellationToken ct) =>
        Mcp.CallToolAsync("update_reminder", args, ct);

    public Task DeleteReminderAsync(string id, CancellationToken ct) =>
        Mcp.CallToolAsync("delete_reminder", new { id }, ct);

    // ───────────── 通知 ─────────────

    public Task ArchiveNotificationsAsync(IReadOnlyCollection<string> ids, CancellationToken ct) =>
        Mcp.CallToolAsync("archive_notifications", new { ids }, ct);

    public Task SnoozeNotificationsAsync(IReadOnlyCollection<string> ids, string preset, CancellationToken ct) =>
        Mcp.CallToolAsync("snooze_notifications", new { ids, preset }, ct);

    // ───────────── 請假與團隊 ─────────────

    public async Task<List<LeaveItem>> ListLeavesAsync(DateOnly from, DateOnly to, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_leaves", new { from = Iso(from), to = Iso(to) }, ct);
        var list = new List<LeaveItem>();
        foreach (var l in Items(r, "leaves"))
        {
            if (Str(l, "id") is not { } id || Date(l, "startAt") is not { } start || Date(l, "endAt") is not { } end)
                continue;
            var user = l.TryGetProperty("user", out var u) ? Person(u) : null;
            var lost = l.TryGetProperty("lost", out var lo) && lo.ValueKind == JsonValueKind.Object ? Str(lo, "display") ?? "" : "";
            list.Add(new LeaveItem(id, user ?? new PersonRef("", ""), Str(l, "period") ?? "", start, end, lost));
        }
        return list.OrderBy(l => l.StartAt).ToList();
    }

    public Task AddLeaveAsync(IDictionary<string, object?> args, CancellationToken ct) =>
        Mcp.CallToolAsync("add_leave", args, ct);

    public Task CancelLeaveAsync(string id, CancellationToken ct) =>
        Mcp.CallToolAsync("cancel_leave", new { id }, ct);

    public async Task<List<TeamMember>> TeamScheduleAsync(CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("team_schedule", null, ct);
        var list = new List<TeamMember>();
        foreach (var p in Items(r, "people"))
        {
            var user = new PersonRef(Str(p, "username") ?? "", Str(p, "displayName") ?? Str(p, "username") ?? "");
            var leaves = Items(p, "leaves").Where(x => x.ValueKind == JsonValueKind.String).Select(x => x.GetString()!).ToList();
            var timing = Items(p, "timing")
                .Select(t => new TeamTiming(Str(t, "task") ?? "", Str(t, "title") ?? "", Str(t, "project") ?? "", Date(t, "since"),
                    t.TryGetProperty("canOpen", out var co) && co.ValueKind == JsonValueKind.True))
                .ToList();
            list.Add(new TeamMember(user, Str(p, "jobTitle"),
                p.TryGetProperty("online", out var on) && on.ValueKind == JsonValueKind.True,
                p.TryGetProperty("onLeave", out var ol) && ol.ValueKind == JsonValueKind.True,
                leaves, timing));
        }
        return list;
    }

    public async Task<List<DaySchedule>> GetScheduleRangeAsync(DateOnly from, DateOnly to, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("get_my_schedule", new { from = Iso(from), to = Iso(to) }, ct);
        return Items(r, "days").Select(d => ParseDay(d, from)).ToList();
    }

    // ───────────── 工作日誌 ─────────────

    public async Task<WorkLog> GetWorkLogAsync(DateOnly date, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("get_work_log", new { date = Iso(date) }, ct);
        var entries = Items(r, "entries").Select(e => new WorkLogEntry(
            Str(e, "task") ?? "",
            e.TryGetProperty("minutes", out var m) && m.TryGetInt32(out var mins) ? mins : 0,
            Items(e, "ranges")
                .Where(x => Date(x, "start") is not null && Date(x, "end") is not null)
                .Select(x => (Date(x, "start")!.Value, Date(x, "end")!.Value))
                .ToList())).ToList();
        var total = r.TryGetProperty("totalMinutes", out var tm) && tm.TryGetInt32(out var t) ? t : 0;
        return new WorkLog(date, total, entries, Str(r, "markdown") ?? "");
    }

    // ───────────── 聊天 ─────────────

    public async Task<List<string>> ListChannelsAsync(string project, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_channels", new { project }, ct);
        return Items(r, "channels")
            .Where(c => Str(c, "name") is not null && !(c.TryGetProperty("archived", out var a) && a.ValueKind == JsonValueKind.True))
            .Select(c => Str(c, "name")!)
            .ToList();
    }

    /// <summary>給 task，或 project+channel；新到舊。</summary>
    public async Task<List<ChatMessage>> ListChatAsync(string? task, string? project, string? channel, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["limit"] = 50 };
        if (task is not null)
            args["task"] = task;
        else
        {
            args["project"] = project;
            args["channel"] = channel;
        }
        var r = await Mcp.CallToolAsync("list_chat", args, ct);
        var list = new List<ChatMessage>();
        foreach (var m in Items(r, "items", "messages"))
        {
            if (Str(m, "id") is not { } id)
                continue;
            var author = m.TryGetProperty("author", out var a) ? Person(a) : null;
            var poll = m.TryGetProperty("poll", out var p) && p.ValueKind == JsonValueKind.Object ? Str(p, "question") : null;
            list.Add(new ChatMessage(id, author ?? new PersonRef("", "系統"), Str(m, "body") ?? "", Date(m, "createdAt") ?? DateTimeOffset.Now,
                m.TryGetProperty("deletedAt", out var d) && d.ValueKind == JsonValueKind.String, poll));
        }
        return list;
    }

    public Task SendChatAsync(string? task, string? project, string? channel, string body, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["body"] = body };
        if (task is not null)
            args["task"] = task;
        else
        {
            args["project"] = project;
            args["channel"] = channel;
        }
        return Mcp.CallToolAsync("send_chat", args, ct);
    }

    // ───────────── 會議 ─────────────

    public async Task<List<MeetingItem>> UpcomingMeetingsAsync(CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_meetings", new { limit = 30 }, ct);
        return ParseMeetings(r);
    }

    public async Task<List<MeetingItem>> PastMeetingsAsync(CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_meetings", new { past = true, limit = 20 }, ct);
        return ParseMeetings(r).OrderByDescending(m => m.StartAt).ToList();
    }

    public Task CreateMeetingAsync(IDictionary<string, object?> args, CancellationToken ct) =>
        Mcp.CallToolAsync("create_meeting", args, ct);

    public Task LeaveMeetingAsync(string meeting, CancellationToken ct) =>
        Mcp.CallToolAsync("leave_meeting", new { meeting }, ct);

    public Task CancelMeetingAsync(string meeting, CancellationToken ct) =>
        Mcp.CallToolAsync("cancel_meeting", new { meeting }, ct);

    // ───────────── 統計 ─────────────

    public async Task<StatsSummary> MyStatsAsync(string preset, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("get_stats", new { perspective = "assignee", preset, users = new[] { "me" } }, ct);
        var row = Items(r, "rows").FirstOrDefault();
        int Minutes(JsonElement e, string name) =>
            e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.Object
            && v.TryGetProperty("minutes", out var m) && m.TryGetInt32(out var mins) ? mins : 0;
        int Int(JsonElement e, string name) =>
            e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) && v.TryGetInt32(out var i) ? i : 0;
        var names = Items(r, "projects").Where(p => Str(p, "key") is not null).ToDictionary(p => Str(p, "key")!, p => Str(p, "name") ?? "");
        var byProject = Items(r, "cells")
            .GroupBy(c => Str(c, "project") ?? "")
            .Select(g => (names.GetValueOrDefault(g.Key, g.Key), g.Sum(c => Minutes(c, "duration"))))
            .OrderByDescending(x => x.Item2)
            .ToList();
        return new StatsSummary(Minutes(row, "actual"), Minutes(row, "expected"), Int(row, "activeDays"), Int(row, "workdays"),
            Int(row, "assignedCount"), Int(row, "doneCount"), Int(row, "overdueCount"), byProject);
    }

    // ───────────── 共用 ─────────────

    static PersonRef? Person(JsonElement e) =>
        e.ValueKind == JsonValueKind.Object && Str(e, "username") is { } u ? new PersonRef(u, Str(e, "displayName") ?? u) : null;

    static DateOnly? Day(JsonElement e, string name) =>
        Str(e, name) is { } s && DateOnly.TryParse(s.Length > 10 ? s[..10] : s, out var d) ? d : null;

    static string? Message(JsonElement r) =>
        r.ValueKind == JsonValueKind.String ? r.GetString() : FirstStr(r, "message", "note", "changeRequest");
}
