using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using HayakuBar.Config;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.Claude;

public sealed class RepoActivity(RepoConfig repo)
{
    public RepoConfig Repo { get; set; } = repo;
    public string Name => Path.GetFileName(Repo.Path.TrimEnd('\\', '/'));

    public DateTime LastActivity { get; set; } = DateTime.MinValue;
    public DateTime? EndedAt { get; set; }
    public string? SessionId { get; set; }
    public string? SessionFile { get; set; }
    public long FileOffset { get; set; }
    public string? Branch { get; set; }
    public string? PromptKey { get; set; }

    public bool Active { get; set; }
    public string? ResolvedTask { get; set; }
    public string? TaskSource { get; set; }
}

/// <summary>
/// 監看設定的 repo 裡 Claude Code 的活動。主要來源是 ~/.claude/projects/&lt;編碼路徑&gt;/*.jsonl
/// （每行都有 cwd、gitBranch、timestamp），Claude Code hooks 只是讓反應更即時。
/// </summary>
public sealed class ClaudeMonitor
{
    const int InitialTailBytes = 512 * 1024;

    readonly Dictionary<string, RepoActivity> _repos = new(StringComparer.OrdinalIgnoreCase);

    public Func<string, bool> IsKnownKey { get; set; } = _ => true;

    public IReadOnlyCollection<RepoActivity> Repos => _repos.Values;

