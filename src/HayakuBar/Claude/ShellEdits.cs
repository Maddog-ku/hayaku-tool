using System.Text.RegularExpressions;

namespace HayakuBar.Claude;

/// <summary>
/// 判斷 Claude 跑的 Bash／PowerShell 指令有沒有在專案裡寫檔：Claude 常用 sed -i、python 腳本改程式，不一定走 Edit 工具。
/// 只認常見的寫法：sed／perl -i、重導向或 tee 到專案裡的檔案、Set-Content 類、git apply／patch、
/// 交給 python／node 執行的腳本（heredoc 或 -c）裡開檔寫入。
/// 純讀取（sed -n、grep、cat、git status）、寫到專案外（暫存資料夾、/dev/null）、只是把腳本寫成檔案的都不算。
/// 所在資料夾照指令先後順序追 cd（變數也會展開）；cd 到看不出來的地方，之後的動作就不算。
/// </summary>
public static partial class ShellEdits
{
    /// <summary>標記「這裡有一段會寫檔的腳本」，取代掉腳本內容後留在指令裡，才知道它在第幾個 cd 之後。</summary>
    const char ScriptMark = '\u0002';

    /// <summary>heredoc：&lt;&lt;'EOF' 之後到 EOF 那行；本文是腳本或文字內容，不當成 shell 指令看。</summary>
    [GeneratedRegex(@"<<-?\s*(['""]?)(\w+)\1([^\n]*)\n(.*?)(?:\n\2[ \t]*(?=\n|$)|$)", RegexOptions.Singleline)]
    private static partial Regex Heredoc();

    [GeneratedRegex(@"'[^']*'|""(?:[^""\\]|\\.)*""")]
    private static partial Regex Quoted();

    [GeneratedRegex("\u0001(\\d+)\u0001")]
    private static partial Regex Placeholder();

    [GeneratedRegex(@"(?:^|[\s;&|(])(?:python3?|py|node|perl|ruby|php)(?:\.exe)?(?=\s|$)", RegexOptions.IgnoreCase)]
    private static partial Regex Interpreter();

    [GeneratedRegex(@"(?:^|[\s;&|(])(?:python3?|py|node|perl|ruby|php)(?:\.exe)?\s+(?:-\S+\s+)*?-[ce]\s*$", RegexOptions.IgnoreCase)]
    private static partial Regex InlineScript();

    /// <summary>變數設定：bash 的 S="..."、PowerShell 的 $S = "..."。</summary>
    [GeneratedRegex(@"(?:^|[;&|(\n]|\s)(?:export\s+)?(?:\$(?<ps>\w+)\s*=\s*|(?<sh>[A-Za-z_]\w*)=)(?<value>[^\s;&|)]*)")]
    private static partial Regex Assignment();

    [GeneratedRegex(@"\$\{(\w+)\}|\$env:(\w+)|\$(\w+)", RegexOptions.IgnoreCase)]
    private static partial Regex Variable();

    [GeneratedRegex(@"(?:^|[;&|(\n])\s*(?:cd|Set-Location|pushd)\s+([^\s;&|)]+)", RegexOptions.IgnoreCase)]
    private static partial Regex ChangeDir();

    /// <summary>&gt; 或 &gt;&gt; 到某個檔案（2&gt;&amp;1、-&gt;、=&gt; 不算）。</summary>
    [GeneratedRegex(@"(?<![\w<>=\-$])(?:[12*]|&)?>>?\|?(?![&(>=])\s*([^\s;&|<>()]+)")]
    private static partial Regex Redirect();

    [GeneratedRegex(@"(?:^|[\s;&|(])tee\s+(?:-\S+\s+)*([^\s;&|<>()]+)")]
    private static partial Regex Tee();

    [GeneratedRegex(@"(?:^|[\s;&|(])(?:sed|perl)\s(?:[^;&|\n]*?\s)?-(?:[a-zA-Z]*i[a-zA-Z]*|-in-place)(?=[\s.=]|$)")]
    private static partial Regex InPlace();

    [GeneratedRegex(@"\b(?:Set-Content|Add-Content|Out-File|Clear-Content)\b|\[(?:System\.)?IO\.File\]::(?:Write|Append)|(?:^|[\s;&|(])(?:git\s+(?:apply|am|mv|rm)|patch)\s", RegexOptions.IgnoreCase)]
    private static partial Regex Writers();

