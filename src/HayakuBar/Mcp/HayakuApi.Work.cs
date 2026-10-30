using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.Mcp;

/// <summary>我的任務列表的一筆（my_tasks）；Ancestors 是由上而下的上層任務。</summary>
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
    string Path,
    IReadOnlyList<(string Key, string Title)>? Ancestors = null,
    int? ChildCount = null)
{
    /// <summary>沒有子任務（真正要做的事）；舊版回應沒有子任務數時當作沒有。</summary>
    public bool IsLeaf => ChildCount is null or 0;
}

public sealed record PersonRef(string Username, string DisplayName);

/// <summary>專案裡的一個任務（list_tasks）；Parent 是直接上層的編號，最上層為 null。</summary>
public sealed record ProjectTask(
    string Key,
    string Title,
    string Status,
    string StatusCategory,
    string? Parent,
    DateOnly? PlannedStart,
    DateOnly? PlannedEnd,
    bool IsCategory = false);

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
    int PendingChangeRequests,
    double? LoggedMinutes = null,
    bool IsCategory = false,
    int ContentBlockCount = 0);

public sealed record StatusOption(string Name, string Category, bool IsDefault);

/// <summary>專案；Icon、IconColor 是 Hayaku 的圖示與顏色名稱（例 rocket、moss）。</summary>
public sealed record ProjectInfo(string Key, string Name, string Role, string? Icon = null, string? IconColor = null, bool IsArchived = false)
{
    /// <summary>專案管理員或編輯者：可以審核變更申請。</summary>
    public bool CanReview => Role is "ADMIN" or "MANAGER" or "OWNER" or "EDITOR";

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

    /// <summary>
    /// 指派給我、還沒完成的全部任務（不給分頁，順序跟網頁一樣）；一頁放不下時自動翻頁，最多 maxPages 頁。
    /// 四個分頁用 MyTaskTabs 從這份算，不用一個分頁打一次。
    /// </summary>
    public async Task<List<TaskSummary>> MyOpenTasksAsync(CancellationToken ct, int maxPages = 10)
    {
        var list = new List<TaskSummary>();
        string? cursor = null;
        for (var page = 0; page < maxPages; page++)
        {
            var r = await Mcp.CallToolAsync("my_tasks", cursor is null ? null : new { cursor }, ct);
            list.AddRange(ParseTaskSummaries(r));
            cursor = Str(r, "cursor") ?? Str(r, "nextCursor");
            if (string.IsNullOrEmpty(cursor))
                break;
        }
        return list.DistinctBy(t => t.Key, StringComparer.OrdinalIgnoreCase).ToList();
    }

    /// <summary>指派給我、某個狀態類別的任務（例 DONE）；最多讀 maxPages 頁。</summary>
    public async Task<List<TaskSummary>> MyTasksByCategoryAsync(string statusCategory, int maxPages, CancellationToken ct)
    {
        var list = new List<TaskSummary>();
        string? cursor = null;
        for (var page = 0; page < maxPages; page++)
        {
            var args = new Dictionary<string, object?> { ["statusCategory"] = statusCategory };
            if (cursor is not null)
                args["cursor"] = cursor;
            var r = await Mcp.CallToolAsync("my_tasks", args, ct);
            list.AddRange(ParseTaskSummaries(r));
            cursor = Str(r, "cursor") ?? Str(r, "nextCursor");
            if (string.IsNullOrEmpty(cursor))
                break;
        }
        return list;
    }

    internal static List<TaskSummary> ParseTaskSummaries(JsonElement r)
    {
        var list = new List<TaskSummary>();
        foreach (var t in Items(r, "tasks"))
        {
            if (Str(t, "task") is not { } key)
                continue;
            var ancestors = Items(t, "path")
                .Where(x => Str(x, "task") is not null)
                .Select(x => (Str(x, "task")!, Str(x, "title") ?? ""))
                .ToList();
            var path = string.Join(" › ", Items(t, "path").Select(x => Str(x, "title")).Where(x => x is not null));
            list.Add(new TaskSummary(key, Str(t, "title") ?? "", Str(t, "project") ?? "", Str(t, "status") ?? "",
                Str(t, "statusCategory") ?? "", Str(t, "priority") ?? "", Day(t, "plannedStart"), Day(t, "plannedEnd"),
                t.TryGetProperty("timing", out var timing) && timing.ValueKind == JsonValueKind.True, path, ancestors,
                t.TryGetProperty("childCount", out var cc) && cc.TryGetInt32(out var children) ? children : null));
        }
        return list;
    }

    /// <summary>專案的全部任務（含各層子任務），依 Hayaku 的順序；一頁 200 筆，自動翻頁。</summary>
    public async Task<List<ProjectTask>> ListProjectTasksAsync(string project, CancellationToken ct)
    {
        var list = new List<ProjectTask>();
        string? cursor = null;
        for (var page = 0; page < 25; page++)
        {
            var args = new Dictionary<string, object?> { ["project"] = project, ["limit"] = 200 };
            if (cursor is not null)
                args["cursor"] = cursor;
            var r = await Mcp.CallToolAsync("list_tasks", args, ct);
            list.AddRange(ParseProjectTasks(r));
            cursor = Str(r, "cursor");
            if (string.IsNullOrEmpty(cursor))
                break;
        }
        return list;
    }

