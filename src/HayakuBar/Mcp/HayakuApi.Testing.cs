using System.Text.Json;

namespace HayakuBar.Mcp;

/// <summary>任務類型：一般、測試（QA 開的測試任務）、測試結果（測試任務底下的一個問題）。</summary>
public enum TaskKind
{
    Normal,
    Test,
    TestResult,
}

/// <summary>任務類型、測試結果狀態的文字與分組（跟網頁同一套名稱）。</summary>
public static class TaskKinds
{
    /// <summary>類型的值可能是代碼（TEST、TEST_RESULT）或中文（測試、測試結果）；認不得的當一般任務。</summary>
    public static TaskKind Parse(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
            return TaskKind.Normal;
        var v = value.Trim().ToUpperInvariant();
        if (v.Contains("RESULT") || v.Contains("結果") || v is "BUG" or "ISSUE")
            return TaskKind.TestResult;
        if (v.Contains("TEST") || v.Contains("測試") || v == "QA")
            return TaskKind.Test;
        return TaskKind.Normal;
    }

    /// <summary>update_task、create_task 的 type 參數。</summary>
    public static string Label(TaskKind kind) => kind switch
    {
        TaskKind.Test => "測試",
        TaskKind.TestResult => "測試結果",
        _ => "一般",
    };

    /// <summary>標題前的類型圖示；一般任務沒有。</summary>
    public static string Icon(TaskKind kind) => kind switch
    {
        TaskKind.Test => "🧪",
        TaskKind.TestResult => "🐞",
        _ => "",
    };

    public static readonly IReadOnlyList<string> Severities = ["致命", "嚴重", "一般", "輕微"];
    public static readonly IReadOnlyList<string> Categories = ["待判斷", "Bug", "需求變更"];

    /// <summary>測試結果的狀態（網頁的順序）；「不處理」是結案的一種。</summary>
    public static readonly IReadOnlyList<string> ResultStatuses = ["未開始", "測試中", "待修復", "修復中", "待復驗", "已完成", "不處理"];

    /// <summary>
    /// 專案自訂的測試子狀態名稱 → 所屬階段（例 待更新測試機 → 修復中、待更新正式機 → 待復驗）。
    /// 讀到專案設定或測試總覽時記下；只有狀態名稱、沒有 testStage 的回應（我的任務、任務列表）靠它分對組。
    /// </summary>
    static readonly System.Collections.Concurrent.ConcurrentDictionary<string, string> KnownStages = new();

    public static void LearnStages(IEnumerable<TestSubStatus> subStatuses)
    {
        foreach (var s in subStatuses)
            if (s.Name != s.Stage)
                KnownStages[s.Name] = s.Stage;
    }

    /// <summary>
    /// 同一段裡 status 的下一個子狀態：內建狀態（修復中、待復驗、已完成）固定是每段第一列，後面接專案自訂的子狀態
    /// （Hayaku 1.4.2 起，不用再自己建同名的狀態；回應有沒有列出內建那一列、或舊專案自己建了同名的都一樣）。
    /// status 不在這一段或已經是最後一個是 null。
    /// </summary>
    public static TestSubStatus? NextInStage(IEnumerable<TestSubStatus> subStatuses, string stage, string status)
    {
        var custom = subStatuses.Where(s => s.Stage == stage && s.Name != stage).ToList();
        var i = status == stage ? -1 : custom.FindIndex(s => s.Name == status);
        if (i < 0 && status != stage)
            return null;
        return i + 1 < custom.Count ? custom[i + 1] : null;
    }

    /// <summary>狀態所屬的測試階段（子狀態換成階段名稱）；stage 有給就用它。</summary>
    public static string StageOf(string status, string? stage = null) =>
        stage is { Length: > 0 } ? stage : KnownStages.TryGetValue(status, out var known) ? known : status;

