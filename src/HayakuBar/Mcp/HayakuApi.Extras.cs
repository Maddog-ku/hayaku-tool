using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.Mcp;

public sealed record MeetingItem(
    string Key,
    string Id,
    string Title,
    string Project,
    DateTimeOffset StartAt,
    DateTimeOffset EndAt,
    string? Location,
    string? MyStatus)
{
    public bool InMeeting => MyStatus == "會議中";
}

public sealed record TodoItem(string Id, string Text, string? TaskCode, DateTimeOffset? RemindAt, string? Source, string? AssignedBy);

public sealed record ReminderItem(string Id, string Text, DateTimeOffset? At, string? Task);

public sealed record NotificationItem(
    string Id,
    string Type,
    string Label,
    string Text,
    string? Actor,
    string? TaskKey,
    string? ProjectKey,
    string? MeetingId,
    DateTimeOffset CreatedAt,
    bool Unread,
    int Count);

/// <summary>會議、待辦、提醒、隨手記、通知、工時修改。</summary>
public sealed partial class HayakuApi
{
    // ───────────── 會議 ─────────────

    public async Task<List<MeetingItem>> ListMeetingsAsync(DateOnly from, DateOnly to, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_meetings", new { from = Iso(from), to = Iso(to) }, ct);
        return ParseMeetings(r);
    }

    public Task JoinMeetingAsync(string meeting, CancellationToken ct) =>
        Mcp.CallToolAsync("join_meeting", new { meeting }, ct);

    internal static List<MeetingItem> ParseMeetings(JsonElement r)
    {
        var list = new List<MeetingItem>();
        if (!r.TryGetProperty("meetings", out var arr) || arr.ValueKind != JsonValueKind.Array)
            return list;
        foreach (var m in arr.EnumerateArray())
        {
            if (Date(m, "startAt") is not { } start || Date(m, "endAt") is not { } end)
                continue;
            list.Add(new MeetingItem(
                Str(m, "meeting") ?? Str(m, "id") ?? "",
                Str(m, "id") ?? "",
                Str(m, "title") ?? "",
                Str(m, "project") ?? "",
                start,
                end,
                Str(m, "location"),
                Str(m, "me")));
        }
        return list.OrderBy(m => m.StartAt).ToList();
    }

    // ───────────── 待辦、提醒、隨手記 ─────────────

    bool _loggedTodoSample;
    bool _loggedReminderSample;

    public async Task<List<TodoItem>> ListTodosAsync(CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_my_todos", null, ct);
        var list = new List<TodoItem>();
        foreach (var t in Items(r, "items", "todos"))
        {
            if (!_loggedTodoSample)
            {
                _loggedTodoSample = true;
                Log.Info($"待辦格式樣本：{Shorten(t.GetRawText())}");
            }
            var id = Str(t, "id");
            var text = FirstStr(t, "text", "title", "content", "body");
            if (id is null || text is null)
                continue;
            list.Add(new TodoItem(id, text.Trim(), FirstKey(t, "taskCode", "task"), FirstDate(t, "remindAt", "reminderAt", "dueAt"),
                FirstStr(t, "source", "page", "doc"), FirstName(t, "assignedBy")));
        }
        return list;
    }

    public Task CompleteTodoAsync(string todoId, CancellationToken ct) =>
        Mcp.CallToolAsync("complete_my_todo", new { todoId }, ct);

    public async Task<List<ReminderItem>> ListRemindersAsync(CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_reminders", new { tab = "upcoming" }, ct);
        var list = new List<ReminderItem>();
        foreach (var t in Items(r, "reminders", "items"))
        {
            if (!_loggedReminderSample)
            {
                _loggedReminderSample = true;
                Log.Info($"提醒格式樣本：{Shorten(t.GetRawText())}");
            }
            if (Str(t, "id") is not { } id)
                continue;
            list.Add(new ReminderItem(id, FirstStr(t, "text", "title", "message", "note", "target") ?? "提醒",
                FirstDate(t, "nextAt", "remindAt", "at", "fireAt", "time", "dueAt"), FirstKey(t, "task", "taskCode")));
        }
        return list.OrderBy(x => x.At ?? DateTimeOffset.MaxValue).ToList();
    }

    public Task SnoozeReminderAsync(string id, int minutes, CancellationToken ct) =>
        Mcp.CallToolAsync("snooze_reminder", new { id, minutes }, ct);

