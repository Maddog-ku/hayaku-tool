using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>我的任務樹的一個節點；Task 為 null 表示只是上層脈絡（不在這次的清單裡）。</summary>
public sealed class MyTaskNode(string key, string title)
{
    public string Key { get; } = key;
    public string Title { get; } = title;
    public TaskSummary? Task { get; set; }
    public List<MyTaskNode> Children { get; } = [];
}

/// <summary>一個專案底下的我的任務。</summary>
public sealed record MyTaskProject(string Project, List<MyTaskNode> Roots, int Count);

/// <summary>把我的任務依專案分組，再照上下層排成樹（上層不在清單裡的也補上當脈絡）。</summary>
public static class MyTaskTree
{
    /// <summary>專案與兄弟節點的順序照清單第一次出現的順序，所以排序方式照樣有效。</summary>
    public static List<MyTaskProject> Build(IEnumerable<TaskSummary> tasks)
    {
        var groups = new List<(string Project, List<MyTaskNode> Roots, Dictionary<string, MyTaskNode> Nodes, int Count)>();
        foreach (var t in tasks)
        {
            var index = groups.FindIndex(g => g.Project == t.Project);
            if (index < 0)
            {
                groups.Add((t.Project, [], new Dictionary<string, MyTaskNode>(StringComparer.OrdinalIgnoreCase), 0));
                index = groups.Count - 1;
            }
            var group = groups[index];
            groups[index] = group with { Count = group.Count + 1 };

            var siblings = group.Roots;
            foreach (var (key, title) in (t.Ancestors ?? []).Append((t.Key, t.Title)))
            {
                if (!group.Nodes.TryGetValue(key, out var node))
                {
                    node = new MyTaskNode(key, title);
                    group.Nodes[key] = node;
                    siblings.Add(node);
                }
                if (string.Equals(key, t.Key, StringComparison.OrdinalIgnoreCase))
                    node.Task = t;
                siblings = node.Children;
            }
        }
        return groups.Select(g => new MyTaskProject(g.Project, g.Roots, g.Count)).ToList();
    }
}
