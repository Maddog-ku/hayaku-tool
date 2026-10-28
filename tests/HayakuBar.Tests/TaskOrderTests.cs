using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class TaskOrderTests
{
    static TaskSummary Task(string key, string status = "處理中", string priority = "", int? dueDay = null, bool timing = false) =>
        new(key, key, "P", status, "", priority, null, dueDay is { } d ? new DateOnly(2026, 10, d) : null, timing, "");

    static readonly HashSet<string> NoneRunning = [];

    [Fact]
    public void Sort_預計結束近到遠且沒日期的放最後()
    {
        var sorted = TaskOrder.Sort([Task("A-1"), Task("A-2", dueDay: 20), Task("A-3", dueDay: 5)], "due", NoneRunning);
        Assert.Equal(["A-3", "A-2", "A-1"], sorted.Select(t => t.Key));
        var desc = TaskOrder.Sort([Task("A-1"), Task("A-2", dueDay: 20), Task("A-3", dueDay: 5)], "dueDesc", NoneRunning);
        Assert.Equal(["A-2", "A-3", "A-1"], desc.Select(t => t.Key));
    }

    [Fact]
    public void Sort_優先級緊急在前沒設的在後()
    {
        var sorted = TaskOrder.Sort([Task("A-1", priority: "無"), Task("A-2", priority: "低"), Task("A-3", priority: "緊急"), Task("A-4", priority: "中")],
            "priority", NoneRunning);
        Assert.Equal(["A-3", "A-4", "A-2", "A-1"], sorted.Select(t => t.Key));
    }

    [Fact]
    public void Sort_編號依數字大小不是字串()
    {
        var sorted = TaskOrder.Sort([Task("F2P-21"), Task("EG-7"), Task("F2P-5"), Task("EG-11")], "key", NoneRunning);
        Assert.Equal(["EG-7", "EG-11", "F2P-5", "F2P-21"], sorted.Select(t => t.Key));
    }

    [Fact]
    public void Sort_計時中優先且其餘維持原順序()
    {
        var sorted = TaskOrder.Sort([Task("A-1"), Task("A-2"), Task("A-3", timing: true), Task("A-4")], "timing", new HashSet<string> { "A-4" });
        Assert.Equal(["A-3", "A-4", "A-1", "A-2"], sorted.Select(t => t.Key));
    }

    [Fact]
    public void Statuses_依數量多到少()
    {
        var statuses = TaskOrder.Statuses([Task("A-1", "待客戶回應"), Task("A-2"), Task("A-3"), Task("A-4", "")]);
        Assert.Equal([("處理中", 2), ("待客戶回應", 1)], statuses);
    }
}