    public Task QuickCaptureAsync(string text, string? taskCode, bool todo, CancellationToken ct) =>
        taskCode is null
            ? Mcp.CallToolAsync("quick_capture", new { text, todo }, ct)
            : Mcp.CallToolAsync("quick_capture", new { text, todo, taskCode }, ct);

    // ───────────── 通知 ─────────────

    static readonly Dictionary<string, string> NotificationLabels = new()
    {
        ["TASK_ASSIGNED"] = "被指派任務", ["MENTIONED"] = "被提及", ["COMMENT_ON_MY_TASK"] = "任務有新聊天",
        ["TASK_DUE_SOON"] = "即將到期", ["TASK_OVERDUE"] = "已逾期", ["STATUS_CHANGED"] = "狀態被變更",
        ["ADDED_TO_PROJECT"] = "被加入專案", ["TIMER_AUTO_STOPPED"] = "計時被自動停止", ["TIMER_RESUME_PROMPT"] = "要繼續計時嗎",
        ["TASK_REMINDER"] = "提醒", ["CHANGE_REQUESTED"] = "有新的變更申請", ["CHANGE_REVIEWED"] = "申請已審核",
        ["DEPENDENCY_UNBLOCKED"] = "可以開始了", ["DEPENDENCY_TRIGGER_BLOCKED"] = "自動變更被擋住", ["HOURS_UNFILLED"] = "補工時提醒",
        ["HOURS_UNFILLED_WEEKLY"] = "上週工時沒填滿彙總", ["MEETING_STARTING"] = "會議即將開始", ["MEETING_INVITED"] = "被邀請參加會議",
        ["MEETING_CHANGED"] = "會議有變更", ["TASK_MOVED"] = "任務移到別的專案", ["WATCHED_ASSIGNEE_CHANGED"] = "關注的任務換了執行者",
        ["WATCHED_DUE_CHANGED"] = "關注的任務改了預計結束", ["WATCHER_ADDED"] = "被加為關注者", ["WATCHED_TASK"] = "我關注的任務",
        ["AUTOMATION"] = "自動化通知", ["DAILY_DIGEST"] = "每日摘要", ["MILESTONE_DUE_SOON"] = "里程碑快到了",
        ["BUDGET_THRESHOLD"] = "工時預算快用完", ["THREAD_REPLY"] = "討論串回覆", ["CHANNEL_MESSAGE"] = "頻道訊息",
        ["CHAT_REACTION"] = "訊息的表情回應", ["POLL_CLOSING"] = "投票快截止", ["POLL_CLOSED"] = "我的投票已結束",
        ["FORUM_POST"] = "論壇新貼文", ["SPACE_PAGE_COMMENT"] = "空間頁面有新評論", ["SECURITY_ALERT"] = "帳號安全",
        ["DOC_MENTIONED"] = "在文件被提及", ["DOC_TODO_ASSIGNED"] = "被指派待辦", ["DOC_TODO_DONE"] = "我指派的待辦完成了",
    };

    public async Task<List<NotificationItem>> ListNotificationsAsync(CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_notifications", null, ct);
        return ParseNotifications(r);
    }

    public Task MarkNotificationsReadAsync(IReadOnlyCollection<string>? ids, CancellationToken ct) =>
        ids is { Count: > 0 }
            ? Mcp.CallToolAsync("mark_notifications_read", new { ids }, ct)
            : Mcp.CallToolAsync("mark_notifications_read", null, ct);