    public static string ProjectsRoot
    {
        get
        {
            var configDir = Environment.GetEnvironmentVariable("CLAUDE_CONFIG_DIR");
            var root = string.IsNullOrWhiteSpace(configDir)
                ? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".claude")
                : configDir;
            return Path.Combine(root, "projects");
        }
    }

    public static string Normalize(string path) => Path.GetFullPath(path).TrimEnd('\\', '/');

    /// <summary>Claude Code 的專案資料夾命名：路徑中非英數字元都換成 '-'（D:\sj\hayaku tool → D--sj-hayaku-tool）。</summary>
    public static string Encode(string path) => Regex.Replace(Normalize(path), "[^A-Za-z0-9]", "-");

    public void Sync(IEnumerable<RepoConfig> repos)
    {
        var keep = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var repo in repos)
        {
            if (string.IsNullOrWhiteSpace(repo.Path))
                continue;
            var key = Normalize(repo.Path);
            keep.Add(key);
            if (_repos.TryGetValue(key, out var existing))
                existing.Repo = repo;
            else
                _repos[key] = new RepoActivity(repo);
        }
        foreach (var key in _repos.Keys.Where(k => !keep.Contains(k)).ToList())
            _repos.Remove(key);
    }

    public void Scan()
    {
        var root = ProjectsRoot;
        if (!Directory.Exists(root))
            return;

        foreach (var (path, activity) in _repos)
        {
            var dir = Path.Combine(root, Encode(path));
            if (!Directory.Exists(dir))
                continue;

            FileInfo? newest = null;
            try
            {
                foreach (var file in new DirectoryInfo(dir).EnumerateFiles("*.jsonl"))
                    if (newest is null || file.LastWriteTime > newest.LastWriteTime)
                        newest = file;
            }
            catch (Exception ex)
            {
                Log.Error($"列出 {dir} 失敗", ex);
            }
            if (newest is null)
                continue;

            if (!string.Equals(activity.SessionFile, newest.FullName, StringComparison.OrdinalIgnoreCase))
            {
                activity.SessionFile = newest.FullName;
                activity.SessionId = Path.GetFileNameWithoutExtension(newest.Name);
                activity.PromptKey = null;
                activity.Branch = null;
                activity.FileOffset = Math.Max(0, newest.Length - InitialTailBytes);
            }

            var write = newest.LastWriteTime;
            if (write > activity.LastActivity && (activity.EndedAt is null || write > activity.EndedAt.Value.AddSeconds(5)))
            {
                activity.LastActivity = write;
                activity.EndedAt = null;
            }

            if (newest.Length < activity.FileOffset)
                activity.FileOffset = 0;
            if (newest.Length > activity.FileOffset)
                ReadNewLines(activity, newest);
        }
    }

    void ReadNewLines(RepoActivity activity, FileInfo file)
    {
        try
        {
            using var fs = new FileStream(file.FullName, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            var length = fs.Length - activity.FileOffset;
            if (length > 4 * 1024 * 1024)
            {
                activity.FileOffset = fs.Length - InitialTailBytes;
                length = InitialTailBytes;
            }
            fs.Seek(activity.FileOffset, SeekOrigin.Begin);
            var buffer = new byte[length];
            var read = fs.ReadAtLeast(buffer, buffer.Length, throwOnEndOfStream: false);
            var lastNewline = Array.LastIndexOf(buffer, (byte)'\n', read - 1);
            if (lastNewline < 0)
                return; // 還沒寫完一整行
            activity.FileOffset += lastNewline + 1;
            foreach (var line in Encoding.UTF8.GetString(buffer, 0, lastNewline + 1).Split('\n'))
                if (line.Length > 1)
                    ParseLine(activity, line);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 Claude session {file.Name} 失敗", ex);
        }
    }

    void ParseLine(RepoActivity activity, string line)
    {
        try
        {
            using var doc = JsonDocument.Parse(line);
            var r = doc.RootElement;
            if (r.ValueKind != JsonValueKind.Object)
                return;
            if (r.TryGetProperty("gitBranch", out var b) && b.ValueKind == JsonValueKind.String && b.GetString() is { Length: > 0 } branch)
                activity.Branch = branch;
            if (r.TryGetProperty("type", out var type) && type.GetString() == "user"
                && !(r.TryGetProperty("isMeta", out var meta) && meta.ValueKind == JsonValueKind.True)
                && r.TryGetProperty("message", out var msg) && msg.ValueKind == JsonValueKind.Object
                && msg.TryGetProperty("content", out var content)
                && KeyFinder.FindInPrompt(PromptText(content), IsKnownKey) is { } key)
                activity.PromptKey = key;
        }
        catch (JsonException)
        {
            // 從檔案中段開始讀時第一行可能不完整
        }
    }

    static string? PromptText(JsonElement content)
    {
        if (content.ValueKind == JsonValueKind.String)
            return content.GetString();
        if (content.ValueKind != JsonValueKind.Array)
            return null;
        var sb = new StringBuilder();
        foreach (var part in content.EnumerateArray())
            if (part.ValueKind == JsonValueKind.Object && part.TryGetProperty("type", out var t) && t.GetString() == "text"
                && part.TryGetProperty("text", out var text))
                sb.AppendLine(text.GetString());
        return sb.ToString();
    }

    /// <summary>處理 Claude Code hook 送來的事件；對應到設定的 repo 才回傳 true。</summary>
    public bool OnHook(JsonElement payload, DateTime now)
    {
        if (payload.ValueKind != JsonValueKind.Object || Str(payload, "cwd") is not { } cwd)
            return false;
        string normalized;
        try { normalized = Normalize(cwd); }
        catch { return false; }

        var activity = _repos
            .Where(kv => normalized.Equals(kv.Key, StringComparison.OrdinalIgnoreCase)
                         || normalized.StartsWith(kv.Key + "\\", StringComparison.OrdinalIgnoreCase))
            .Select(kv => kv.Value)
            .FirstOrDefault();
        if (activity is null)
            return false;

        if (Str(payload, "session_id") is { } sid && sid != activity.SessionId)
        {
            activity.SessionId = sid;
            activity.PromptKey = null;
        }

        switch (Str(payload, "hook_event_name"))
        {
            case "SessionStart":
                break;
            case "SessionEnd":
                activity.EndedAt = now;
                break;
            case "UserPromptSubmit":
                activity.LastActivity = now;
                activity.EndedAt = null;
                if (KeyFinder.FindInPrompt(Str(payload, "prompt"), IsKnownKey) is { } key)
                    activity.PromptKey = key;
                break;
            default:
                activity.LastActivity = now;
                activity.EndedAt = null;
                break;
        }
        return true;
    }

    /// <summary>判斷各 repo 是否活動中，並決定對應的任務（手動指定 → 對話內容 → 分支 → repo 預設）。</summary>
    public void Resolve(DateTime now, int idleMinutes, IReadOnlyDictionary<string, string> sessionTasks)
    {
        var idle = TimeSpan.FromMinutes(Math.Max(1, idleMinutes));
        foreach (var a in _repos.Values)
        {
            a.Active = a.Repo.Enabled && a.EndedAt is null && a.LastActivity != DateTime.MinValue && now - a.LastActivity < idle;

            (string? task, string? source) = (null, null);
            if (a.SessionId is { } sid && sessionTasks.TryGetValue(sid, out var picked))
                (task, source) = (picked, "手動指定");
            else if (a.PromptKey is { } promptKey)
                (task, source) = (promptKey, "對話內容");
            else if (KeyFinder.FindInBranch(a.Branch, IsKnownKey) is { } branchKey)
                (task, source) = (branchKey, $"分支 {a.Branch}");
            else if (KeyFinder.Parse(a.Repo.DefaultTask) is { } defaultKey)
                (task, source) = (defaultKey, "repo 預設");
            a.ResolvedTask = task;
            a.TaskSource = source;
        }
    }

    static string? Str(JsonElement e, string name) =>
        e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
}
