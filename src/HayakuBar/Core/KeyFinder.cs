using System.Text.RegularExpressions;

namespace HayakuBar.Core;

/// <summary>從文字找出 Hayaku 任務編號（例 F2P-97、EG-7）。</summary>
public static partial class KeyFinder
{
    [GeneratedRegex(@"(?<![A-Za-z0-9])([A-Za-z][A-Za-z0-9]{0,14})-(\d{1,6})(?![0-9])")]
    private static partial Regex KeyRegex();

    /// <summary>對話內容：只認大寫代碼，取最後一個提到的。</summary>
    public static string? FindInPrompt(string? text, Func<string, bool> isKnown)
    {
        if (string.IsNullOrEmpty(text))
            return null;
        string? last = null;
        foreach (Match m in KeyRegex().Matches(text))
        {
            var prefix = m.Groups[1].Value;
            if (prefix != prefix.ToUpperInvariant())
                continue;
            var key = $"{prefix}-{m.Groups[2].Value}";
            if (isKnown(key))
                last = key;
        }
        return last;
    }

    /// <summary>git 分支名稱：大小寫都認（feature/f2p-97-xxx），取第一個。</summary>
    public static string? FindInBranch(string? branch, Func<string, bool> isKnown)
    {
        if (string.IsNullOrEmpty(branch))
            return null;
        foreach (Match m in KeyRegex().Matches(branch))
        {
            var key = $"{m.Groups[1].Value.ToUpperInvariant()}-{m.Groups[2].Value}";
            if (isKnown(key))
                return key;
        }
        return null;
    }

    /// <summary>使用者輸入的文字（「F2P-97  報表頁面…」或「f2p-97」）→ 任務編號。</summary>
    public static string? Parse(string? text)
    {
        if (string.IsNullOrWhiteSpace(text))
            return null;
        var m = KeyRegex().Match(text.Trim());
        return m.Success ? $"{m.Groups[1].Value.ToUpperInvariant()}-{m.Groups[2].Value}" : null;
    }

    /// <summary>文字裡出現的所有任務編號（照出現順序，代碼轉大寫）。</summary>
    public static IEnumerable<string> All(string? text) =>
        string.IsNullOrEmpty(text) ? [] : KeyRegex().Matches(text).Select(m => $"{m.Groups[1].Value.ToUpperInvariant()}-{m.Groups[2].Value}");

    public static string ProjectOf(string key)
    {
        var dash = key.LastIndexOf('-');
        return dash > 0 ? key[..dash] : key;
    }
}