    internal static List<NotificationItem> ParseNotifications(JsonElement r)
    {
        var list = new List<NotificationItem>();
        foreach (var group in Items(r, "items"))
        {
            var n = group.TryGetProperty("latest", out var latest) && latest.ValueKind == JsonValueKind.Object ? latest : group;
            if (Str(n, "id") is not { } id || Str(n, "type") is not { } type)
                continue;
            var unreadCount = group.TryGetProperty("unreadCount", out var uc) && uc.TryGetInt32(out var u) ? u : 0;
            var count = group.TryGetProperty("count", out var c) && c.TryGetInt32(out var cc) ? cc : 1;
            var project = n.TryGetProperty("project", out var p) && p.ValueKind == JsonValueKind.Object ? p : default;
            var task = n.TryGetProperty("task", out var t) && t.ValueKind == JsonValueKind.Object ? t : default;
            var payload = n.TryGetProperty("payload", out var pl) && pl.ValueKind == JsonValueKind.Object ? pl : default;
            var projectKey = project.ValueKind == JsonValueKind.Object ? Str(project, "key") : null;
            string? taskKey = null, meetingId = null;
            if (task.ValueKind == JsonValueKind.Object)
            {
                if (projectKey is not null && task.TryGetProperty("number", out var num) && num.TryGetInt32(out var number))
                    taskKey = $"{projectKey}-{number}";
                if (Str(task, "kind") == "MEETING")
                    meetingId = Str(task, "id");
            }
            var text = payload.ValueKind == JsonValueKind.Object ? Str(payload, "task") : null;
            text ??= task.ValueKind == JsonValueKind.Object ? Str(task, "title") : null;
            if (type == "DAILY_DIGEST")
                text = "今天到期、逾期、等你審核的事情與會議";
            text ??= project.ValueKind == JsonValueKind.Object ? Str(project, "name") : null;
            var actor = n.TryGetProperty("actor", out var a) && a.ValueKind == JsonValueKind.Object ? Str(a, "displayName") : null;
            list.Add(new NotificationItem(
                id,
                type,
                NotificationLabels.GetValueOrDefault(type, type),
                text ?? "",
                actor,
                taskKey,
                projectKey,
                meetingId,
                Date(n, "createdAt") ?? DateTimeOffset.Now,
                unreadCount > 0 || (n.TryGetProperty("readAt", out var readAt) && readAt.ValueKind == JsonValueKind.Null),
                count));
        }
        return list;
    }

    // ───────────── 工時（補登、修改、刪除） ─────────────

    public async Task<List<TimeEntry>> ListDayEntriesAsync(DateOnly day, CancellationToken ct) =>
        await ListTimeEntriesAsync(day, day, ct);

    public Task AddTimeEntryAsync(string task, DateTimeOffset start, DateTimeOffset end, string? note, CancellationToken ct) =>
        string.IsNullOrWhiteSpace(note)
            ? Mcp.CallToolAsync("add_time_entry", new { task, start = IsoTime(start), end = IsoTime(end) }, ct)
            : Mcp.CallToolAsync("add_time_entry", new { task, start = IsoTime(start), end = IsoTime(end), note }, ct);

    public Task UpdateTimeEntryAsync(string id, DateTimeOffset? start, DateTimeOffset? end, string? note, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["id"] = id };
        if (start is { } s)
            args["start"] = IsoTime(s);
        if (end is { } e)
            args["end"] = IsoTime(e);
        if (note is not null)
            args["note"] = note.Length == 0 ? null : note;
        return Mcp.CallToolAsync("update_time_entry", args, ct);
    }

    public Task DeleteTimeEntryAsync(string id, CancellationToken ct) =>
        Mcp.CallToolAsync("delete_time_entry", new { id }, ct);

    static string IsoTime(DateTimeOffset t) => t.ToString("yyyy-MM-dd'T'HH:mm:sszzz");

    // ───────────── 寬鬆的欄位讀取（待辦、提醒的格式沒有固定樣本） ─────────────

    static IEnumerable<JsonElement> Items(JsonElement r, params string[] names)
    {
        foreach (var name in names)
            if (r.ValueKind == JsonValueKind.Object && r.TryGetProperty(name, out var arr) && arr.ValueKind == JsonValueKind.Array)
                return arr.EnumerateArray().ToList();
        return [];
    }

    static string? FirstStr(JsonElement e, params string[] names) =>
        names.Select(n => Str(e, n)).FirstOrDefault(v => !string.IsNullOrWhiteSpace(v));

    static DateTimeOffset? FirstDate(JsonElement e, params string[] names) =>
        names.Select(n => Date(e, n)).FirstOrDefault(v => v is not null);

    /// <summary>任務編號：字串（「F2P-97 標題」也行）或物件 { key / code }。</summary>
    static string? FirstKey(JsonElement e, params string[] names)
    {
        foreach (var name in names)
        {
            if (!e.TryGetProperty(name, out var v))
                continue;
            if (v.ValueKind == JsonValueKind.String && KeyFinder.Parse(v.GetString()) is { } key)
                return key;
            if (v.ValueKind == JsonValueKind.Object && (Str(v, "key") ?? Str(v, "code")) is { } objKey)
                return KeyFinder.Parse(objKey) ?? objKey;
        }
        return null;
    }

    static string? FirstName(JsonElement e, string name) =>
        e.TryGetProperty(name, out var v) ? v.ValueKind switch
        {
            JsonValueKind.String => v.GetString(),
            JsonValueKind.Object => Str(v, "displayName") ?? Str(v, "username"),
            _ => null,
        } : null;

    static string Shorten(string s) => s.Length > 600 ? s[..600] + "…" : s;
}