    /// <summary>腳本裡寫檔：open(..., 'w')、write_text、writeFileSync、File.WriteAllText、os.replace…</summary>
    [GeneratedRegex(@"\bopen\([^)\n]*(?:,\s*|mode\s*=\s*)['""][wax]|\.write_(?:text|bytes)\(|\bwriteFileSync\(|\bFile\.(?:WriteAll|AppendAll)\w*\(|\bos\.(?:replace|rename)\(|\bshutil\.(?:copy\w*|move)\(")]
    private static partial Regex ScriptWrite();

    /// <summary>在 cwd 跑 command 會不會改到 root 底下的檔案。cwd 是那筆紀錄的工作目錄（沒有就當成 root）。</summary>
    public static bool Writes(string command, string? cwd, string root)
    {
        var shell = Heredoc().Replace(command, m =>
        {
            var lineStart = command.LastIndexOf('\n', Math.Max(0, m.Index - 1)) + 1;
            var runs = Interpreter().IsMatch(command[lineStart..m.Index]) && ScriptWrite().IsMatch(m.Groups[4].Value);
            return m.Groups[3].Value + (runs ? $" {ScriptMark}" : "");
        });
        var quoted = new List<string>();
        shell = Quoted().Replace(shell, m =>
        {
            quoted.Add(m.Value[1..^1]);
            var runs = InlineScript().IsMatch(shell[..m.Index]) && ScriptWrite().IsMatch(m.Value);
            return $"\u0001{quoted.Count - 1}\u0001" + (runs ? $" {ScriptMark}" : "");
        });

        var vars = new Dictionary<string, string?>(StringComparer.OrdinalIgnoreCase);
        string? Expand(string s)
        {
            s = Placeholder().Replace(s, m => quoted[int.Parse(m.Groups[1].Value)]);
            var unknown = false;
            s = Variable().Replace(s, m =>
            {
                var name = m.Groups[1].Success ? m.Groups[1].Value : m.Groups[2].Success ? m.Groups[2].Value : m.Groups[3].Value;
                var value = vars.TryGetValue(name, out var v) ? v : Environment.GetEnvironmentVariable(name);
                unknown |= value is null;
                return value ?? "";
            });
            return unknown ? null : s;
        }
        foreach (Match m in Assignment().Matches(shell))
            vars[m.Groups["ps"].Success ? m.Groups["ps"].Value : m.Groups["sh"].Value] = Expand(m.Groups["value"].Value);

        // 每個 cd 之後的所在資料夾；null 表示看不出來
        var dirs = new List<(int At, string? Dir)> { (0, Resolve(root, cwd ?? root) ?? root) };
        foreach (Match m in ChangeDir().Matches(shell))
            dirs.Add((m.Index, Expand(m.Groups[1].Value) is { } target ? Resolve(dirs[^1].Dir, target) : null));
        string? DirAt(int index) => dirs.Last(d => d.At <= index).Dir;

        foreach (var m in Redirect().Matches(shell).Concat(Tee().Matches(shell)))
            if (Expand(m.Groups[1].Value) is { } path && Resolve(DirAt(m.Index), path) is { } target && ClaudeMonitor.IsUnder(target, root))
                return true;

        var writes = InPlace().Matches(shell).Concat(Writers().Matches(shell)).Select(m => m.Index)
            .Concat(shell.Select((c, i) => (c, i)).Where(x => x.c == ScriptMark).Select(x => x.i));
        return writes.Any(i => DirAt(i) is { } dir && ClaudeMonitor.IsUnder(dir, root));
    }

    /// <summary>把指令裡的路徑轉成完整路徑；~、/dev/null 這類看不出來或不是檔案的回 null。</summary>
    static string? Resolve(string? baseDir, string path)
    {
        if (path.Length == 0 || path[0] is '~' or '-' or '%' || path.Contains('*') || path.Contains('$')
            || path.StartsWith("/dev/", StringComparison.Ordinal) || path.Equals("nul", StringComparison.OrdinalIgnoreCase))
            return null;
        var rooted = path.StartsWith('/') || Path.IsPathRooted(path);
        if (!rooted && baseDir is null)
            return null;
        try
        {
            return ClaudeMonitor.Normalize(rooted ? path : Path.Combine(baseDir!, path));
        }
        catch (Exception ex) when (ex is ArgumentException or NotSupportedException or PathTooLongException)
        {
            return null;
        }
    }
}