    internal static List<ProjectTask> ParseProjectTasks(JsonElement r) =>
        Items(r, "tasks")
            .Where(t => Str(t, "task") is not null)
            .Select(t => new ProjectTask(
                Str(t, "task")!,
                Str(t, "title") ?? "",
                Str(t, "status") ?? "",
                Str(t, "statusCategory") ?? "",
                Items(t, "path").Select(p => Str(p, "task")).LastOrDefault(k => k is not null),
                Day(t, "plannedStart"),
                Day(t, "plannedEnd"),
                t.TryGetProperty("isCategory", out var cat) && cat.ValueKind == JsonValueKind.True))
            .ToList();

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
        double? loggedMinutes = lg.ValueKind == JsonValueKind.Object && lg.TryGetProperty("minutes", out var lm) && lm.TryGetDouble(out var lmv)
            ? lmv : null;
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
            Items(r, "pendingChangeRequests").Count(),
            loggedMinutes,
            r.TryGetProperty("isCategory", out var cat) && cat.ValueKind == JsonValueKind.True,
            Items(r, "contentBlocks").Count());
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
        var key = CreatedKey(r, fields.TryGetValue("parent", out var parent) ? parent as string : null);
        if (key is null)
        {
            var raw = r.ValueKind == JsonValueKind.Undefined ? "" : r.GetRawText();
            Log.Info($"create_task 的回應裡找不到任務編號：{(raw.Length > 500 ? raw[..500] + "…" : raw)}");
        }
        return key ?? "";
    }

    /// <summary>上層、路徑、子任務裡的編號不是新任務的，找的時候跳過。</summary>
    static readonly HashSet<string> NotTheNewTask = new(StringComparer.OrdinalIgnoreCase)
        { "parent", "path", "ancestors", "children", "project", "dependsOn", "blockedBy" };

    /// <summary>
    /// create_task 的回應 → 新任務的編號。直接建立是 { task: "F2P-103" }；執行者建立（要等核准）時可能包在物件裡，
    /// 所以由淺到深找 task／key／number／code 欄位（跳過上層、路徑）；都沒有才從訊息文字裡找（排除上層的編號）。
    /// </summary>
    internal static string? CreatedKey(JsonElement r, string? parent)
    {
        if (r.ValueKind == JsonValueKind.String)
            return KeyFinder.Parse(r.GetString()) is { } k && !string.Equals(k, parent, StringComparison.OrdinalIgnoreCase) ? k : null;
        if (r.ValueKind != JsonValueKind.Object)
            return null;
        var level = new List<JsonElement> { r };
        var texts = new List<string>();
        for (var depth = 0; depth < 4 && level.Count > 0; depth++)
        {
            var next = new List<JsonElement>();
            foreach (var obj in level)
            {
                foreach (var prop in obj.EnumerateObject())
                {
                    if (NotTheNewTask.Contains(prop.Name))
                        continue;
                    var v = prop.Value;
                    if (v.ValueKind == JsonValueKind.String)
                    {
                        if (prop.Name is "task" or "key" or "number" or "code" && KeyFinder.Parse(v.GetString()) is { } key)
                            return key;
                        texts.Add(v.GetString() ?? "");
                    }
                    else if (v.ValueKind == JsonValueKind.Object)
                        next.Add(v);
                    else if (v.ValueKind == JsonValueKind.Array)
                        next.AddRange(v.EnumerateArray().Where(x => x.ValueKind == JsonValueKind.Object));
                }
            }
            level = next;
        }
        return texts.SelectMany(KeyFinder.All)
            .FirstOrDefault(k => !string.Equals(k, parent, StringComparison.OrdinalIgnoreCase));
    }

    public async Task<List<SearchHit>> SearchTasksAsync(string query, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("search_tasks", new { query }, ct);
        return Items(r, "tasks")
            .Where(t => Str(t, "task") is not null)
            .Select(t => new SearchHit(Str(t, "task")!, Str(t, "title") ?? "", Str(t, "project") ?? ""))
            .ToList();
    }

    /// <summary>看得到的專案（含圖示與我的角色）；includeArchived 連封存的也列。</summary>
    public async Task<List<ProjectInfo>> ListProjectsAsync(CancellationToken ct, bool includeArchived = false)
    {
        var r = await Mcp.CallToolAsync("list_projects", includeArchived ? new { includeArchived = true } : null, ct);
        return ParseProjects(r);
    }

    internal static List<ProjectInfo> ParseProjects(JsonElement r) =>
        Items(r, "projects")
            .Where(p => Str(p, "key") is not null)
            .Select(p => new ProjectInfo(Str(p, "key")!, Str(p, "name") ?? "", Str(p, "role") ?? "", Str(p, "icon"), Str(p, "iconColor"),
                p.TryGetProperty("isArchived", out var a) && a.ValueKind == JsonValueKind.True))
            .ToList();

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

    /// <summary>user：帳號、顯示名稱或 "me"；null 列出看得到的所有人。</summary>
    public async Task<List<LeaveItem>> ListLeavesAsync(DateOnly from, DateOnly to, CancellationToken ct, string? user = null)
    {
        object args = user is null ? new { from = Iso(from), to = Iso(to) } : new { from = Iso(from), to = Iso(to), user };
        return ParseLeaves(await Mcp.CallToolAsync("list_leaves", args, ct));
    }

    internal static List<LeaveItem> ParseLeaves(JsonElement r)
    {
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

    /// <summary>登記請假；回傳新請假的 id（回應裡沒有就是 null）。</summary>
    public async Task<string?> AddLeaveAsync(IDictionary<string, object?> args, CancellationToken ct) =>
        CreatedId(await Mcp.CallToolAsync("add_leave", args, ct));

    /// <summary>新增類工具回傳的 id：{ id } 或 { leave: { id } }。</summary>
    internal static string? CreatedId(JsonElement r)
    {
        if (r.ValueKind != JsonValueKind.Object)
            return null;
        if (Str(r, "id") is { Length: > 0 } id)
            return id;
        return r.TryGetProperty("leave", out var leave) && leave.ValueKind == JsonValueKind.Object ? Str(leave, "id") : null;
    }

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
