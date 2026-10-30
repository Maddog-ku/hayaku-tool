using System.Globalization;
using System.Text.Json;
using HayakuBar.Core;

namespace HayakuBar.Mcp;

/// <summary>
/// 相依關係的一筆。OtherKey／OtherTitle 是另一端的任務；Rule 是規則的說明（例「完成後才能開始」）；
/// Blocking：這條相依目前擋著（前置還沒到條件）。
/// </summary>
public sealed record DependencyItem(string Id, string OtherKey, string OtherTitle, string? OtherStatus, string Rule, bool Blocking);

/// <summary>Predecessors：這個任務在等的；Successors：在等這個任務的。</summary>
public sealed record DependencyInfo(IReadOnlyList<DependencyItem> Predecessors, IReadOnlyList<DependencyItem> Successors);

public sealed record AttachmentItem(string Name, long? Size, string? Uploader, DateTimeOffset? At, string? Url);

/// <summary>任務異動紀錄的一則：Text 已經換成中文說明。</summary>
public sealed record TaskActivityItem(DateTimeOffset At, string Actor, string Text);

/// <summary>任務的文件（Markdown）與白板文字。</summary>
public sealed record ContentDoc(string Title, string Text);

/// <summary>任務詳細的附加資料：相依、附件、文件、異動紀錄、刪除。都是展開那一塊才讀。</summary>
public sealed partial class HayakuApi
{
    /// <summary>新增相依時可選的常用規則（template 代碼 → 說明）。</summary>
    public static readonly IReadOnlyList<(string Code, string Label)> DependencyTemplates =
    [
        ("finishToStart", "完成後才能開始"),
        ("finishToFinish", "完成後才能完成"),
        ("finishThenTodo", "完成後自動變成待處理"),
        ("finishThenStart", "完成後自動開始"),
    ];

    public async Task<DependencyInfo> ListDependenciesAsync(string task, CancellationToken ct) =>
        ParseDependencies(await Mcp.CallToolAsync("list_dependencies", new { task }, ct));

