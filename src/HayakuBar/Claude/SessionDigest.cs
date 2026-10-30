using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Claude;

/// <summary>某天在 Claude Code 送出的一句指令（已整理成一行摘要）。</summary>
public sealed record ClaudePrompt(DateTime At, string Folder, string? Task, string Summary);

/// <summary>某天在 repo 裡的一個 commit（只算自己的）。</summary>
public sealed record GitCommit(DateTime At, string Folder, string Hash, string Subject, string? Task);

/// <summary>
/// 把一天的 Claude Code 對話與 git commit 整理成工作日誌草稿，也能替還沒寫備註的計時段產生備註。
/// 只讀「專案」頁設定過的資料夾；任務判斷跟自動計時一樣：手動指定 → 對話裡的編號 → 分支；都沒有就歸到「其他」。
/// </summary>
public static partial class SessionDigest
{
    const int SummaryLength = 60;
    const int NoteLength = 200;

    /// <summary>沒有內容的回覆（繼續、好、yes…），不寫進日誌。</summary>
    static readonly HashSet<string> Noise = new(StringComparer.OrdinalIgnoreCase)
    {
        "continue", "go", "go on", "ok", "okay", "yes", "y", "no", "n", "sure", "thanks", "thank you",
        "繼續", "好", "好的", "對", "是", "可以", "沒問題", "謝謝", "嗯", "收到", "開始", "同意", "確定", "執行",
    };

    /// <summary>貼上的圖片標記（[Image #3]）。</summary>
    [GeneratedRegex(@"\[Image #\d+\]")]
    private static partial Regex ImageTag();

    /// <summary>只是在下指令的短句（commit 並 push、直接做到完成…），沒有工作內容。</summary>
    [GeneratedRegex("commit|push|[直值]到完成|做完|繼續", RegexOptions.IgnoreCase)]
    private static partial Regex Instruction();

