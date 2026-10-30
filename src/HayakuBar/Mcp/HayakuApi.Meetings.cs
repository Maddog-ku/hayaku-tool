using System.Text.Json;

namespace HayakuBar.Mcp;

/// <summary>會議的參加者：狀態（會議中、已離開、還沒加入、請假）與這場會議記到的工時。</summary>
public sealed record MeetingAttendee(string Username, string Name, string State, int LoggedMinutes);

/// <summary>
/// 一場會議（get_meeting）。Note 是主要的會議紀錄（Markdown）；Recurrence 是重複規則的說明（不是重複會議為 null）；
/// CanEdit：主辦、專案管理員、系統管理員才能改時間、參加者（紀錄參加者也能改）。
/// </summary>
public sealed record MeetingDetail(
    string Key,
    string Id,
    string Title,
    string Project,
    DateTimeOffset StartAt,
    DateTimeOffset EndAt,
    string? Location,
    bool Recurring,
    string? Recurrence,
    string? Organizer,
    string? MyState,
    IReadOnlyList<MeetingAttendee> Attendees,
    string? Note,
    bool CanEdit);

/// <summary>會議紀錄的一份（一場會議可能有好幾份）。</summary>
public sealed record MeetingNote(string Title, DateTimeOffset? UpdatedAt, string Markdown);

/// <summary>會議的檔案：Source 是 ATTACHMENT（會議附件）、CHAT（留言的附件）或紀錄裡上傳的。</summary>
public sealed record MeetingFile(string Id, string FileName, long SizeBytes, string Source, string? UploadedBy, DateTimeOffset? UploadedAt)
{
    public string SourceText => Source switch
    {
        "ATTACHMENT" => "會議附件",
        "CHAT" => "留言",
        "NOTE" or "NOTES" => "會議紀錄",
        "" => "",
        _ => Source,
    };
}

/// <summary>白板：文字（由上到下、由左到右）、圖形數量、圖片數、快照數。MCP 不能改白板。</summary>
public sealed record MeetingWhiteboard(string Title, DateTimeOffset? UpdatedAt, IReadOnlyList<string> Text, int ElementCount, int ImageCount,
    int Snapshots);

/// <summary>會議詳細、修改、會議紀錄、檔案、白板。</summary>
public sealed partial class HayakuApi
{
    public async Task<MeetingDetail> GetMeetingAsync(string meeting, CancellationToken ct) =>
        ParseMeetingDetail(await Mcp.CallToolAsync("get_meeting", new { meeting }, ct))
        ?? throw new McpException($"讀不到會議 {meeting}");

    /// <summary>只給要改的欄位（title、startAt、endAt、location、note、addAttendees、removeAttendees、scope…），照 update_meeting 的參數。</summary>
    public Task UpdateMeetingAsync(string meeting, IDictionary<string, object?> fields, CancellationToken ct)
    {
        var args = new Dictionary<string, object?>(fields) { ["meeting"] = meeting };
        return Mcp.CallToolAsync("update_meeting", args, ct);
    }

    public async Task<List<MeetingNote>> MeetingNotesAsync(string meeting, CancellationToken ct) =>
        ParseMeetingNotes(await Mcp.CallToolAsync("get_meeting_notes", new { meeting }, ct));

    public async Task<List<MeetingFile>> MeetingFilesAsync(string meeting, CancellationToken ct) =>
        ParseMeetingFiles(await Mcp.CallToolAsync("list_meeting_files", new { meeting }, ct));

    public async Task<List<MeetingWhiteboard>> MeetingWhiteboardsAsync(string meeting, CancellationToken ct) =>
        ParseWhiteboards(await Mcp.CallToolAsync("get_meeting_whiteboards", new { meeting }, ct));

    internal static MeetingDetail? ParseMeetingDetail(JsonElement r)
    {
        if (r.ValueKind != JsonValueKind.Object || Date(r, "startAt") is not { } start || Date(r, "endAt") is not { } end)
            return null;
        var attendees = Items(r, "attendees")
            .Select(a => new MeetingAttendee(
                FirstStr(a, "user", "username") ?? "",
                FirstStr(a, "name", "displayName", "user") ?? "",
                FirstStr(a, "state", "status") ?? "",
                a.TryGetProperty("logged", out var l) && l.ValueKind == JsonValueKind.Object
                && l.TryGetProperty("minutes", out var m) && m.TryGetInt32(out var mins) ? mins : 0))
            .Where(a => a.Name.Length > 0)
            .ToList();
        var recurring = r.TryGetProperty("recurring", out var rec) && rec.ValueKind == JsonValueKind.True;
        var series = r.TryGetProperty("series", out var s) ? s : default;
        return new MeetingDetail(
            FirstStr(r, "meeting", "id") ?? "",
            Str(r, "id") ?? "",
            Str(r, "title") ?? "",
            Str(r, "project") ?? "",
            start,
            end,
            Str(r, "location"),
            recurring || series.ValueKind == JsonValueKind.Object,
            DescribeRecurrence(series) ?? (recurring ? "重複會議" : null),
            FirstStr(r, "organizer", "host"),
            Str(r, "me"),
            attendees,
            Str(r, "note"),
            r.TryGetProperty("canEdit", out var ce) && ce.ValueKind == JsonValueKind.True);
    }

