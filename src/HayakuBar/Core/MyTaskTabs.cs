using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>
/// 「我的任務」四個分頁，從一份未完成的任務清單（my_tasks 不給分頁）在本機算，跟網頁同一套規則：
/// 進行中（含我正在計時的）、測試（指派給我的測試任務與測試結果）、未開始（待處理，不分日期）、
/// 逾期（預計結束已過，含進行中的）、該開始還沒開始。
/// 順序照清單原本的順序（跟網頁一樣）。
/// </summary>
public static class MyTaskTabs
{
    static bool InProgress(TaskSummary t) => t.StatusCategory == "IN_PROGRESS" || t.Timing;

    public static Dictionary<string, IReadOnlyList<TaskSummary>> Split(IReadOnlyList<TaskSummary> open, DateOnly today)
    {
        static bool NotStarted(TaskSummary t) => t.StatusCategory == "TODO" && !t.Timing;
        return new()
        {
            ["in_progress"] = open.Where(InProgress).ToList(),
            ["test"] = open.Where(t => t.Kind != TaskKind.Normal).ToList(),
            ["not_started"] = open.Where(NotStarted).ToList(),
            ["overdue"] = open.Where(t => t.PlannedEnd is { } end && end < today).ToList(),
            ["should_start"] = open.Where(t => NotStarted(t) && t.PlannedStart is { } start && start <= today).ToList(),
        };
    }

    /// <summary>「全部」分頁（Hayaku 1.5.1）：所有還沒完成的任務，進行中的排前面，其餘照清單原本的順序。</summary>
    public static IReadOnlyList<TaskSummary> All(IReadOnlyList<TaskSummary> open) =>
        open.Where(InProgress).Concat(open.Where(t => !InProgress(t))).ToList();
}
