using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>專案樹的一個節點。</summary>
public sealed class TaskTreeNode(ProjectTask task)
{
    public ProjectTask Task { get; } = task;
    public List<TaskTreeNode> Children { get; } = [];
}

/// <summary>把 list_tasks 的平面清單組成專案樹，並依「顯示已完成」與關鍵字篩選。</summary>
public static class TaskTree
{
    /// <summary>
    /// 組樹：Parent 找不到（例如沒權限看）的當成最上層；順序照原清單。
    /// 隱藏已完成／不處理的任務，但底下還有要顯示的子任務時保留，樹才不會斷掉。
    /// filter：編號或名稱包含；符合的任務連同它的上層都會顯示。
    /// </summary>
    public static List<TaskTreeNode> Build(IEnumerable<ProjectTask> tasks, bool includeDone, string? filter = null)
    {
        var all = tasks.ToList();
        var nodes = all.ToDictionary(t => t.Key, t => new TaskTreeNode(t), StringComparer.OrdinalIgnoreCase);
        var roots = new List<TaskTreeNode>();
        foreach (var t in all)
        {
            if (t.Parent is { } p && nodes.TryGetValue(p, out var parent) && !ReferenceEquals(parent, nodes[t.Key]))
                parent.Children.Add(nodes[t.Key]);
            else
                roots.Add(nodes[t.Key]);
        }
        var q = filter?.Trim() ?? "";
        return Prune(roots, includeDone, q);
    }

    static List<TaskTreeNode> Prune(List<TaskTreeNode> nodes, bool includeDone, string q)
    {
        var kept = new List<TaskTreeNode>();
        foreach (var n in nodes)
        {
            var children = Prune(n.Children, includeDone, q);
            var done = n.Task.StatusCategory is "DONE" or "CANCELLED";
            var matches = q.Length == 0
                          || n.Task.Key.Contains(q, StringComparison.OrdinalIgnoreCase)
                          || n.Task.Title.Contains(q, StringComparison.OrdinalIgnoreCase);
            if (children.Count == 0 && (!matches || done && !includeDone))
                continue;
            var copy = new TaskTreeNode(n.Task);
            copy.Children.AddRange(children);
            kept.Add(copy);
        }
        return kept;
    }
}
