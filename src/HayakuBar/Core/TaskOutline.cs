namespace HayakuBar.Core;

/// <summary>批次新增任務的一個節點：名稱與底下的子任務。</summary>
public sealed record OutlineNode(string Title, List<OutlineNode> Children)
{
    public int Count => 1 + Children.Sum(c => c.Count);
}

/// <summary>
/// 把貼上的清單變成任務樹：一行一個任務，用縮排（Tab 或空白）表示是上一個縮排較少那行的子任務。
/// 行首的項目符號（-、*、•、1.、1)、[ ]）會拿掉，空白行略過。
/// </summary>
public static class TaskOutline
{
    public static List<OutlineNode> Parse(string? text)
    {
        var roots = new List<OutlineNode>();
        var stack = new List<(int Indent, OutlineNode Node)>();
        foreach (var raw in (text ?? "").Replace("\r\n", "\n").Split('\n'))
        {
            var indent = 0;
            foreach (var c in raw)
            {
                if (c == ' ')
                    indent++;
                else if (c == '\t')
                    indent += 4;
                else if (c == '　')
                    indent += 2;
                else
                    break;
            }
            var title = StripBullet(raw.Trim());
            if (title.Length == 0)
                continue;
            var node = new OutlineNode(title.Length > 200 ? title[..200] : title, []);
            while (stack.Count > 0 && stack[^1].Indent >= indent)
                stack.RemoveAt(stack.Count - 1);
            if (stack.Count == 0)
                roots.Add(node);
            else
                stack[^1].Node.Children.Add(node);
            stack.Add((indent, node));
        }
        return roots;
    }

    public static int Count(IEnumerable<OutlineNode> nodes) => nodes.Sum(n => n.Count);

    static string StripBullet(string line)
    {
        if (line.StartsWith("[ ]") || line.StartsWith("[x]", StringComparison.OrdinalIgnoreCase))
            return line[3..].Trim();
        if (line.Length > 1 && line[0] is '-' or '*' or '•' or '・' or '+' && line[1] == ' ')
            return line[2..].Trim();
        var i = 0;
        while (i < line.Length && char.IsAsciiDigit(line[i]))
            i++;
        // 1. 標題、1) 標題、1、標題
        var numbered = i > 0 && i < line.Length - 1 && (line[i] == '、' || line[i] is '.' or ')' && line[i + 1] == ' ');
        if (numbered)
            return line[(i + 1)..].Trim();
        return line;
    }
}
