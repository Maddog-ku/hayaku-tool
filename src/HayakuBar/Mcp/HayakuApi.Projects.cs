using System.Text.Json;

namespace HayakuBar.Mcp;

/// <summary>釘在托盤上的任務（list_pins）：今天累積、是否正在計時、能不能計時。</summary>
public sealed record PinItem(string Key, string Title, string Project, string Status, int TodayMinutes, bool Running, bool CanTime);

/// <summary>
/// 變更申請（list_change_requests）。Type：CREATE、UPDATE、DELETE；Status：PENDING、APPROVED、REJECTED、WITHDRAWN。
/// Changes：改了哪些欄位（已換成中文名稱與值）。
/// </summary>
public sealed record ChangeRequest(
    string Id,
    string Project,
    string Type,
    string Status,
    string? TaskKey,
    string TaskTitle,
    PersonRef? Requester,
    PersonRef? Reviewer,
    DateTimeOffset CreatedAt,
    IReadOnlyList<string> Changes,
    string? Reason,
    string? ReviewNote,
    int LoggedMinutes,
    int DescendantCount)
{
    public string TypeText => Type switch
    {
        "CREATE" => "新增任務",
        "UPDATE" => "修改任務",
        "DELETE" => "刪除任務",
        _ => Type,
    };
}

/// <summary>專案動態的一則（list_project_activity）；At 是伺服器排好的顯示文字（例「10/30 13:41」）。</summary>
public sealed record ActivityItem(string At, string Project, string Text);

/// <summary>專案動態的摘要數字與最活躍的人。</summary>
public sealed record ActivityFeed(DateOnly From, DateOnly To, int Created, int Started, int Completed, int Overdue,
    IReadOnlyList<(string Name, int Count)> TopMembers, IReadOnlyList<ActivityItem> Items);

/// <summary>工作負荷的一天：分配到或實際計時的分鐘、當天的容量（應出勤）、做了幾個任務。</summary>
public sealed record WorkloadDay(DateOnly Date, int Minutes, int CapacityMinutes, int TaskCount);

public sealed record WorkloadTask(string Key, string Title, int Minutes);

/// <summary>某人在一段期間的工作負荷（get_workload）；Mode：planned 預計、actual 實際計時。</summary>
public sealed record Workload(string Mode, DateOnly From, DateOnly To, int TotalMinutes, int CapacityMinutes,
    IReadOnlyList<WorkloadDay> Days, IReadOnlyList<WorkloadTask> Tasks, IReadOnlyList<(string Key, string Title, string Reason)> Incomplete);

/// <summary>釘選托盤、切換計時、移動任務、變更申請、專案動態、工作負荷。</summary>
public sealed partial class HayakuApi
{
    // ───────────── 釘選托盤 ─────────────

    public async Task<List<PinItem>> ListPinsAsync(CancellationToken ct) =>
        ParsePins(await Mcp.CallToolAsync("list_pins", null, ct));

    /// <summary>釘到托盤（加在最後，最多 10 個）；回傳釘選後的整份清單。</summary>
    public async Task<List<PinItem>> PinTaskAsync(string task, CancellationToken ct) =>
        ParsePins(await Mcp.CallToolAsync("pin_task", new { task }, ct));

    public async Task<List<PinItem>> UnpinTaskAsync(string task, CancellationToken ct) =>
        ParsePins(await Mcp.CallToolAsync("unpin_task", new { task }, ct));

    internal static List<PinItem> ParsePins(JsonElement r) =>
        Items(r, "pins")
            .Where(p => Str(p, "task") is not null)
            .Select(p => new PinItem(
                Str(p, "task")!,
                Str(p, "title") ?? "",
                Str(p, "project") ?? "",
                Str(p, "status") ?? "",
                MinutesOf(p, "today"),
                Bool(p, "running"),
                !(p.TryGetProperty("canTime", out var c) && c.ValueKind == JsonValueKind.False)))
            .ToList();

