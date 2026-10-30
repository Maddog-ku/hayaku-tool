using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>
/// 狀態篩選：可以混著選狀態類別（進行中、完成…）與各專案自己的狀態（處理中、待客戶回應…）；
/// 符合任一個就顯示，都沒選＝全部。
/// </summary>
public sealed class StatusFilter
{
    public HashSet<string> Categories { get; } = new(StringComparer.OrdinalIgnoreCase);
    public HashSet<string> Statuses { get; } = [];

    public bool IsEmpty => Categories.Count == 0 && Statuses.Count == 0;
    public int Count => Categories.Count + Statuses.Count;

    public bool Matches(TaskSummary t) =>
        IsEmpty || Categories.Contains(t.StatusCategory) || Statuses.Contains(t.Status);

    /// <summary>選的項目在新清單裡都不見了就清掉（換分頁、任務改狀態）。</summary>
    public void KeepOnly(IEnumerable<string> categories, IEnumerable<string> statuses)
    {
        Categories.IntersectWith(categories);
        Statuses.IntersectWith(statuses);
    }

    public void Clear()
    {
        Categories.Clear();
        Statuses.Clear();
    }
}

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

    /// <summary>狀態類別與顯示名稱，依工作流程的順序。</summary>
    public static readonly IReadOnlyList<(string Category, string Label)> CategoryLabels =
    [
        ("TODO", "待處理"),
        ("IN_PROGRESS", "進行中"),
        ("DONE", "完成"),
        ("CANCELLED", "已取消"),
    ];

    static readonly string[] PriorityRank = ["緊急", "高", "中", "低"];

    /// <summary>
    /// 排序；同順位維持 Hayaku 給的原本順序。running：正在計時的任務（「計時中優先」用）。
    /// statusRank：狀態在專案設定裡的順序（「依狀態」用：同一類的不同狀態照專案的順序，不再照名稱穿插）。
    /// </summary>
    public static List<TaskSummary> Sort(IEnumerable<TaskSummary> tasks, string sort, IReadOnlySet<string> running,
        Func<TaskSummary, int>? statusRank = null)
    {
        var list = tasks.ToList();
        return sort switch
        {
            "due" => list.OrderBy(t => t.PlannedEnd is null).ThenBy(t => t.PlannedEnd).ToList(),
            "dueDesc" => list.OrderBy(t => t.PlannedEnd is null).ThenByDescending(t => t.PlannedEnd).ToList(),
            "priority" => list.OrderBy(t => PriorityOf(t.Priority)).ThenBy(t => t.PlannedEnd is null).ThenBy(t => t.PlannedEnd).ToList(),
            "status" => list.OrderBy(t => CategoryRank(t.StatusCategory))
                .ThenBy(t => statusRank?.Invoke(t) ?? 0)
                .ThenBy(t => t.Status, StringComparer.CurrentCulture)
                .ThenBy(t => t.PlannedEnd is null).ThenBy(t => t.PlannedEnd).ToList(),
            "key" => list.OrderBy(t => KeyFinder.ProjectOf(t.Key), StringComparer.OrdinalIgnoreCase).ThenBy(t => NumberOf(t.Key)).ToList(),
            "timing" => list.OrderByDescending(t => t.Timing || running.Contains(t.Key)).ToList(),
            _ => list,
        };
    }

    public static int CategoryRank(string category)
    {
        for (var i = 0; i < CategoryLabels.Count; i++)
            if (CategoryLabels[i].Category == category)
                return i;
        return CategoryLabels.Count;
    }

    /// <summary>清單裡出現的狀態與各自的數量（多的在前）。</summary>
    public static List<(string Status, int Count)> Statuses(IEnumerable<TaskSummary> tasks) =>
        tasks.Where(t => t.Status.Length > 0)
            .GroupBy(t => t.Status)
            .Select(g => (g.Key, g.Count()))
            .OrderByDescending(x => x.Item2)
            .ThenBy(x => x.Key, StringComparer.CurrentCulture)
            .ToList();

    /// <summary>清單裡出現的狀態類別與數量，依工作流程的順序。</summary>
    public static List<(string Category, string Label, int Count)> Categories(IEnumerable<TaskSummary> tasks)
    {
        var counts = tasks.Where(t => t.StatusCategory.Length > 0).GroupBy(t => t.StatusCategory).ToDictionary(g => g.Key, g => g.Count());
        return CategoryLabels.Where(c => counts.ContainsKey(c.Category)).Select(c => (c.Category, c.Label, counts[c.Category])).ToList();
    }

    static int PriorityOf(string priority)
    {
        var i = Array.IndexOf(PriorityRank, priority);
        return i < 0 ? PriorityRank.Length : i;
    }

    static int NumberOf(string key) =>
        int.TryParse(key[(key.LastIndexOf('-') + 1)..], out var n) ? n : int.MaxValue;
}
