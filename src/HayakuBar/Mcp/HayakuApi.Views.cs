using System.Text.Json;

namespace HayakuBar.Mcp;

/// <summary>
/// 專案的視圖（list_views）。Filter：認得的篩選條件（statuses、assignees…，值都轉成字串清單）；
/// HasOtherConditions：還有本機不會套用的條件，要請伺服器用 view 篩。
/// </summary>
public sealed record ProjectView(string Id, string Name, string Type, bool IsDefault,
    IReadOnlyDictionary<string, IReadOnlyList<string>> Filter, bool HasOtherConditions)
{
    public override string ToString() => Name;
}

/// <summary>批次修改的一筆結果；Error 為 null 是成功（Message 可能是「已送出變更申請」）。</summary>
public sealed record BatchResult(string Task, string? Error, string? Message);

public sealed partial class HayakuApi
{
    public async Task<List<ProjectView>> ListViewsAsync(string project, CancellationToken ct) =>
        ParseViews(await Mcp.CallToolAsync("list_views", new { project }, ct));

    internal static List<ProjectView> ParseViews(JsonElement r)
    {
        var list = new List<ProjectView>();
        foreach (var v in Items(r, "views"))
        {
            if (Str(v, "id") is not { } id)
                continue;
            var filter = new Dictionary<string, IReadOnlyList<string>>(StringComparer.OrdinalIgnoreCase);
            var other = false;
            if (v.TryGetProperty("filter", out var f) && f.ValueKind == JsonValueKind.Object)
                foreach (var p in f.EnumerateObject())
                {
                    switch (p.Value.ValueKind)
                    {
                        case JsonValueKind.Array when p.Value.EnumerateArray().All(x => x.ValueKind == JsonValueKind.String):
                            if (p.Value.GetArrayLength() > 0)
                                filter[p.Name] = p.Value.EnumerateArray().Select(x => x.GetString()!).ToList();
                            break;
                        case JsonValueKind.String when p.Value.GetString() is { Length: > 0 } s:
                            filter[p.Name] = [s];
                            break;
                        case JsonValueKind.Null or JsonValueKind.False:
                        case JsonValueKind.String:
                            break;
                        default:
                            other = true;
                            break;
                    }
                }
            list.Add(new ProjectView(id, Str(v, "name") ?? "視圖", Str(v, "type") ?? "",
                v.TryGetProperty("isDefault", out var d) && d.ValueKind == JsonValueKind.True, filter, other));
        }
        return list;
    }

    /// <summary>一次改多個任務（最多 100 筆，逐筆成功或失敗）；每筆是 { task, 欄位… }。</summary>
    public async Task<List<BatchResult>> UpdateTasksAsync(IReadOnlyList<Dictionary<string, object?>> tasks, CancellationToken ct) =>
        ParseBatchResults(await Mcp.CallToolAsync("update_tasks", new { tasks }, ct), tasks.Select(t => t["task"]?.ToString() ?? "").ToList());

    /// <summary>每筆結果在 results 或 items；沒有逐筆結果時當作全部成功。</summary>
    internal static List<BatchResult> ParseBatchResults(JsonElement r, IReadOnlyList<string> sent)
    {
        var items = Items(r, "results", "items", "tasks").Where(i => i.ValueKind == JsonValueKind.Object).ToList();
        if (items.Count == 0)
            return sent.Select(t => new BatchResult(t, null, null)).ToList();
        return items.Select((i, n) =>
        {
            var task = Str(i, "task") ?? Str(i, "key") ?? (n < sent.Count ? sent[n] : "");
            var error = Str(i, "error") ?? (i.TryGetProperty("ok", out var ok) && ok.ValueKind == JsonValueKind.False ? Str(i, "message") ?? "失敗" : null);
            return new BatchResult(task, error, error is null ? Str(i, "message") ?? Str(i, "note") : null);
        }).ToList();
    }
}