    static readonly string[] WeekdayNames = ["日", "一", "二", "三", "四", "五", "六"];

    /// <summary>
    /// 重複規則 → 「每週一、三」「每 2 天（只算工作日）」；series 可能直接有說明文字，或有 rule（同 create_meeting 的 recurrence.rule）。
    /// 格式沒有固定樣本，看不懂回 null。
    /// </summary>
    internal static string? DescribeRecurrence(JsonElement series)
    {
        if (series.ValueKind != JsonValueKind.Object)
            return null;
        if (FirstStr(series, "text", "label", "summary", "description") is { } text)
            return text;
        var rule = series.TryGetProperty("rule", out var ru) && ru.ValueKind == JsonValueKind.Object ? ru : series;
        var freq = Str(rule, "freq");
        var interval = rule.TryGetProperty("interval", out var iv) && iv.TryGetInt32(out var n) && n > 1 ? n : 1;
        var every = interval > 1 ? $"每 {interval} " : "每";
        string? result = freq switch
        {
            "DAILY" => (interval > 1 ? $"每 {interval} 天" : "每天")
                       + (rule.TryGetProperty("workdaysOnly", out var w) && w.ValueKind == JsonValueKind.True ? "（只算工作日）" : ""),
            "WEEKLY" => (interval > 1 ? $"每 {interval} 週" : "每週")
                        + (rule.TryGetProperty("weekdays", out var wd) && wd.ValueKind == JsonValueKind.Array
                            ? string.Join("、", wd.EnumerateArray()
                                .Where(d => d.TryGetInt32(out var i) && i is >= 0 and <= 6)
                                .Select(d => WeekdayNames[d.GetInt32()]))
                            : ""),
            "MONTHLY" => $"{every}個月",
            "YEARLY" => $"{every}年",
            _ => null,
        };
        if (result is not null && Str(series, "until") is { Length: >= 10 } until)
            result += $"，到 {until[..10]}";
        return result;
    }

    internal static List<MeetingNote> ParseMeetingNotes(JsonElement r) =>
        Items(r, "notes")
            .Select(n => new MeetingNote(FirstStr(n, "title", "name") ?? "會議紀錄", FirstDate(n, "updatedAt"),
                FirstStr(n, "markdown", "content", "text") ?? ""))
            .ToList();

    internal static List<MeetingFile> ParseMeetingFiles(JsonElement r) =>
        Items(r, "files", "items")
            .Select(f => new MeetingFile(
                Str(f, "id") ?? "",
                FirstStr(f, "fileName", "name", "filename") ?? "（沒有檔名）",
                f.TryGetProperty("sizeBytes", out var sz) && sz.TryGetInt64(out var bytes) ? bytes : 0,
                Str(f, "source") ?? "",
                FirstStr(f, "uploadedBy", "uploader"),
                FirstDate(f, "uploadedAt", "createdAt")))
            .ToList();

    internal static List<MeetingWhiteboard> ParseWhiteboards(JsonElement r) =>
        Items(r, "whiteboards", "items")
            .Select(b => new MeetingWhiteboard(
                FirstStr(b, "title", "name") ?? "白板",
                FirstDate(b, "updatedAt"),
                b.TryGetProperty("text", out var t) && t.ValueKind == JsonValueKind.Array
                    ? t.EnumerateArray().Select(x => x.ValueKind == JsonValueKind.String ? x.GetString() : FirstStr(x, "text", "content"))
                        .Where(x => !string.IsNullOrWhiteSpace(x)).Select(x => x!).ToList()
                    : [],
                b.TryGetProperty("elementCount", out var ec) && ec.TryGetInt32(out var count) ? count : 0,
                Items(b, "images").Count(),
                b.TryGetProperty("snapshots", out var sn) && sn.TryGetInt32(out var snaps) ? snaps : 0))
            .ToList();
}
