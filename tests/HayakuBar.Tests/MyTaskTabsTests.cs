using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class MyTaskTabsTests
{
    static readonly DateOnly Today = new(2026, 10, 30);

    static TaskSummary Task(string key, string category, string? start = null, string? end = null, bool timing = false) =>
        new(key, key, "F2P", "", category, "", start is null ? null : DateOnly.Parse(start), end is null ? null : DateOnly.Parse(end), timing, "");

    static List<string> Keys(IReadOnlyList<TaskSummary> list) => list.Select(t => t.Key).ToList();

    [Fact]
    public void 四個分頁照網頁的規則分()
    {
        var tabs = MyTaskTabs.Split(
        [
            Task("A", "IN_PROGRESS", "2026-10-01", "2026-10-14"), // 進行中、逾期
            Task("B", "TODO"), // 未開始，沒有日期
            Task("C", "TODO", "2026-10-29", "2026-11-05"), // 未開始、該開始
            Task("D", "TODO", "2026-10-30", timing: true), // 正在計時：算進行中，不算未開始、該開始
            Task("E", "TODO", "2026-11-02", "2026-10-30"), // 今天到期：不算逾期；還沒到開始日
            Task("F", "TODO", "2026-10-20", "2026-10-25"), // 未開始、該開始、逾期
        ], Today);
        Assert.Equal(["A", "D"], Keys(tabs["in_progress"]));
        Assert.Equal(["B", "C", "E", "F"], Keys(tabs["not_started"]));
        Assert.Equal(["A", "F"], Keys(tabs["overdue"]));
        Assert.Equal(["C", "F"], Keys(tabs["should_start"]));
    }

    [Fact]
    public void 分頁涵蓋所有引擎用到的分頁() =>
        Assert.Equal(TimerEngine.TaskTabs.Select(t => t.Tab).Order(), MyTaskTabs.Split([], Today).Keys.Order());
}