    /// <summary>切換計時（同網頁托盤的 ▶）：同一個交易裡暫停我所有正在計時的，開始這個；keepOthers 是同時計時。</summary>
    public Task SwitchTimerAsync(string task, bool keepOthers, CancellationToken ct) =>
        keepOthers
            ? Mcp.CallToolAsync("switch_timer", new { task, keepOthers = true }, ct)
            : Mcp.CallToolAsync("switch_timer", new { task }, ct);

    // ───────────── 移動任務 ─────────────

    /// <summary>換上層（parent 為 null＝移到第一層）；只能在同一個專案裡。</summary>
    public Task MoveTaskAsync(string task, string? parent, CancellationToken ct) =>
        Mcp.CallToolAsync("move_task", new Dictionary<string, object?> { ["task"] = task, ["parent"] = parent }, ct);

    /// <summary>某個任務的全部計時段（每個人的都有，User 是顯示名稱）。</summary>
    public async Task<List<TimeEntry>> ListTaskEntriesAsync(string task, CancellationToken ct) =>
        ParseEntries(await Mcp.CallToolAsync("list_time_entries", new { task }, ct), "entries");

    /// <summary>補登一段工時，回傳新計時段的 id（回應裡沒有就是 null）。</summary>
    public async Task<string?> AddTimeEntryWithIdAsync(string task, DateTimeOffset start, DateTimeOffset end, string? note, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["task"] = task, ["start"] = IsoTime(start), ["end"] = IsoTime(end) };
        if (!string.IsNullOrWhiteSpace(note))
            args["note"] = note;
        var r = await Mcp.CallToolAsync("add_time_entry", args, ct);
        if (r.ValueKind != JsonValueKind.Object)
            return null;
        return Str(r, "id") ?? (r.TryGetProperty("entry", out var e) && e.ValueKind == JsonValueKind.Object ? Str(e, "id") : null);
    }

    // ───────────── 變更申請 ─────────────

    /// <summary>status：PENDING（預設）、APPROVED、REJECTED、WITHDRAWN；不給專案＝看得到的全部專案。</summary>
    public async Task<List<ChangeRequest>> ListChangeRequestsAsync(string status, CancellationToken ct) =>
        ParseChangeRequests(await Mcp.CallToolAsync("list_change_requests", new { status }, ct));

    internal static List<ChangeRequest> ParseChangeRequests(JsonElement r)
    {
        var list = new List<ChangeRequest>();
        foreach (var c in Items(r, "items"))
        {
            if (Str(c, "id") is not { } id)
                continue;
            var task = c.TryGetProperty("task", out var t) && t.ValueKind == JsonValueKind.Object ? t : default;
            var hasTask = task.ValueKind == JsonValueKind.Object;
            var logged = c.TryGetProperty("loggedSeconds", out var ls) && ls.TryGetInt32(out var secs) ? secs / 60 : 0;
            var descendants = c.TryGetProperty("descendantCount", out var dc) && dc.TryGetInt32(out var d) ? d : 0;
            list.Add(new ChangeRequest(
                id,
                Str(c, "project") ?? "",
                Str(c, "type") ?? "",
                Str(c, "status") ?? "",
                hasTask ? Str(task, "key") : null,
                hasTask ? Str(task, "title") ?? "" : "",
                c.TryGetProperty("requester", out var rq) ? Person(rq) : null,
                c.TryGetProperty("reviewer", out var rv) ? Person(rv) : null,
                Date(c, "createdAt") ?? DateTimeOffset.Now,
                ChangeTexts(c),
                Str(c, "reason"),
                Str(c, "reviewNote"),
                logged,
                descendants));
        }
        return list;
    }

    static readonly Dictionary<string, string> ChangeLabels = new()
    {
        ["title"] = "名稱", ["statusId"] = "狀態", ["status"] = "狀態", ["priority"] = "優先級",
        ["plannedStart"] = "預計開始", ["plannedEnd"] = "預計結束", ["actualStart"] = "實際開始", ["actualEnd"] = "實際結束",
        ["estimatedHours"] = "預計工時", ["estimatedMinutes"] = "預計工時", ["estimate"] = "預計工時",
        ["assigneeIds"] = "執行者", ["assignees"] = "執行者", ["note"] = "備註", ["parentId"] = "上層",
        ["milestoneIds"] = "里程碑", ["customFields"] = "自訂欄位", ["isCategory"] = "當作分類",
    };

    /// <summary>changes 物件 → 「預計結束：2026-11-05」這類文字；值是 id 的只寫「有變更」。</summary>
    internal static List<string> ChangeTexts(JsonElement c)
    {
        if (!c.TryGetProperty("changes", out var changes) || changes.ValueKind != JsonValueKind.Object)
            return [];
        var list = new List<string>();
        foreach (var p in changes.EnumerateObject())
        {
            var label = ChangeLabels.GetValueOrDefault(p.Name, p.Name);
            var value = p.Value.ValueKind switch
            {
                JsonValueKind.Null => "清空",
                JsonValueKind.True => "是",
                JsonValueKind.False => "否",
                JsonValueKind.Number when p.Name == "estimatedMinutes" && p.Value.TryGetDouble(out var m) => $"{Math.Round(m / 60, 2)} 小時",
                JsonValueKind.Number => p.Value.GetRawText(),
                JsonValueKind.String when p.Name.EndsWith("Id", StringComparison.Ordinal) => "有變更",
                JsonValueKind.String => p.Value.GetString() is { Length: > 40 } s ? s[..40] + "…" : p.Value.GetString(),
                _ => "有變更",
            };
            list.Add($"{label}：{value}");
        }
        return list;
    }

    /// <summary>核准或拒絕；watch：核准並關注那個任務；note：給申請人的說明；rejectAction：拒絕新增申請時 delete（刪掉那個任務）。</summary>
    public Task ReviewChangeRequestAsync(string id, bool approve, bool watch, string? note, CancellationToken ct, string? rejectAction = null)
    {
        var args = new Dictionary<string, object?> { ["id"] = id, ["decision"] = approve ? "approve" : "reject" };
        if (!approve && rejectAction is not null)
            args["rejectAction"] = rejectAction;
        if (approve && watch)
            args["watch"] = true;
        if (!string.IsNullOrWhiteSpace(note))
            args["note"] = note;
        return Mcp.CallToolAsync("review_change_request", args, ct);
    }

    public Task WithdrawChangeRequestAsync(string id, CancellationToken ct) =>
        Mcp.CallToolAsync("withdraw_change_request", new { id }, ct);

    // ───────────── 專案動態、工作負荷 ─────────────

    /// <summary>types：progress、assign、talk、other；project 為 null 是看得到的全部專案。</summary>
    /// <summary>members：只看這些人（帳號、顯示名稱或 "me"）。</summary>
    public async Task<ActivityFeed> ProjectActivityAsync(string? project, DateOnly from, DateOnly to, IReadOnlyCollection<string> types,
        CancellationToken ct, IReadOnlyCollection<string>? members = null)
    {
        var args = new Dictionary<string, object?> { ["from"] = Iso(from), ["to"] = Iso(to), ["types"] = types };
        if (project is not null)
            args["project"] = project;
        if (members is { Count: > 0 })
            args["members"] = members;
        return ParseActivity(await Mcp.CallToolAsync("list_project_activity", args, ct), from, to);
    }

    internal static ActivityFeed ParseActivity(JsonElement r, DateOnly from, DateOnly to)
    {
        var summary = r.TryGetProperty("summary", out var s) && s.ValueKind == JsonValueKind.Object ? s : default;
        int Num(string name) => summary.ValueKind == JsonValueKind.Object && summary.TryGetProperty(name, out var v) && v.TryGetInt32(out var i) ? i : 0;
        var top = summary.ValueKind == JsonValueKind.Object
            ? Items(summary, "topMembers")
                .Where(m => Str(m, "name") is not null)
                .Select(m => (Str(m, "name")!, m.TryGetProperty("count", out var c) && c.TryGetInt32(out var n) ? n : 0))
                .ToList()
            : [];
        var items = Items(r, "items")
            .Where(i => Str(i, "text") is not null)
            .Select(i => new ActivityItem((Str(i, "at") ?? "").Replace(' ', ' '), Str(i, "project") ?? "", Str(i, "text")!))
            .ToList();
        return new ActivityFeed(Day(r, "from") ?? from, Day(r, "to") ?? to, Num("created"), Num("started"), Num("completed"), Num("overdue"),
            top, items);
    }

    /// <summary>我在一段期間的工作負荷；mode：planned（預計工時照容量分配）或 actual（實際計時）。</summary>
    public async Task<Workload> MyWorkloadAsync(DateOnly from, DateOnly to, string mode, CancellationToken ct) =>
        ParseWorkload(await Mcp.CallToolAsync("get_workload", new { from = Iso(from), to = Iso(to), mode }, ct), from, to, mode);

    internal static Workload ParseWorkload(JsonElement r, DateOnly from, DateOnly to, string mode)
    {
        var actual = (Str(r, "mode") ?? mode) == "actual";
        var field = actual ? "actual" : "scheduled";
        var person = Items(r, "people").FirstOrDefault();
        var days = person.ValueKind == JsonValueKind.Object
            ? Items(person, "days")
                .Where(d => Day(d, "date") is not null)
                .Select(d => new WorkloadDay(Day(d, "date")!.Value, MinutesOf(d, field, "minutes"), MinutesOf(d, "capacity"),
                    d.TryGetProperty("taskCount", out var tc) && tc.TryGetInt32(out var n) ? n : Items(d, "tasks").Count()))
                .ToList()
            : [];
        var tasks = person.ValueKind == JsonValueKind.Object
            ? Items(person, "tasks")
                .Where(t => Str(t, "task") is not null)
                .Select(t => new WorkloadTask(Str(t, "task")!, Str(t, "title") ?? "", MinutesOf(t, field, "minutes")))
                .ToList()
            : [];
        var incomplete = r.TryGetProperty("incomplete", out var inc) && inc.ValueKind == JsonValueKind.Object
            ? Items(inc, "tasks").Where(t => Str(t, "task") is not null)
                .Select(t => (Str(t, "task")!, Str(t, "title") ?? "", Str(t, "reason") ?? "")).ToList()
            : [];
        var total = person.ValueKind == JsonValueKind.Object ? MinutesOf(person, field) : MinutesOf(r, field);
        var capacity = person.ValueKind == JsonValueKind.Object ? MinutesOf(person, "capacity") : 0;
        return new Workload(actual ? "actual" : "planned", Day(r, "from") ?? from, Day(r, "to") ?? to, total, capacity, days, tasks, incomplete);
    }

    // ───────────── 共用 ─────────────

    /// <summary>{ minutes, display } 這類欄位的分鐘數；也接受直接是數字（備用的欄位名稱放後面）。</summary>
    static int MinutesOf(JsonElement e, params string[] names)
    {
        foreach (var name in names)
        {
            if (e.ValueKind != JsonValueKind.Object || !e.TryGetProperty(name, out var v))
                continue;
            if (v.ValueKind == JsonValueKind.Object && v.TryGetProperty("minutes", out var m) && m.TryGetDouble(out var mins))
                return (int)mins;
            if (v.ValueKind == JsonValueKind.Number && v.TryGetDouble(out var n))
                return (int)n;
        }
        return 0;
    }

    static bool Bool(JsonElement e, string name) =>
        e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.True;
}