    /// <summary>waitsFor：這個任務等 other（other 是前置）；否則 other 等這個任務。回傳伺服器的說明（執行者會變成變更申請）。</summary>
    public async Task<string?> AddDependencyAsync(string task, string other, bool waitsFor, string template, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["task"] = task, [waitsFor ? "waitsFor" : "blocks"] = other, ["template"] = template };
        return Message(await Mcp.CallToolAsync("add_dependency", args, ct));
    }

    public async Task<string?> RemoveDependencyAsync(string id, CancellationToken ct) =>
        Message(await Mcp.CallToolAsync("remove_dependency", new { id }, ct));

    /// <summary>附件清單；WebUrl 是網頁上這個任務的附件區（下載、上傳要到網頁）。</summary>
    public async Task<(List<AttachmentItem> Items, string? WebUrl)> ListAttachmentsAsync(string task, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("list_attachments", new { task }, ct);
        return (ParseAttachments(r), Str(r, "webUrl"));
    }

    public async Task<List<ContentDoc>> GetTaskContentAsync(string task, CancellationToken ct) =>
        ParseContent(await Mcp.CallToolAsync("get_content", new { task }, ct));

    /// <summary>異動紀錄與留言（新到舊，一頁 50 筆）；回傳下一頁的 cursor。</summary>
    public async Task<(List<TaskActivityItem> Items, string? Cursor)> TaskActivityAsync(string task, string? cursor, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["task"] = task, ["filter"] = "all" };
        if (cursor is not null)
            args["cursor"] = cursor;
        var r = await Mcp.CallToolAsync("get_task_activity", args, ct);
        return (ParseTaskActivity(r), Str(r, "nextCursor") ?? Str(r, "cursor"));
    }

    /// <summary>軟刪除（含子任務），可以從網頁垃圾桶還原；執行者會變成刪除申請（reason 給審核者看）。</summary>
    public async Task<string?> DeleteTaskAsync(string task, string? reason, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["task"] = task, ["confirm"] = true };
        if (!string.IsNullOrWhiteSpace(reason))
            args["reason"] = reason;
        return Message(await Mcp.CallToolAsync("delete_task", args, ct));
    }

    // ───────────── 解析 ─────────────

    internal static DependencyInfo ParseDependencies(JsonElement r) =>
        new(Items(r, "predecessors").Select(d => ParseDependency(d, "predecessor")).OfType<DependencyItem>().ToList(),
            Items(r, "successors").Select(d => ParseDependency(d, "successor")).OfType<DependencyItem>().ToList());

    /// <summary>另一端的任務可能放在 task、predecessor／successor、other 底下，或直接在這一層。</summary>
    static DependencyItem? ParseDependency(JsonElement d, string side)
    {
        if (d.ValueKind != JsonValueKind.Object)
            return null;
        var other = new[] { side, "task", "other" }
            .Select(name => d.TryGetProperty(name, out var o) && o.ValueKind == JsonValueKind.Object ? o : default)
            .FirstOrDefault(o => o.ValueKind == JsonValueKind.Object);
        var source = other.ValueKind == JsonValueKind.Object ? other : d;
        var key = Str(source, "task") ?? Str(source, "key") ?? Str(d, "taskKey") ?? Str(d, $"{side}Key");
        if (key is null)
            return null;
        var rule = Str(d, "templateName") ?? Str(d, "label") ?? Str(d, "description")
                   ?? DependencyTemplates.FirstOrDefault(t => t.Code == Str(d, "template")).Label
                   ?? (Str(d, "kind") switch { "GATE" => "限制", "TRIGGER" => "自動", _ => "" }
                       + (d.TryGetProperty("condition", out var c) && c.ValueKind == JsonValueKind.Array
                           ? $"（{string.Join("、", c.EnumerateArray().Select(x => x.ToString()))}）" : ""));
        var blocking = new[] { "blocking", "isBlocking", "blocked" }
            .Any(n => d.TryGetProperty(n, out var b) && b.ValueKind == JsonValueKind.True);
        return new DependencyItem(Str(d, "id") ?? "", key, Str(source, "title") ?? "", Str(source, "status"), rule, blocking);
    }

    internal static List<AttachmentItem> ParseAttachments(JsonElement r) =>
        Items(r, "attachments")
            .Where(a => a.ValueKind == JsonValueKind.Object)
            .Select(a => new AttachmentItem(
                Str(a, "name") ?? Str(a, "fileName") ?? Str(a, "filename") ?? "（沒有檔名）",
                a.TryGetProperty("size", out var s) && s.TryGetInt64(out var bytes) ? bytes
                : a.TryGetProperty("sizeBytes", out var sb) && sb.TryGetInt64(out var b2) ? b2 : null,
                (a.TryGetProperty("uploadedBy", out var u) || a.TryGetProperty("uploader", out u) || a.TryGetProperty("createdBy", out u))
                    ? u.ValueKind == JsonValueKind.String ? u.GetString() : Person(u)?.DisplayName
                    : null,
                Date(a, "createdAt") ?? Date(a, "uploadedAt"),
                Str(a, "webUrl") ?? Str(a, "url")))
            .ToList();

    internal static List<ContentDoc> ParseContent(JsonElement r)
    {
        var list = Items(r, "documents")
            .Select(d => new ContentDoc(Str(d, "title") ?? "文件", Str(d, "markdown") ?? ""))
            .ToList();
        foreach (var w in Items(r, "whiteboards"))
        {
            var texts = w.TryGetProperty("texts", out var t) && t.ValueKind == JsonValueKind.Array
                ? t.EnumerateArray().Select(x => x.ValueKind == JsonValueKind.String ? x.GetString() : Str(x, "text"))
                : [];
            list.Add(new ContentDoc($"白板：{Str(w, "title") ?? "白板"}", string.Join("\n", texts.Where(x => !string.IsNullOrWhiteSpace(x)))));
        }
        return list;
    }

    internal static List<TaskActivityItem> ParseTaskActivity(JsonElement r) =>
        Items(r, "items")
            .Where(i => i.ValueKind == JsonValueKind.Object)
            .Select(i =>
            {
                var actor = i.TryGetProperty("actor", out var a) ? Person(a)?.DisplayName : null;
                var source = Str(i, "source");
                var text = ActivityText(i);
                if (source == "MCP")
                    text += "（透過 MCP）";
                if (i.TryGetProperty("undone", out var u) && u.ValueKind == JsonValueKind.True)
                    text += "（已復原）";
                return new TaskActivityItem(Date(i, "createdAt") ?? DateTimeOffset.MinValue,
                    actor ?? (source == "SYSTEM" ? "系統" : ""), text);
            })
            .ToList();

    /// <summary>一則異動 → 中文說明；認不得的動作照原本的代碼顯示。</summary>
    static string ActivityText(JsonElement i)
    {
        if (Str(i, "type") is "comment" or "chat")
            return $"留言：{Str(i, "body") ?? Str(i, "text") ?? ""}";
        var changes = i.TryGetProperty("changes", out var c) && c.ValueKind == JsonValueKind.Object ? c : default;
        string? New(string field) =>
            changes.ValueKind == JsonValueKind.Object && changes.TryGetProperty(field, out var pair)
            && pair.ValueKind == JsonValueKind.Array && pair.GetArrayLength() == 2 ? Value(pair[1]) : null;
        var duration = changes.ValueKind == JsonValueKind.Object && changes.TryGetProperty("durationSeconds", out var ds)
                       && ds.ValueKind == JsonValueKind.Array && ds.GetArrayLength() == 2 && ds[1].TryGetDouble(out var secs)
            ? $"（{WorkMath.HoursMinutes(secs / 60)}）" : "";
        return Str(i, "action") switch
        {
            "task.create" => $"建立任務「{New("title")}」",
            "task.subtask.create" => $"新增子任務 {New("subtask")}",
            "task.update" => "修改：" + FieldChanges(changes),
            "task.delete" => "刪除任務",
            "task.restore" => "還原任務",
            "task.move" => $"移動到 {New("parent") ?? "最上層"}",
            "time_entry.start" => "開始計時",
            "time_entry.pause" => "暫停計時" + duration,
            "time_entry.auto_pause" => (New("stopReason") switch { "BREAK" => "休息時間自動暫停", "END" or "OFF_DUTY" => "下班自動暫停", _ => "自動暫停計時" }) + duration,
            "time_entry.discard" => "捨棄太短的計時" + duration,
            "time_entry.update" => "修改計時段",
            "time_entry.create" or "time_entry.add" => "補登工時" + duration,
            "time_entry.delete" => "刪除計時段",
            "time_entry.move" => "計時段移到別的任務",
            "change_request.create" => "送出變更申請：" + FieldChanges(changes, skip: "type"),
            "change_request.update" => "修改變更申請：" + FieldChanges(changes, skip: "type"),
            "change_request.approve" => "核准變更申請",
            "change_request.reject" => "拒絕變更申請",
            "change_request.withdraw" => "撤回變更申請",
            { } a when a.StartsWith("dependency.", StringComparison.Ordinal) => a.EndsWith("delete") || a.EndsWith("remove") ? "移除相依" : "設定相依",
            { } a when a.StartsWith("attachment.", StringComparison.Ordinal) => a.EndsWith("delete") ? "刪除附件" : $"上傳附件 {New("name") ?? ""}".TrimEnd(),
            { } a => a,
            null => "",
        };
    }

    /// <summary>{"title": ["舊", "新"], …} → 「名稱：舊 → 新、狀態：處理中」。</summary>
    static string FieldChanges(JsonElement changes, string? skip = null)
    {
        if (changes.ValueKind != JsonValueKind.Object)
            return "";
        var parts = new List<string>();
        foreach (var p in changes.EnumerateObject())
        {
            if (p.Name == skip || p.Value.ValueKind != JsonValueKind.Array || p.Value.GetArrayLength() != 2)
                continue;
            var label = ChangeLabels.GetValueOrDefault(p.Name, ActivityLabels.GetValueOrDefault(p.Name, p.Name));
            var (before, after) = (Value(p.Value[0]), Value(p.Value[1]));
            parts.Add(before is null ? $"{label}：{after ?? "清空"}" : $"{label}：{before} → {after ?? "清空"}");
        }
        return string.Join("、", parts);
    }

    static readonly Dictionary<string, string> ActivityLabels = new()
    {
        ["parent"] = "上層", ["plannedStartDate"] = "預計開始", ["plannedEndDate"] = "預計結束",
        ["actualStartDate"] = "實際開始", ["actualEndDate"] = "實際結束", ["fields"] = "欄位",
    };

    /// <summary>異動的值：日期時間換成本地時間、陣列用頓號接起來、太長的截斷。</summary>
    static string? Value(JsonElement v) => v.ValueKind switch
    {
        JsonValueKind.Null or JsonValueKind.Undefined => null,
        JsonValueKind.True => "是",
        JsonValueKind.False => "否",
        JsonValueKind.Array => string.Join("、", v.EnumerateArray().Select(Value).Where(x => x is not null)),
        JsonValueKind.String when v.GetString() is { Length: > 10 } s && s[4] == '-' && s.Contains('T')
                                  && DateTimeOffset.TryParse(s, CultureInfo.InvariantCulture, DateTimeStyles.None, out var at)
            => at.ToLocalTime().ToString("MM/dd HH:mm", CultureInfo.InvariantCulture),
        JsonValueKind.String => v.GetString() is { Length: > 60 } s ? s[..60] + "…" : v.GetString(),
        _ => v.GetRawText(),
    };
}
