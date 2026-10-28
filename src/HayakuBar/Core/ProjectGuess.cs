namespace HayakuBar.Core;

/// <summary>替資料夾猜要建在哪個 Hayaku 專案。</summary>
public static class ProjectGuess
{
    /// <summary>
    /// 依序：這個資料夾上次用的專案 → 專案名稱或代碼跟資料夾名稱相同 → 互相包含（至少 3 個字）→ 正在計時的任務所屬專案；
    /// 都沒有回 null。projects 是 (代碼, 名稱)。
    /// </summary>
    public static string? Guess(IReadOnlyList<(string Key, string Name)> projects, string? hinted, string folderName,
        IEnumerable<string> runningProjects)
    {
        if (hinted is not null && projects.FirstOrDefault(p => Same(p.Key, hinted)) is { Key: not null } h)
            return h.Key;
        var folder = Simplify(folderName);
        if (folder.Length > 0)
        {
            foreach (var p in projects)
                if (Simplify(p.Name) == folder || Simplify(p.Key) == folder)
                    return p.Key;
            foreach (var p in projects)
            {
                var name = Simplify(p.Name);
                if (name.Length >= 3 && folder.Length >= 3 && (name.Contains(folder) || folder.Contains(name)))
                    return p.Key;
            }
        }
        foreach (var running in runningProjects)
            if (projects.FirstOrDefault(p => Same(p.Key, running)) is { Key: not null } r)
                return r.Key;
        return null;
    }

    static bool Same(string a, string b) => string.Equals(a, b, StringComparison.OrdinalIgnoreCase);

    /// <summary>只留字母與數字（含中文），轉小寫：Print-App、print_app、Print App 都一樣。</summary>
    static string Simplify(string text) =>
        new(text.Where(char.IsLetterOrDigit).Select(char.ToLowerInvariant).ToArray());
}