    /// <summary>
    /// 測試結果狀態 → 分組：pass（已完成）、closed（不處理）、fixing（待修復、修復中）、retest（待復驗）、testing（未開始、測試中）。
    /// stage：回應的 testStage（子狀態所屬的階段，例 待上正式機 屬於待復驗）；沒有時查記下的子狀態，再認不得用狀態類別判斷。
    /// </summary>
    public static string Bucket(string status, string? category = null, string? stage = null) => StageOf(status, stage) switch
    {
        "已完成" or "通過" => "pass",
        "不處理" => "closed",
        "待修復" or "修復中" => "fixing",
        "待復驗" => "retest",
        "未開始" or "測試中" => "testing",
        _ => category switch
        {
            "DONE" => "pass",
            "CANCELLED" => "closed",
            "IN_PROGRESS" => "fixing",
            _ => "testing",
        },
    };

    /// <summary>嚴重度的顏色（brush 名稱）：致命、嚴重紅色，一般預設，輕微淡色。</summary>
    public static string SeverityBrush(string? severity) => severity switch
    {
        "致命" or "嚴重" => "ErrorBrush",
        "輕微" => "MutedBrush",
        _ => "TextBrush",
    };
}

/// <summary>
/// 專案的測試子狀態（get_project 的 testSubStatuses）：Stage 是所屬階段（修復中、待復驗、已完成），同一階段照順序；
/// BatchLabel 有值的可以批次推到下一步（畫面上是〔已{BatchLabel}〕，例 已更新到測試機）。
/// </summary>
public sealed record TestSubStatus(string Stage, string Name, string? BatchLabel, string? Color, int? Count = null, int? OldestDays = null)
{
    /// <summary>批次按鈕的文字，例「已更新到測試機」。</summary>
    public string? BatchText => BatchLabel is { Length: > 0 } b ? (b.StartsWith('已') ? b : "已" + b) : null;
}

/// <summary>測試相關欄位：類型、分類、嚴重度、修復輪數（退回一次加一輪）、回報的人、所屬階段（testStage，子狀態才跟狀態不同）。</summary>
public readonly record struct TestInfo(TaskKind Kind, string? Category, string? Severity, int Round, string? Reporter, string? Stage = null)
{
    /// <summary>第 3 輪起要醒目標示（網頁同一套）。</summary>
    public bool Hot => Round >= 3;
}

/// <summary>
/// 測試結果的一列（list_test_results）。Group：fix 視圖是 fix；test 視圖是 retest（待復驗）、toTest（要測的測試結果）、
/// tests（要測的測試任務）；overview 的逾時清單是 stale。
/// </summary>
public sealed record TestResultRow(
    string Key,
    string Title,
    string Project,
    string Status,
    TaskKind Kind,
    string? Category,
    string? Severity,
    int Round,
    IReadOnlyList<string> Assignees,
    string? Reporter,
    DateOnly? PlannedEnd,
    string? TestTask,
    string? TestTaskTitle,
    bool Stale,
    string Group,
    string? Stage = null)
{
    /// <summary>所屬階段（子狀態換成階段名稱）。</summary>
    public string StageName => TaskKinds.StageOf(Status, Stage);
}

/// <summary>測試總覽的一個測試任務：底下的測試結果各狀態數量。</summary>
public sealed record TestTaskProgress(string Key, string Title, string Project, int Total, int Passed, int Fixing, int Testing, int Retest);

/// <summary>測試總覽（list_test_results view=overview）。</summary>
public sealed record TestOverview(
    int Open,
    int Critical,
    int Major,
    int Retest,
    int? OldestRetestDays,
    int Created,
    int Done,
    int WontFix,
    double? AvgRound,
    int Round3Plus,
    int Stale,
    IReadOnlyList<(string Severity, int Count)> BySeverity,
    IReadOnlyList<TestTaskProgress> Tests,
    IReadOnlyList<TestResultRow> StaleItems,
    int PendingRelease = 0,
    IReadOnlyList<TestSubStatus>? SubStatuses = null);

/// <summary>read_file 的結果：圖片（Image 有值）或文字（Text，太長分段：NextOffset 不是 null 就還有）。</summary>
public sealed record FileContent(string Id, string? Name, string? MimeType, byte[]? Image, string? Text, int? NextOffset, string? Note);

