using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class TaskTreeTests
{
    static ProjectTask T(string key, string? parent = null, string category = "IN_PROGRESS", string? title = null) =>
        new(key, title ?? key, "處理中", category, parent, null, null);

    static List<string> Flat(IEnumerable<TaskTreeNode> nodes, int depth = 0) =>
        nodes.SelectMany(n => new[] { new string('-', depth) + n.Task.Key }.Concat(Flat(n.Children, depth + 1))).ToList();

    [Fact]
    public void ParseProjectTasks_用路徑最後一層當上層()
    {
        var json = JsonDocument.Parse("""
            {"tasks":[
              {"task":"F2P-2","title":"文管系統開發","status":"處理中","statusCategory":"IN_PROGRESS","path":[],"plannedStart":"2026-09-01","plannedEnd":null},
              {"task":"F2P-14","title":"機密列印","status":"處理中","statusCategory":"IN_PROGRESS",
               "path":[{"task":"F2P-2","title":"文管系統開發"},{"task":"F2P-5","title":"第一階段研究"}]}
            ],"cursor":null}
            """).RootElement;
        var tasks = HayakuApi.ParseProjectTasks(json);
        Assert.Null(tasks[0].Parent);
        Assert.Equal(new DateOnly(2026, 9, 1), tasks[0].PlannedStart);
        Assert.Equal("F2P-5", tasks[1].Parent);
    }

    [Fact]
    public void Build_依上層組樹並保留順序_找不到上層的放最上層()
    {
        var tree = TaskTree.Build([T("A-1"), T("A-2", "A-1"), T("A-3", "A-2"), T("A-4", "A-1"), T("A-5", "A-99")], includeDone: true);
        Assert.Equal(["A-1", "-A-2", "--A-3", "-A-4", "A-5"], Flat(tree));
    }

    [Fact]
    public void Build_隱藏已完成但保留還有子任務的上層()
    {
        var tree = TaskTree.Build([T("A-1", category: "DONE"), T("A-2", "A-1"), T("A-3", category: "CANCELLED"), T("A-4", "A-1", "DONE")],
            includeDone: false);
        Assert.Equal(["A-1", "-A-2"], Flat(tree));
    }

    [Fact]
    public void Build_篩選時顯示符合的任務與它的上層()
    {
        var tree = TaskTree.Build([T("A-1", title: "文管"), T("A-2", "A-1", title: "列印"), T("A-3", "A-1", title: "掃描"), T("A-4", title: "其他")],
            includeDone: true, filter: "列印");
        Assert.Equal(["A-1", "-A-2"], Flat(tree));
    }
}