    /// <summary>讀某天所有在監看資料夾裡的 session，整理出每句指令。</summary>
    public static List<ClaudePrompt> Collect(DateOnly day, IReadOnlyCollection<RepoActivity> units,
        IReadOnlyDictionary<string, string> sessionTasks, Func<string, bool> isKnown)
    {
        var result = new List<ClaudePrompt>();
        var root = ClaudeMonitor.ProjectsRoot;
        var enabled = units.Where(u => u.Enabled).ToList();
        if (!Directory.Exists(root) || enabled.Count == 0)
            return result;
        var dayStart = day.ToDateTime(TimeOnly.MinValue);
        // 只看各專案資料夾第一層的 session 檔（子代理的紀錄放在更深的資料夾，不是自己送出的指令）
        foreach (var file in new DirectoryInfo(root).EnumerateDirectories().SelectMany(d => d.EnumerateFiles("*.jsonl")))
        {
            if (file.LastWriteTime < dayStart || file.CreationTime >= dayStart.AddDays(1))
                continue;
            try
            {
                using var fs = new FileStream(file.FullName, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
                using var reader = new StreamReader(fs, Encoding.UTF8);
                var lines = new List<string>();
                while (reader.ReadLine() is { } line)
                    lines.Add(line);
                var session = Path.GetFileNameWithoutExtension(file.Name);
                result.AddRange(Parse(lines, day, enabled, sessionTasks.GetValueOrDefault(session), isKnown));
            }
            catch (IOException ex)
            {
                Log.Error($"讀取 Claude session {file.Name} 失敗", ex);
            }
        }
        result.Sort((a, b) => a.At.CompareTo(b.At));
        return result;
    }

    /// <summary>解析一個 session 檔的每一行，取出當天使用者送出的指令。</summary>
    public static List<ClaudePrompt> Parse(IEnumerable<string> lines, DateOnly day, IReadOnlyCollection<RepoActivity> units,
        string? pickedTask, Func<string, bool> isKnown)
    {
        var list = new List<ClaudePrompt>();
        string? cwd = null, branch = null, promptKey = null;
        foreach (var line in lines)
        {
            if (line.Length < 2)
                continue;
            JsonElement r;
            try
            {
                using var doc = JsonDocument.Parse(line);
                r = doc.RootElement.Clone();
            }
            catch (JsonException)
            {
                continue;
            }
            if (r.ValueKind != JsonValueKind.Object)
                continue;
            if (Str(r, "cwd") is { Length: > 0 } c)
                cwd = c;
            if (Str(r, "gitBranch") is { Length: > 0 } b)
                branch = b;
            if (Str(r, "type") != "user" || Flag(r, "isMeta") || Flag(r, "isSidechain")
                || !r.TryGetProperty("message", out var msg) || msg.ValueKind != JsonValueKind.Object
                || !msg.TryGetProperty("content", out var content))
                continue;
            var text = PromptText(content);
            if (KeyFinder.FindInPrompt(text, isKnown) is { } key)
                promptKey = key; // 跟自動計時一樣：提到的編號延續到之後的指令
            if (Str(r, "timestamp") is not { } ts
                || !DateTimeOffset.TryParse(ts, CultureInfo.InvariantCulture, DateTimeStyles.None, out var at)
                || DateOnly.FromDateTime(at.LocalDateTime) != day
                || cwd is null
                || Summarize(text) is not { } summary)
                continue;
            string folder;
            try
            {
                folder = ClaudeMonitor.Normalize(cwd);
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                continue;
            }
            var unit = units.Where(u => ClaudeMonitor.IsUnder(folder, u.Path)).MaxBy(u => u.Path.Length);
            if (unit is null)
                continue;
            var task = pickedTask ?? promptKey ?? KeyFinder.FindInBranch(branch, isKnown);
            list.Add(new ClaudePrompt(at.LocalDateTime, unit.Path, task, summary));
        }
        return list;
    }

    /// <summary>一句指令 → 一行摘要（第一行有內容的文字，最多 60 字）；指令、系統訊息、「繼續」這類回覆回 null。</summary>
    public static string? Summarize(string? prompt)
    {
        if (string.IsNullOrWhiteSpace(prompt))
            return null;
        var text = prompt.Trim();
        if (text.StartsWith('<') || text.StartsWith('/') || text.StartsWith("[Request interrupted", StringComparison.Ordinal)
            || text.StartsWith("Caveat:", StringComparison.Ordinal))
            return null;
        var first = text.Split('\n').Select(l => ImageTag().Replace(l, "").Trim()).FirstOrDefault(l => l.Length > 0) ?? "";
        first = first.TrimEnd('。', '.', '，', ',', '？', '?', '！', '!', ' ');
        if (first.Length < 2 || Noise.Contains(first) || first.Length <= 20 && Instruction().IsMatch(first))
            return null;
        return first.Length > SummaryLength ? first[..SummaryLength] + "…" : first;
    }

    /// <summary>拿掉貼上的圖片標記（[Image #3]）。</summary>
    public static string StripImageTags(string text) => ImageTag().Replace(text, "");

    static string? PromptText(JsonElement content)
    {
        if (content.ValueKind == JsonValueKind.String)
            return content.GetString();
        if (content.ValueKind != JsonValueKind.Array)
            return null;
        var sb = new StringBuilder();
        foreach (var part in content.EnumerateArray())
            if (part.ValueKind == JsonValueKind.Object && Str(part, "type") == "text" && Str(part, "text") is { } t)
                sb.AppendLine(t);
        return sb.ToString();
    }

    // ───────────── git ─────────────

    /// <summary>這些資料夾當天自己的 commit（用各 repo 的 user.email 篩作者）；不是 git repo 或沒裝 git 就略過。</summary>
    public static async Task<List<GitCommit>> CommitsAsync(DateOnly day, IEnumerable<string> folders, IReadOnlyList<ClaudePrompt> prompts,
        Func<string, bool> isKnown)
    {
        var list = new List<GitCommit>();
        var since = day.ToDateTime(TimeOnly.MinValue).ToString("yyyy-MM-dd HH:mm:ss", CultureInfo.InvariantCulture);
        var until = day.ToDateTime(TimeOnly.MinValue).AddDays(1).ToString("yyyy-MM-dd HH:mm:ss", CultureInfo.InvariantCulture);
        foreach (var folder in folders.Distinct(StringComparer.OrdinalIgnoreCase))
        {
            if (!Directory.Exists(Path.Combine(folder, ".git")) && !File.Exists(Path.Combine(folder, ".git")))
                continue;
            var email = (await GitAsync(folder, "config", "user.email"))?.Trim();
            if (string.IsNullOrEmpty(email))
                continue;
            var output = await GitAsync(folder, "log", "--all", "--no-merges", $"--since={since}", $"--until={until}",
                $"--author={email}", "--format=%h%x09%aI%x09%s");
            if (output is null)
                continue;
            // 沒寫編號的 commit 歸給這個資料夾當天最常出現的任務
            var folderTask = prompts.Where(p => string.Equals(p.Folder, folder, StringComparison.OrdinalIgnoreCase) && p.Task is not null)
                .GroupBy(p => p.Task!, StringComparer.OrdinalIgnoreCase)
                .MaxBy(g => g.Count())?.Key;
            foreach (var line in output.Split('\n', StringSplitOptions.RemoveEmptyEntries))
            {
                var parts = line.Split('\t', 3);
                if (parts.Length < 3 || !DateTimeOffset.TryParse(parts[1], CultureInfo.InvariantCulture, DateTimeStyles.None, out var at))
                    continue;
                var subject = parts[2].Trim();
                list.Add(new GitCommit(at.LocalDateTime, folder, parts[0], subject, KeyFinder.FindInBranch(subject, isKnown) ?? folderTask));
            }
        }
        list.Sort((a, b) => a.At.CompareTo(b.At));
        return list;
    }

    static async Task<string?> GitAsync(string folder, params string[] args)
    {
        try
        {
            var psi = new ProcessStartInfo("git")
            {
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
                StandardOutputEncoding = Encoding.UTF8,
            };
            psi.ArgumentList.Add("-C");
            psi.ArgumentList.Add(folder);
            foreach (var a in args)
                psi.ArgumentList.Add(a);
            using var p = Process.Start(psi);
            if (p is null)
                return null;
            using var cts = new CancellationTokenSource(TimeSpan.FromSeconds(10));
            var output = p.StandardOutput.ReadToEndAsync(cts.Token);
            await p.WaitForExitAsync(cts.Token);
            return p.ExitCode == 0 ? await output : null;
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or OperationCanceledException or InvalidOperationException)
        {
            Log.Error($"git {string.Join(' ', args)}（{folder}）失敗", ex);
            return null;
        }
    }

    // ───────────── 輸出 ─────────────

    /// <summary>整理成 Markdown 草稿：依任務分組（照工時多到少），列出做了什麼與 commit。</summary>
    public static string Markdown(DateOnly day, IReadOnlyList<ClaudePrompt> prompts, IReadOnlyList<GitCommit> commits,
        IReadOnlyList<WorkLogEntry> logEntries, Func<string, string?> titleOf)
    {
        var minutes = logEntries.ToDictionary(e => e.Task, e => e.Minutes, StringComparer.OrdinalIgnoreCase);
        const string none = "";
        var groups = prompts.Select(p => p.Task ?? none)
            .Concat(commits.Select(c => c.Task ?? none))
            .Concat(logEntries.Select(e => e.Task))
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .OrderBy(t => t == none)
            .ThenByDescending(t => minutes.GetValueOrDefault(t))
            .ToList();
        var sb = new StringBuilder();
        sb.AppendLine($"# {day.ToString("M/d（ddd）", new CultureInfo("zh-TW"))} 工作日誌");
        foreach (var task in groups)
        {
            var items = prompts.Where(p => string.Equals(p.Task ?? none, task, StringComparison.OrdinalIgnoreCase))
                .Select(p => p.Summary)
                .Distinct()
                .ToList();
            var taskCommits = commits.Where(c => string.Equals(c.Task ?? none, task, StringComparison.OrdinalIgnoreCase)).ToList();
            sb.AppendLine();
            if (task == none)
                sb.AppendLine("## 其他（沒對應到任務）");
            else
                sb.AppendLine($"## {task} {titleOf(task)}".TrimEnd() +
                              (minutes.GetValueOrDefault(task) is var m and > 0 ? $"（{WorkMath.HoursMinutes(m)}）" : ""));
            foreach (var item in items)
                sb.AppendLine($"- {item}");
            foreach (var c in taskCommits)
                sb.AppendLine($"- commit {c.Hash}：{c.Subject}");
            if (items.Count == 0 && taskCommits.Count == 0)
                sb.AppendLine("- （沒有 Claude 對話紀錄）");
        }
        return sb.ToString().TrimEnd() + Environment.NewLine;
    }

    /// <summary>
    /// 還沒有備註的計時段 → 建議的備註：同任務、在這段時間內送出的指令與 commit（最多 200 字）。
    /// 找不到對應內容的計時段不會出現在結果裡。
    /// </summary>
    public static Dictionary<string, string> NotesFor(IReadOnlyList<TimeEntry> entries, IReadOnlyList<ClaudePrompt> prompts,
        IReadOnlyList<GitCommit> commits, DateTime now)
    {
        var notes = new Dictionary<string, string>();
        foreach (var e in entries)
        {
            if (!string.IsNullOrWhiteSpace(e.Note) || e.StartedAt is not { } started)
                continue;
            var start = started.LocalDateTime;
            var end = e.Running ? now : e.EndedAt?.LocalDateTime ?? start;
            bool Inside(string? task, DateTime at) =>
                string.Equals(task, e.Task, StringComparison.OrdinalIgnoreCase) && at >= start && at <= end;
            var parts = prompts.Where(p => Inside(p.Task, p.At)).Select(p => p.Summary)
                .Concat(commits.Where(c => Inside(c.Task, c.At)).Select(c => c.Subject))
                .Distinct()
                .ToList();
            if (parts.Count == 0)
                continue;
            var note = string.Join("；", parts);
            notes[e.Id] = note.Length > NoteLength ? note[..(NoteLength - 1)] + "…" : note;
        }
        return notes;
    }

    static bool Flag(JsonElement e, string name) => e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.True;

    static string? Str(JsonElement e, string name) =>
        e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
}