/// <summary>問題回報（get_feedback）：全文、處理紀錄與截圖 id（用 read_file 看）。</summary>
public sealed record FeedbackDetail(int Number, string Title, string Status, string? Reporter, DateTimeOffset? CreatedAt, string Body,
    string? Resolution, IReadOnlyList<(string Label, string Value)> Environment, IReadOnlyList<string> History,
    IReadOnlyList<string> CreatedTasks, IReadOnlyList<string> ScreenshotIds);

/// <summary>測試流程（測試結果、復驗、三種視圖）、讀檔案內容、已完成的任務、關聯任務、問題回報。</summary>
public sealed partial class HayakuApi
{
    // ───────────── 測試結果 ─────────────

    /// <summary>
    /// 在測試任務底下加一筆測試結果（我是回報的人）。start：要修（待修復，預設）或待測（未開始）。回傳新測試結果的編號。
    /// </summary>
    public async Task<string?> AddTestResultAsync(string testTask, string title, string? content, string? category, string? severity,
        IReadOnlyList<string>? assignees, IReadOnlyList<string>? related, bool toFix, DateOnly? due, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["testTask"] = testTask, ["title"] = title, ["start"] = toFix ? "要修" : "待測" };
        if (!string.IsNullOrWhiteSpace(content))
            args["content"] = content;
        if (category is not null)
            args["category"] = category;
        if (severity is not null)
            args["severity"] = severity;
        if (assignees is { Count: > 0 })
            args["assignees"] = assignees;
        if (related is { Count: > 0 })
            args["related"] = related;
        if (due is { } d)
            args["plannedEnd"] = Iso(d);
        var r = await Mcp.CallToolAsync("add_test_result", args, ct);
        return CreatedKey(r, testTask);
    }

    /// <summary>復驗：pass＝通過（已完成），否則退回（待修復，輪數 +1）；comment 變成一則留言。只有回報的人或專案管理員能用。</summary>
    public async Task<string?> RetestAsync(string task, bool pass, string? comment, IReadOnlyList<string>? assignees, DateOnly? due,
        CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["task"] = task, ["result"] = pass ? "pass" : "reopen" };
        if (!pass && !string.IsNullOrWhiteSpace(comment))
            args["comment"] = comment;
        if (!pass && assignees is { Count: > 0 })
            args["assignees"] = assignees;
        if (!pass && due is { } d)
            args["plannedEnd"] = Iso(d);
        return Message(await Mcp.CallToolAsync("retest_test_result", args, ct));
    }

    /// <summary>
    /// 批次把測試結果推到下一個子狀態（例 已更新到測試機：待更新測試機 → 待復驗）。from：子狀態名稱或批次按鈕文字；
    /// tasks 為 null 是那個子狀態的全部；comment（更新說明，例版本號）變成每筆的留言。專案管理員、編輯者、測試員能用。
    /// </summary>
    public async Task<string?> AdvanceTestResultsAsync(string project, string from, IReadOnlyList<string>? tasks, string? comment,
        CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["project"] = project, ["from"] = from };
        if (tasks is { Count: > 0 })
            args["tasks"] = tasks;
        if (!string.IsNullOrWhiteSpace(comment))
            args["comment"] = comment.Trim();
        return Message(await Mcp.CallToolAsync("advance_test_results", args, ct));
    }

    /// <summary>testSubStatuses（get_project）或 subStatuses（測試總覽）；順便記下子狀態屬於哪個階段。</summary>
    internal static List<TestSubStatus> ParseSubStatuses(JsonElement r, params string[] names)
    {
        var list = Items(r, names)
            .Where(s => Str(s, "name") is not null && Str(s, "stage") is not null)
            .Select(s => new TestSubStatus(Str(s, "stage")!, Str(s, "name")!, Str(s, "batchLabel"), Str(s, "color"),
                Number(s, "count"), Number(s, "oldestDays")))
            .ToList();
        TaskKinds.LearnStages(list);
        return list;
    }

    /// <summary>需修復（fix）或需測試（test）的列表；project 為 null 是全部專案，mine 只看跟我有關的。</summary>
    public async Task<List<TestResultRow>> ListTestResultsAsync(string view, string? project, bool mine, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["view"] = view, ["mine"] = mine };
        if (project is not null)
            args["project"] = project;
        return ParseTestRows(await Mcp.CallToolAsync("list_test_results", args, ct));
    }

    /// <summary>測試總覽；period：week、month、30d、all。</summary>
    public async Task<TestOverview> TestOverviewAsync(string? project, string period, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["view"] = "overview", ["period"] = period };
        if (project is not null)
            args["project"] = project;
        return ParseTestOverview(await Mcp.CallToolAsync("list_test_results", args, ct));
    }

    /// <summary>fix 視圖的 results；test 視圖的 retest、toTest、tests 三組（照這個順序）。</summary>
    internal static List<TestResultRow> ParseTestRows(JsonElement r)
    {
        var list = new List<TestResultRow>();
        foreach (var group in new[] { "results", "retest", "toTest", "tests", "items" })
            list.AddRange(Items(r, group).Select(t => ParseTestRow(t, group == "results" || group == "items" ? "fix" : group)).OfType<TestResultRow>());
        return list.DistinctBy(t => (t.Key, t.Group)).ToList();
    }

    internal static TestResultRow? ParseTestRow(JsonElement t, string group)
    {
        if (t.ValueKind != JsonValueKind.Object || FirstKey(t, "task", "key", "number") is not { } key)
            return null;
        var info = ReadTestInfo(t);
        var kind = info.Kind != TaskKind.Normal ? info.Kind : group == "tests" ? TaskKind.Test : TaskKind.TestResult;
        string? testTask = null, testTitle = null;
        foreach (var name in new[] { "testTask", "parent", "test" })
        {
            if (!t.TryGetProperty(name, out var p))
                continue;
            if (p.ValueKind == JsonValueKind.String)
                testTask ??= Core.KeyFinder.Parse(p.GetString());
            else if (p.ValueKind == JsonValueKind.Object)
            {
                testTask ??= FirstKey(p, "task", "key", "number");
                testTitle ??= FirstStr(p, "title", "name");
            }
        }
        testTask ??= Items(t, "path").Select(x => Str(x, "task")).LastOrDefault(x => x is not null);
        testTitle ??= Items(t, "path").Select(x => Str(x, "title")).LastOrDefault(x => x is not null);
        return new TestResultRow(
            key,
            FirstStr(t, "title", "name") ?? "",
            ProjectName(t),
            FirstStr(t, "status") ?? "",
            kind,
            info.Category,
            info.Severity,
            info.Round,
            Names(t, "assignees"),
            info.Reporter,
            Day(t, "plannedEnd") ?? Day(t, "due") ?? Day(t, "dueDate"),
            testTask,
            testTitle,
            Bool(t, "stale") || Bool(t, "isStale") || Bool(t, "overdue"),
            group,
            info.Stage);
    }

    internal static TestOverview ParseTestOverview(JsonElement r)
    {
        var subStatuses = ParseSubStatuses(r, "subStatuses");
        var cards = Obj(r, "cards");
        var open = Obj(cards, "open");
        var retest = Obj(cards, "retest");
        var closed = Obj(cards, "closed");
        var avg = Obj(cards, "avgRound");
        var bySeverity = Obj(r, "bySeverity");
        var severities = bySeverity.ValueKind == JsonValueKind.Object
            ? bySeverity.EnumerateObject().Select(p => (p.Name, p.Value.ValueKind == JsonValueKind.Number && p.Value.TryGetInt32(out var n) ? n : 0)).ToList()
            : [];
        var tests = Items(r, "tests")
            .Where(t => FirstKey(t, "task", "key") is not null)
            .Select(t =>
            {
                // 實際回應：results { total, open, byStatus { 狀態: 筆數 } }，依狀態分組加總
                var byStatus = Obj(Obj(t, "results"), "byStatus");
                if (byStatus.ValueKind == JsonValueKind.Object)
                {
                    var buckets = byStatus.EnumerateObject()
                        .GroupBy(p => TaskKinds.Bucket(p.Name))
                        .ToDictionary(g => g.Key, g => g.Sum(p => p.Value.ValueKind == JsonValueKind.Number && p.Value.TryGetInt32(out var n) ? n : 0));
                    int B(string bucket) => buckets.GetValueOrDefault(bucket);
                    return new TestTaskProgress(FirstKey(t, "task", "key")!, FirstStr(t, "title", "name") ?? "", ProjectName(t),
                        Number(Obj(t, "results"), "total") ?? buckets.Values.Sum(), B("pass"), B("fixing"), B("testing"), B("retest"));
                }
                var counts = Obj(t, "counts").ValueKind == JsonValueKind.Object ? Obj(t, "counts") : t;
                int N(params string[] names) => Number(counts, names) ?? 0;
                return new TestTaskProgress(FirstKey(t, "task", "key")!, FirstStr(t, "title", "name") ?? "", ProjectName(t),
                    N("total", "count"), N("passed", "pass", "done"), N("fixing", "fix") + N("toFix"), N("testing", "test", "todo"),
                    N("retest", "toRetest"));
            })
            .ToList();
        var stale = Obj(r, "stale");
        var staleItems = Items(stale.ValueKind == JsonValueKind.Object ? stale : r, "items")
            .Select(t => ParseTestRow(t, "stale")).OfType<TestResultRow>().ToList();
        double? avgRound = avg.ValueKind == JsonValueKind.Object && avg.TryGetProperty("value", out var av)
                           && av.ValueKind == JsonValueKind.Number && av.TryGetDouble(out var a) ? a : null;
        return new TestOverview(
            Number(open, "total") ?? 0, Number(open, "critical") ?? 0, Number(open, "major") ?? 0,
            Number(retest, "total") ?? 0, Number(retest, "oldestDays"),
            Number(Obj(cards, "created"), "count") ?? 0,
            Number(closed, "done") ?? 0, Number(closed, "wontfix", "wontFix") ?? 0,
            avgRound, Number(avg, "round3Plus") ?? 0,
            Number(cards, "stale") ?? Number(stale, "total") ?? 0,
            severities, tests, staleItems,
            Number(closed, "pendingRelease") ?? 0, subStatuses);
    }

    /// <summary>
    /// 任務的測試欄位；可能直接放在這一層（type、category、severity、round），或包在 testResult／test 物件裡。
    /// 一般任務通常沒有這些欄位。
    /// </summary>
    internal static TestInfo ReadTestInfo(JsonElement t)
    {
        if (t.ValueKind != JsonValueKind.Object)
            return default;
        var nested = new[] { "testResult", "test", "bug" }
            .Select(n => t.TryGetProperty(n, out var o) && o.ValueKind == JsonValueKind.Object ? o : default)
            .FirstOrDefault(o => o.ValueKind == JsonValueKind.Object);
        string? Field(params string[] names)
        {
            foreach (var source in new[] { t, nested })
            {
                if (source.ValueKind != JsonValueKind.Object)
                    continue;
                foreach (var name in names)
                {
                    if (!source.TryGetProperty(name, out var v))
                        continue;
                    if (v.ValueKind == JsonValueKind.String && v.GetString() is { Length: > 0 } s)
                        return s;
                    if (v.ValueKind == JsonValueKind.Object && FirstStr(v, "name", "label", "value") is { } named)
                        return named;
                }
            }
            return null;
        }
        int? Int(params string[] names)
        {
            foreach (var source in new[] { t, nested })
                if (Number(source, names) is { } n)
                    return n;
            return null;
        }
        var kind = TaskKinds.Parse(Field("type", "taskType"));
        if (kind == TaskKind.Normal && nested.ValueKind == JsonValueKind.Object)
            kind = TaskKind.TestResult;
        var reporter = Field("reporter", "reportedBy", "createdBy");
        if (reporter is null)
            foreach (var name in new[] { "reporter", "reportedBy", "createdBy" })
                if (t.TryGetProperty(name, out var p) && Person(p) is { } person)
                {
                    reporter = person.DisplayName;
                    break;
                }
        return new TestInfo(kind, Field("category", "testCategory"), Field("severity"), Int("round", "fixRound", "rounds") ?? 0, reporter,
            Field("testStage", "stage"));
    }

    // ───────────── 讀檔案內容 ─────────────

    /// <summary>
    /// 讀附件、會議檔案、白板、空間檔案、問題回報截圖的內容：圖片回圖、文件回文字（一次最多 5 萬字，offset 往下讀）。
    /// Hayaku 限制每人 10 分鐘最多讀 30 次（跟 Claude 共用），呼叫端要快取。
    /// </summary>
    public async Task<FileContent> ReadFileAsync(string id, int offset, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["id"] = id };
        if (offset > 0)
            args["offset"] = offset;
        return ParseFileContent(await Mcp.CallToolRawAsync("read_file", args, ct), id);
    }

    internal static FileContent ParseFileContent(JsonElement result, string id)
    {
        byte[]? image = null;
        string? mime = null, text = null, name = null, note = null;
        int? next = null;
        void FromObject(JsonElement o)
        {
            if (o.ValueKind != JsonValueKind.Object)
                return;
            name ??= FirstStr(o, "name", "fileName", "filename", "title");
            mime ??= FirstStr(o, "mimeType", "contentType", "mime");
            text ??= FirstStr(o, "text", "content", "markdown");
            note ??= FirstStr(o, "note", "message", "warning");
            if (o.TryGetProperty("nextOffset", out var n) && n.ValueKind == JsonValueKind.Number && n.TryGetInt32(out var nv))
                next ??= nv;
        }
        if (result.TryGetProperty("structuredContent", out var structured))
            FromObject(structured);
        if (result.TryGetProperty("content", out var content) && content.ValueKind == JsonValueKind.Array)
        {
            var texts = new List<string>();
            foreach (var item in content.EnumerateArray())
            {
                switch (Str(item, "type"))
                {
                    case "image" when image is null && Str(item, "data") is { } data:
                        try
                        {
                            image = Convert.FromBase64String(data);
                            mime = Str(item, "mimeType") ?? mime;
                        }
                        catch (FormatException)
                        {
                        }
                        break;
                    case "text" when Str(item, "text") is { } t:
                        texts.Add(t);
                        break;
                }
            }
            // 文字可能是 JSON（{ name, text, nextOffset }），也可能就是內容
            foreach (var t in texts)
            {
                if (t.TrimStart().StartsWith('{'))
                {
                    try
                    {
                        using var doc = JsonDocument.Parse(t);
                        FromObject(doc.RootElement);
                        continue;
                    }
                    catch (JsonException)
                    {
                    }
                }
                if (image is not null)
                    note ??= t; // 圖片旁邊的說明
                else
                    text = text is null ? t : text + "\n" + t;
            }
        }
        return new FileContent(id, name, mime, image, image is null ? text ?? "" : null, next, note);
    }

    // ───────────── 已完成的任務 ─────────────

    /// <summary>我做完的任務，最近完成的在最上面；days：7、30、90 或 all。</summary>
    public async Task<List<TaskSummary>> MyDoneTasksAsync(string days, CancellationToken ct) =>
        ParseTaskSummaries(await Mcp.CallToolAsync("my_tasks", new { tab = "done", days }, ct));

    // ───────────── 關聯任務 ─────────────

    /// <summary>關聯任務（不擋開始、不影響日期，可以跨專案）。</summary>
    public async Task<string?> AddRelatedAsync(string task, string other, CancellationToken ct) =>
        Message(await Mcp.CallToolAsync("add_dependency", new { task, kind = "related", related = other }, ct));

    public async Task<string?> RemoveRelatedAsync(string task, string other, CancellationToken ct) =>
        Message(await Mcp.CallToolAsync("remove_dependency", new { kind = "related", task, related = other }, ct));

    // ───────────── 問題回報 ─────────────

    public async Task<FeedbackDetail> GetFeedbackAsync(int number, CancellationToken ct) =>
        ParseFeedback(await Mcp.CallToolAsync("get_feedback", new { number }, ct), number);

    internal static FeedbackDetail ParseFeedback(JsonElement r, int number)
    {
        var f = Obj(r, "feedback").ValueKind == JsonValueKind.Object ? Obj(r, "feedback") : r;
        var env = new List<(string, string)>();
        var envObj = Obj(f, "environment").ValueKind == JsonValueKind.Object ? Obj(f, "environment") : Obj(f, "env");
        if (envObj.ValueKind == JsonValueKind.Object)
        {
            var labels = new Dictionary<string, string>
            {
                ["url"] = "網址", ["version"] = "版本", ["browser"] = "瀏覽器", ["userAgent"] = "瀏覽器", ["viewport"] = "視窗",
                ["window"] = "視窗", ["theme"] = "色系", ["colorScheme"] = "色系", ["timezone"] = "時區",
            };
            foreach (var p in envObj.EnumerateObject())
                if (p.Value.ValueKind is JsonValueKind.String or JsonValueKind.Number && p.Value.ToString() is { Length: > 0 } v)
                    env.Add((labels.GetValueOrDefault(p.Name, p.Name), v));
        }
        var history = Items(f, "history", "events", "logs")
            .Select(h => h.ValueKind == JsonValueKind.String ? h.GetString()
                : string.Join("  ", new[]
                {
                    FirstDate(h, "at", "createdAt") is { } at ? at.ToLocalTime().ToString("MM/dd HH:mm") : null,
                    FirstName(h, "actor") ?? FirstName(h, "user"),
                    FirstStr(h, "text", "action", "note", "message", "status"),
                }.Where(x => !string.IsNullOrEmpty(x))))
            .Where(x => !string.IsNullOrWhiteSpace(x))
            .Select(x => x!)
            .ToList();
        var tasks = Items(f, "tasks", "createdTasks")
            .Select(t => t.ValueKind == JsonValueKind.String ? t.GetString() : FirstKey(t, "task", "key"))
            .OfType<string>().ToList();
        var shots = Items(f, "screenshots", "attachments", "images")
            .Select(s => s.ValueKind == JsonValueKind.String ? s.GetString() : FirstStr(s, "id", "attachmentId", "fileId"))
            .OfType<string>().ToList();
        return new FeedbackDetail(
            Number(f, "number") ?? number,
            FirstStr(f, "title", "summary") ?? $"#{number}",
            FirstStr(f, "status", "state") ?? "",
            FirstName(f, "reporter") ?? FirstName(f, "createdBy") ?? FirstName(f, "user"),
            FirstDate(f, "createdAt"),
            FirstStr(f, "body", "description", "content", "text") ?? "",
            FirstStr(f, "resolution", "handlingNote", "response", "reply"),
            env, history, tasks, shots);
    }

    // ───────────── 共用 ─────────────

    static JsonElement Obj(JsonElement e, string name) =>
        e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.Object ? v : default;

    /// <summary>專案：字串（名稱或代碼）或物件 { name / key }。</summary>
    static string ProjectName(JsonElement t) =>
        t.TryGetProperty("project", out var p)
            ? p.ValueKind == JsonValueKind.String ? p.GetString() ?? "" : FirstStr(p, "name", "key") ?? ""
            : "";

    /// <summary>人名清單：字串陣列或 { username, displayName } 陣列（取顯示名稱）。</summary>
    static List<string> Names(JsonElement t, string name) =>
        Items(t, name)
            .Select(x => x.ValueKind == JsonValueKind.String ? x.GetString() : Person(x)?.DisplayName ?? FirstStr(x, "name"))
            .OfType<string>()
            .ToList();
}
