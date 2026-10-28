using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>工作台「我的任務」清單的排序與狀態統計。</summary>
public static class TaskOrder
{
    public static readonly IReadOnlyList<(string Key, string Label)> Sorts =
    [
        ("default", "預設排序"),
        ("due", "預計結束：近→遠"),
        ("dueDesc", "預計結束：遠→近"),
        ("priority", "優先級：高→低"),
        ("status", "依狀態"),
        ("key", "任務編號"),
        ("timing", "計時中優先"),
    ];

    static readonly string[] PriorityRank = ["緊急", "高", "中", "低"];

    /// <summary>排序；同順位維持 Hayaku 給的原本順序。running：正在計時的任務（「計時中優先」用）。</summary>
    public static List<TaskSummary> Sort(IEnumerable<TaskSummary> tasks, string sort, IReadOnlySet<string> running)
    {
        var list = tasks.ToList();
        return sort switch
        {
            "due" => list.OrderBy(t => t.PlannedEnd is null).ThenBy(t => t.PlannedEnd).ToList(),
            "dueDesc" => list.OrderBy(t => t.PlannedEnd is null).ThenByDescending(t => t.PlannedEnd).ToList(),
            "priority" => list.OrderBy(t => PriorityOf(t.Priority)).ThenBy(t => t.PlannedEnd is null).ThenBy(t => t.PlannedEnd).ToList(),
            "status" => list.OrderBy(t => t.Status, StringComparer.CurrentCulture).ThenBy(t => t.PlannedEnd is null).ThenBy(t => t.PlannedEnd).ToList(),
            "key" => list.OrderBy(t => KeyFinder.ProjectOf(t.Key), StringComparer.OrdinalIgnoreCase).ThenBy(t => NumberOf(t.Key)).ToList(),
            "timing" => list.OrderByDescending(t => t.Timing || running.Contains(t.Key)).ToList(),
            _ => list,
        };
    }

    /// <summary>清單裡出現的狀態與各自的數量（多的在前）。</summary>
    public static List<(string Status, int Count)> Statuses(IEnumerable<TaskSummary> tasks) =>
        tasks.Where(t => t.Status.Length > 0)
            .GroupBy(t => t.Status)
            .Select(g => (g.Key, g.Count()))
            .OrderByDescending(x => x.Item2)
            .ThenBy(x => x.Key, StringComparer.CurrentCulture)
            .ToList();

    static int PriorityOf(string priority)
    {
        var i = Array.IndexOf(PriorityRank, priority);
        return i < 0 ? PriorityRank.Length : i;
    }

    static int NumberOf(string key) =>
        int.TryParse(key[(key.LastIndexOf('-') + 1)..], out var n) ? n : int.MaxValue;
}
