using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class Hayaku151Tests
{
    static TaskSummary Task(string key, string category, bool timing = false) =>
        new(key, key, "F2P", "", category, "", null, null, timing, "");

    [Fact]
    public void 全部分頁_進行中排前面_其餘照原本順序()
    {
        var all = MyTaskTabs.All(
        [
            Task("A", "TODO"),
            Task("B", "IN_PROGRESS"),
            Task("C", "TODO"),
            Task("D", "TODO", timing: true), // 正在計時算進行中
            Task("E", "IN_PROGRESS"),
        ]);
        Assert.Equal(["B", "D", "E", "A", "C"], all.Select(t => t.Key));
    }

    [Fact]
    public void 全部分頁_沒有漏掉任何任務()
    {
        // 不屬於任何狀態分頁的類別也要在「全部」裡
        var all = MyTaskTabs.All([Task("A", "TODO"), Task("B", "OTHER")]);
        Assert.Equal(["A", "B"], all.Select(t => t.Key));
    }
}
