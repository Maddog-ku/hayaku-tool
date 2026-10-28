using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class MyTaskTreeTests
{
    static TaskSummary T(string key, string project, params (string, string)[] ancestors) =>
        new(key, key + "名稱", project, "處理中", "IN_PROGRESS", "無", null, null, false, "", ancestors);

    static List<string> Flat(IEnumerable<MyTaskNode> nodes, int depth = 0) =>
        nodes.SelectMany(n => new[] { new string('-', depth) + n.Key + (n.Task is null ? "(上層)" : "") }
            .Concat(Flat(n.Children, depth + 1))).ToList();

    [Fact]
    public void Build_依專案分組並補上不在清單裡的上層()
    {
        var groups = MyTaskTree.Build([
            T("F2P-15", "F2P", ("F2P-5", "第一階段"), ("F2P-14", "機密列印")),
            T("EG-7", "EG"),
            T("F2P-14", "F2P", ("F2P-5", "第一階段")),
            T("F2P-6", "F2P", ("F2P-5", "第一階段")),
            T("EG-8", "EG", ("EG-7", "")),
        ]);
        Assert.Equal(["F2P", "EG"], groups.Select(g => g.Project));
        Assert.Equal(["F2P-5(上層)", "-F2P-14", "--F2P-15", "-F2P-6"], Flat(groups[0].Roots));
        Assert.Equal(3, groups[0].Count);
        Assert.Equal(["EG-7", "-EG-8"], Flat(groups[1].Roots));
    }
}
