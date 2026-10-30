using HayakuBar.Config;

namespace HayakuBar.Core;

/// <summary>工作台「我的任務」清單可以設定的欄位（顯示、順序、寬度）。</summary>
public static class TaskColumns
{
    /// <summary>所有欄位：Key、標題、預設寬度（名稱欄 0＝填滿）、預設是否顯示。計時按鈕固定在最前面，不算一欄。</summary>
    public static readonly IReadOnlyList<(string Key, string Label, double Width, bool Visible)> All =
    [
        ("key", "編號", 76, true),
        ("title", "名稱", 0, true),
        ("project", "專案", 150, true),
        ("status", "狀態", 96, true),
        ("priority", "優先級", 60, false),
        ("start", "預計開始", 84, false),
        ("end", "預計結束", 96, true),
        ("elapsed", "今天計時", 76, false),
    ];

    public const double MinWidth = 40;
    public const double MaxWidth = 480;

    public static string LabelOf(string key) => All.FirstOrDefault(c => c.Key == key).Label ?? key;

    /// <summary>
    /// 存的設定補成完整的欄位清單：照存的順序，存檔裡沒有的（新版加的欄位）照預設加在最後；
    /// 名稱欄一定顯示且寬度 0（填滿），其他欄寬度限制在合理範圍。
    /// </summary>
    public static List<TaskColumnConfig> Normalize(IEnumerable<TaskColumnConfig>? saved)
    {
        var list = new List<TaskColumnConfig>();
        foreach (var c in saved ?? [])
        {
            if (All.All(a => a.Key != c.Key) || list.Any(x => x.Key == c.Key))
                continue;
            list.Add(new TaskColumnConfig { Key = c.Key, Visible = c.Visible, Width = c.Width });
        }
        foreach (var a in All)
            if (list.All(x => x.Key != a.Key))
                list.Add(new TaskColumnConfig { Key = a.Key, Visible = a.Visible, Width = a.Width });
        foreach (var c in list)
        {
            if (c.Key == "title")
            {
                c.Visible = true;
                c.Width = 0;
            }
            else
            {
                var fallback = All.First(a => a.Key == c.Key).Width;
                c.Width = c.Width <= 0 ? fallback : Math.Clamp(c.Width, MinWidth, MaxWidth);
            }
        }
        return list;
    }

    /// <summary>把某欄往前（-1）或往後（+1）移一格。</summary>
    public static void Move(List<TaskColumnConfig> columns, string key, int delta)
    {
        var i = columns.FindIndex(c => c.Key == key);
        var j = i + delta;
        if (i < 0 || j < 0 || j >= columns.Count)
            return;
        (columns[i], columns[j]) = (columns[j], columns[i]);
    }
}
