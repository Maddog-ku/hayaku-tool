using System.Text.Json;

namespace HayakuBar.Mcp;

/// <summary>子任務範本（list_subtask_templates）：Project 為 null 是全站範本；Count 是範本裡的任務數（回應沒有就是 null）。</summary>
public sealed record SubtaskTemplate(string Id, string Name, string? Project, int? Count, string? Description)
{
    public string Label =>
        Name + (Count is { } n ? $"（{n} 個任務）" : "") + (Project is null ? " · 全站" : $" · {Project}");
}

/// <summary>套用範本的結果（apply_subtask_template）：Lines 是每個任務會建／建了什麼，Message 是伺服器的說明，Raw 是認不出格式時的原文。</summary>
public sealed record TemplateApplyResult(IReadOnlyList<string> Lines, string? Message, string Raw);

/// <summary>
/// Hayaku 1.5.0 的範本中心：把子任務範本套到任務上。
/// 帳號目前沒有範本（list_subtask_templates 回 {"templates":[]}），範本與套用結果的欄位照工具說明寬鬆讀。
/// </summary>
public sealed partial class HayakuApi
{
    /// <summary>看得到的子任務範本（全站與我是成員的專案）；給 project 時只列能套到那個專案的。</summary>
    public async Task<List<SubtaskTemplate>> ListSubtaskTemplatesAsync(string? project, CancellationToken ct) =>
        ParseSubtaskTemplates(await Mcp.CallToolAsync("list_subtask_templates", project is null ? null : new { project }, ct));

    /// <summary>
    /// 把範本套到任務上（最多 500 個）。dryRun 只回會建哪些、日期，不寫入；skipExisting 略過已經有子任務的任務
    /// （false 也套用，第一層同名的略過）。執行者一次只能套一個任務，建成待核准。
    /// </summary>
    public async Task<TemplateApplyResult> ApplySubtaskTemplateAsync(IReadOnlyList<string> tasks, string template, bool skipExisting, bool dryRun, CancellationToken ct) =>
        ParseTemplateApply(await Mcp.CallToolAsync("apply_subtask_template", new { tasks, template, skipExisting, dryRun }, ct));

    internal static List<SubtaskTemplate> ParseSubtaskTemplates(JsonElement r)
    {
        var list = new List<SubtaskTemplate>();
        foreach (var t in r.ValueKind == JsonValueKind.Array ? r.EnumerateArray().ToList() : Items(r, "templates", "items"))
        {
            var id = IdText(t, "id");
            var name = FirstStr(t, "name", "title") ?? id;
            if (name is null)
                continue;
            var project = t.TryGetProperty("project", out var p) ? p.ValueKind switch
            {
                JsonValueKind.String => p.GetString(),
                JsonValueKind.Object => FirstStr(p, "key", "code", "name"),
                _ => null,
            } : FirstStr(t, "projectKey");
            var count = Number(t, "count", "taskCount", "itemCount", "nodeCount")
                        ?? (Items(t, "items", "nodes", "tasks", "children").ToList() is { Count: > 0 } nodes ? CountNodes(nodes) : null);
            list.Add(new SubtaskTemplate(id ?? name, name, string.IsNullOrWhiteSpace(project) ? null : project, count, FirstStr(t, "description", "note")));
        }
        return list;
    }

    /// <summary>
    /// 套用結果 → 每個任務一行：「F2P-5：建立 6 個子任務（需求確認 11/12～11/13、…）」「F2P-6：略過（已經有子任務）」「F2P-7：失敗：…」。
    /// </summary>
    internal static TemplateApplyResult ParseTemplateApply(JsonElement r)
    {
        var raw = r.ValueKind == JsonValueKind.Undefined ? "" : r.ValueKind == JsonValueKind.String ? r.GetString() ?? "" : r.GetRawText();
        if (r.ValueKind != JsonValueKind.Object)
            return new TemplateApplyResult([], r.ValueKind == JsonValueKind.String ? r.GetString() : null, raw);
        var lines = new List<string>();
        foreach (var e in Items(r, "results", "tasks", "applied", "items", "preview"))
        {
            var key = FirstKey(e, "task", "key", "parent") ?? FirstStr(e, "task", "id") ?? "?";
            if (FirstStr(e, "error") is { } error)
            {
                lines.Add($"{key}：失敗：{error}");
                continue;
            }
            if (e.TryGetProperty("skipped", out var s) && (s.ValueKind == JsonValueKind.True || s.ValueKind == JsonValueKind.String))
            {
                var reason = s.ValueKind == JsonValueKind.String ? s.GetString() : FirstStr(e, "reason", "message");
                lines.Add($"{key}：略過" + (string.IsNullOrWhiteSpace(reason) ? "" : $"（{reason}）"));
                continue;
            }
            var nodes = Items(e, "created", "tasks", "children", "items", "nodes").ToList();
            var count = Number(e, "createdCount", "count") ?? (nodes.Count > 0 ? CountNodes(nodes) : null);
            var titles = nodes.Take(5).Select(n => FirstStr(n, "title", "name") is { } title ? title + Dates(n) : null).OfType<string>().ToList();
            var text = count is { } c ? $"{key}：{c} 個子任務" : $"{key}：" + (FirstStr(e, "message", "note", "status") ?? "完成");
            if (titles.Count > 0)
                text += $"（{string.Join("、", titles)}{(nodes.Count > titles.Count ? "、…" : "")}）";
            lines.Add(text);
        }
        return new TemplateApplyResult(lines, FirstStr(r, "message", "note", "summary"), raw);
    }

    /// <summary>樹狀節點總數（含 children 底下的）。</summary>
    static int CountNodes(IEnumerable<JsonElement> nodes) =>
        nodes.Sum(n => 1 + CountNodes(Items(n, "children", "items", "nodes")));

    static string Dates(JsonElement n)
    {
        var start = Day(n, "plannedStart");
        var end = Day(n, "plannedEnd");
        return (start, end) switch
        {
            ({ } a, { } b) when a != b => $" {a:M/d}～{b:M/d}",
            (_, { } b) => $" {b:M/d}",
            ({ } a, null) => $" {a:M/d}",
            _ => "",
        };
    }

    /// <summary>id 可能是字串或數字。</summary>
    static string? IdText(JsonElement e, string name) =>
        e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) ? v.ValueKind switch
        {
            JsonValueKind.String => v.GetString(),
            JsonValueKind.Number => v.GetRawText(),
            _ => null,
        } : null;
}
