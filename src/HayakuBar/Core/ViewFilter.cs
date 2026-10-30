using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>
/// 在本機套用專案視圖的篩選（狀態、狀態類別、執行者、優先級），不用為了換視圖再打一次 list_tasks。
/// 有認不得的條件時回 null，改請伺服器用 view 篩。
/// </summary>
public static class ViewFilter
{
    static readonly string[] StatusKeys = ["statuses", "status"];
    static readonly string[] CategoryKeys = ["statusCategories", "statusCategory", "categories"];
    static readonly string[] AssigneeKeys = ["assignees", "assignee"];
    static readonly string[] PriorityKeys = ["priorities", "priority"];

    public static List<ProjectTask>? Apply(IEnumerable<ProjectTask> tasks, ProjectView view, string? me)
    {
        if (view.HasOtherConditions
            || view.Filter.Keys.Any(k => !StatusKeys.Concat(CategoryKeys).Concat(AssigneeKeys).Concat(PriorityKeys).Contains(k, StringComparer.OrdinalIgnoreCase)))
            return null;
        IReadOnlyList<string>? Values(string[] keys) => keys.Select(k => view.Filter.GetValueOrDefault(k)).FirstOrDefault(v => v is { Count: > 0 });
        var statuses = Values(StatusKeys);
        var categories = Values(CategoryKeys);
        var assignees = Values(AssigneeKeys)?.Select(a => a == "me" && me is not null ? me : a).ToList();
        var priorities = Values(PriorityKeys);
        return tasks.Where(t =>
                (statuses is null || statuses.Contains(t.Status))
                && (categories is null || categories.Contains(t.StatusCategory, StringComparer.OrdinalIgnoreCase))
                && (assignees is null || (t.Assignees ?? []).Any(a => assignees.Contains(a, StringComparer.OrdinalIgnoreCase)))
                && (priorities is null || t.Priority is { } p && priorities.Contains(p)))
            .ToList();
    }
}
