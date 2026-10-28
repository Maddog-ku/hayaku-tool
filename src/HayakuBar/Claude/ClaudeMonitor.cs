using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using HayakuBar.Config;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.Claude;

/// <summary>一個監看單位：主資料夾本身，或主資料夾裡偵測到的某個專案。</summary>
public sealed class RepoActivity(string path)
{
    public string Path { get; } = path;
    public string Name => System.IO.Path.GetFileName(Path.TrimEnd('\\', '/'));

    /// <summary>所屬的主資料夾；本身就是主資料夾時為 null。</summary>
    public string? RootPath { get; set; }

    public bool Enabled { get; set; } = true;

    /// <summary>Claude 改了這裡的程式、但對應不到任務時，詢問要不要建立任務。</summary>
    public bool AutoCreate { get; set; }

    public DateTime LastActivity { get; set; } = DateTime.MinValue;
    public DateTime? EndedAt { get; set; }
    public string? SessionId { get; set; }
    public string? SessionFile { get; set; }
    public long FileOffset { get; set; }
    public string? Branch { get; set; }
    public string? PromptKey { get; set; }

    /// <summary>這個 session 最近一次在本單位裡改檔（Edit、Write…）的時間；只是問問題、規劃不會有。</summary>
    public DateTime? EditedAt { get; set; }

    /// <summary>這個 session 第一句有內容的指令摘要，當新任務名稱的建議。</summary>
    public string? FirstPrompt { get; set; }

    /// <summary>最近一筆紀錄的來源：cli 或 claude-desktop。</summary>
    public string? Entrypoint { get; set; }

    public string SourceText => ClaudeMonitor.SourceText(Entrypoint);

    public bool Active { get; set; }
    public string? ResolvedTask { get; set; }
    public string? TaskSource { get; set; }
}

/// <summary>最近有 Claude Code session 的資料夾（給設定頁快速加入 repo 用）。</summary>
public sealed record DiscoveredProject(string Path, DateTime LastActivity, string Source);

/// <summary>主資料夾裡偵測到的專案。</summary>
public sealed record DetectedProject(string Path, string Name, bool IsGit, DateTime? LastSession, string Source);

/// <summary>
/// 監看設定的主資料夾（與裡面的專案）的 Claude Code 活動。CLI 和桌面版都寫在
/// ~/.claude/projects/&lt;編碼路徑&gt;/*.jsonl（每行都有 cwd、gitBranch、timestamp、entrypoint），hooks 只是讓反應更即時。
/// 資料夾名稱是「開 Claude 時所在路徑」把非英數字元換成 '-'，中文路徑會撞名，所以一律用檔案裡的 cwd 決定歸屬：
/// session 算給「包含它的最深一層」單位，子專案的 session 不會重複算進主資料夾。
/// </summary>
public sealed partial class ClaudeMonitor
{
    const int InitialTailBytes = 512 * 1024;
    const int MaxReadBytes = 1024 * 1024;
    const int HeadBytes = 256 * 1024;
    const int QuickHeadBytes = 32 * 1024;
    static readonly TimeSpan RecentFiles = TimeSpan.FromDays(7);
    static readonly TimeSpan DetectCacheTime = TimeSpan.FromMinutes(2);
    static readonly TimeSpan IndexCacheTime = TimeSpan.FromSeconds(60);
    static readonly HashSet<string> SkipFolders = new(StringComparer.OrdinalIgnoreCase) { "node_modules", "bin", "obj", "dist", "build" };

    /// <summary>會改檔的工具；Bash 不算（多半是跑指令、看東西）。</summary>
    static readonly HashSet<string> EditTools = new(StringComparer.Ordinal) { "Edit", "Write", "MultiEdit", "NotebookEdit" };

    readonly Dictionary<string, RepoActivity> _repos = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>session 檔 → 開頭的 cwd（開 session 時的資料夾）與 entrypoint；確定後不會變，快取起來。</summary>
    readonly Dictionary<string, (string Cwd, string? Entrypoint)> _sessionHeads = new(StringComparer.OrdinalIgnoreCase);

    readonly Dictionary<string, (DateTime At, IReadOnlyList<DetectedProject> Projects)> _detectCache = new(StringComparer.OrdinalIgnoreCase);
    (DateTime At, List<SessionInfo> Sessions)? _index;

    sealed record SessionInfo(string Root, DateTime LastWrite, string? Entrypoint);

    [GeneratedRegex(@"^/(?:mnt/)?([a-zA-Z])(?:/(.*))?$")]
    private static partial Regex PosixDrivePath();

    [GeneratedRegex("\"cwd\":\"((?:[^\"\\\\]|\\\\.)*)\"")]
    private static partial Regex CwdField();

    [GeneratedRegex("\"entrypoint\":\"([^\"]*)\"")]
    private static partial Regex EntrypointField();

    public Func<string, bool> IsKnownKey { get; set; } = _ => true;

    /// <summary>所有監看單位（主資料夾＋偵測到的專案）。</summary>
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

    public static string SourceText(string? entrypoint) => entrypoint switch
    {
        "cli" => "CLI",
        "claude-desktop" => "桌面版",
        null or "" => "",
        var other => other,
    };

    /// <summary>統一成 Windows 路徑；Git Bash 格式（/d/repo/x、/mnt/d/repo/x）也轉成 D:\repo\x。</summary>
    public static string Normalize(string path)
    {
        var p = path.Trim();
        var m = PosixDrivePath().Match(p);
        if (m.Success)
            p = $"{char.ToUpperInvariant(m.Groups[1].Value[0])}:\\{m.Groups[2].Value.Replace('/', '\\')}";
        return Path.GetFullPath(p).TrimEnd('\\', '/');
    }

    public static bool IsUnder(string path, string root) =>
        path.Equals(root, StringComparison.OrdinalIgnoreCase)
        || path.StartsWith(root + "\\", StringComparison.OrdinalIgnoreCase);

    /// <summary>Claude Code 的專案資料夾命名：路徑中非英數字元都換成 '-'（D:\sj\hayaku tool → D--sj-hayaku-tool）。</summary>
    public static string Encode(string path) => Regex.Replace(Normalize(path), "[^A-Za-z0-9]", "-");

    static bool IsGitRepo(string path) =>
        Directory.Exists(Path.Combine(path, ".git")) || File.Exists(Path.Combine(path, ".git"));

    // ───────────── 監看單位 ─────────────

    /// <summary>依設定建立監看單位：每個主資料夾，加上（開啟自動偵測時）裡面的每個專案。</summary>
    public void Sync(IEnumerable<RepoConfig> repos)
    {
        var configs = repos.Where(r => !string.IsNullOrWhiteSpace(r.Path)).ToList();
        var wanted = new Dictionary<string, Action<RepoActivity>>(StringComparer.OrdinalIgnoreCase);

        foreach (var repo in configs)
        {
            var path = Normalize(repo.Path);
            wanted[path] = a =>
            {
                a.RootPath = null;
                a.Enabled = repo.Enabled;
                a.AutoCreate = repo.AutoCreate;
            };
        }
        foreach (var repo in configs.Where(r => r.IncludeSubprojects))
        {
            var root = Normalize(repo.Path);
            foreach (var project in DetectProjects(root))
            {
                if (wanted.ContainsKey(project.Path))
                    continue; // 使用者另外單獨加的資料夾，以它自己的設定為準
                var custom = repo.Projects.FirstOrDefault(p => string.Equals(p.Name, project.Name, StringComparison.OrdinalIgnoreCase));
                wanted[project.Path] = a =>
                {
                    a.RootPath = root;
                    a.Enabled = repo.Enabled && (custom?.Enabled ?? true);
                    a.AutoCreate = custom?.AutoCreate ?? false;
                };
            }
        }

        foreach (var (path, apply) in wanted)
        {
            if (!_repos.TryGetValue(path, out var activity))
                _repos[path] = activity = new RepoActivity(path);
            apply(activity);
        }
        foreach (var key in _repos.Keys.Where(k => !wanted.ContainsKey(k)).ToList())
            _repos.Remove(key);
    }

    /// <summary>
    /// 主資料夾裡的專案：git repo 的子資料夾，以及 Claude 曾在裡面開過 session 的子資料夾。
    /// 主資料夾本身是 git repo 時，只認巢狀的 git repo（一般子資料夾的 session 算主資料夾的）。
    /// </summary>
    public IReadOnlyList<DetectedProject> DetectProjects(string root, bool refresh = false)
    {
        root = Normalize(root);
        if (!refresh && _detectCache.TryGetValue(root, out var cached) && DateTime.Now - cached.At < DetectCacheTime)
            return cached.Projects;

        var found = new Dictionary<string, DetectedProject>(StringComparer.OrdinalIgnoreCase);
        if (!Directory.Exists(root))
            return [];
        var rootIsGit = IsGitRepo(root);
        try
        {
            foreach (var dir in Directory.GetDirectories(root))
            {
                var name = Path.GetFileName(dir);
                if (name.StartsWith('.') || SkipFolders.Contains(name))
                    continue;
                if (IsGitRepo(dir))
                    found[dir] = new DetectedProject(dir, name, true, null, "");
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            Log.Error($"列出 {root} 失敗", ex);
        }

        foreach (var session in SessionIndex(refresh))
        {
            if (!IsUnder(session.Root, root) || session.Root.Length == root.Length)
                continue;
            var name = session.Root[(root.Length + 1)..].Split('\\')[0];
            var child = Path.Combine(root, name);
            if (name.StartsWith('.') || SkipFolders.Contains(name))
                continue;
            if (!found.TryGetValue(child, out var project))
            {
                if (rootIsGit || !Directory.Exists(child))
                    continue;
                project = new DetectedProject(child, name, IsGitRepo(child), null, "");
            }
            if (project.LastSession is null || session.LastWrite > project.LastSession)
                project = project with { LastSession = session.LastWrite, Source = SourceText(session.Entrypoint) };
            found[child] = project;
        }

        var list = found.Values
            .OrderByDescending(p => p.LastSession ?? DateTime.MinValue)
            .ThenBy(p => p.Name, StringComparer.OrdinalIgnoreCase)
            .ToList();
        _detectCache[root] = (DateTime.Now, list);
        return list;
    }

    /// <summary>每個 session 資料夾最新的一個檔案：開在哪、最後寫入時間、CLI 或桌面版。</summary>
    List<SessionInfo> SessionIndex(bool refresh = false)
    {
        if (!refresh && _index is { } index && DateTime.Now - index.At < IndexCacheTime)
            return index.Sessions;
        var sessions = new List<SessionInfo>();
        var root = ProjectsRoot;
        if (Directory.Exists(root))
        {
            foreach (var dir in Directory.GetDirectories(root))
            {
                FileInfo? newest;
                try
                {
                    newest = new DirectoryInfo(dir).EnumerateFiles("*.jsonl").MaxBy(f => f.LastWriteTime);
                }
                catch
                {
                    continue;
                }
                if (newest is not null && Head(newest) is { } head)
                    sessions.Add(new SessionInfo(head.Cwd, newest.LastWriteTime, head.Entrypoint));
            }
        }
        _index = (DateTime.Now, sessions);
        return sessions;
    }

    /// <summary>清暫存：已刪除或超過 7 天沒動的 session 檔的開頭資訊、專案偵測與 session 索引（下次用到再重建）。</summary>
    public void TrimCaches()
    {
        var since = DateTime.Now - RecentFiles;
        foreach (var file in _sessionHeads.Keys.ToList())
        {
            var info = new FileInfo(file);
            if (!info.Exists || info.LastWriteTime < since)
                _sessionHeads.Remove(file);
        }
        _detectCache.Clear();
        _index = null;
    }

    // ───────────── 掃描 session ─────────────

    public void Scan()
    {
        var root = ProjectsRoot;
        if (!Directory.Exists(root) || _repos.Count == 0)
            return;
        string[] dirs;
        try
        {
            dirs = Directory.GetDirectories(root);
        }
        catch (Exception ex)
        {
            Log.Error($"列出 {root} 失敗", ex);
            return;
        }

        var units = _repos.Values.ToList();
        var encoded = units.Select(u => (Unit: u, Name: Encode(u.Path))).ToList();
        var newest = new Dictionary<RepoActivity, FileInfo>();
        var since = DateTime.Now - RecentFiles;

        foreach (var dir in dirs)
        {
            var dirName = Path.GetFileName(dir);
            var candidates = encoded.Where(e => dirName.Equals(e.Name, StringComparison.OrdinalIgnoreCase)
                                                || dirName.StartsWith(e.Name + "-", StringComparison.OrdinalIgnoreCase)).ToList();
            if (candidates.Count == 0)
                continue;
            var exactUnit = candidates.FirstOrDefault(c => dirName.Equals(c.Name, StringComparison.OrdinalIgnoreCase)).Unit;
            try
            {
                foreach (var file in new DirectoryInfo(dir).EnumerateFiles("*.jsonl"))
                {
                    if (file.LastWriteTime < since)
                        continue;
                    // 歸給包含 session 起始資料夾的最深一層單位；剛開的 session 還沒有 cwd 時先看資料夾名稱
                    var owner = Head(file) is { } head
                        ? units.Where(u => IsUnder(head.Cwd, u.Path)).MaxBy(u => u.Path.Length)
                        : exactUnit;
                    if (owner is not null && (!newest.TryGetValue(owner, out var current) || file.LastWriteTime > current.LastWriteTime))
                        newest[owner] = file;
                }
            }
            catch (Exception ex)
            {
                Log.Error($"列出 {dir} 失敗", ex);
            }
        }

        foreach (var (activity, file) in newest)
            Update(activity, file);
    }

    void Update(RepoActivity activity, FileInfo file)
    {
        if (!string.Equals(activity.SessionFile, file.FullName, StringComparison.OrdinalIgnoreCase))
        {
            activity.SessionFile = file.FullName;
            activity.SessionId = Path.GetFileNameWithoutExtension(file.Name);
            activity.PromptKey = null;
            activity.Branch = null;
            activity.EditedAt = null;
            activity.FirstPrompt = ReadFirstPrompt(file.FullName);
            activity.Entrypoint = Head(file)?.Entrypoint;
            activity.FileOffset = Math.Max(0, file.Length - InitialTailBytes);
        }

        var write = file.LastWriteTime;
        if (write > activity.LastActivity && (activity.EndedAt is null || write > activity.EndedAt.Value.AddSeconds(5)))
        {
            activity.LastActivity = write;
            activity.EndedAt = null;
        }

        if (file.Length < activity.FileOffset)
            activity.FileOffset = 0;
        if (file.Length > activity.FileOffset)
            ReadNewLines(activity, file);
    }

    (string Cwd, string? Entrypoint)? Head(FileInfo file)
    {
        if (_sessionHeads.TryGetValue(file.FullName, out var cached))
            return cached;
        var (cwd, entrypoint) = ReadHead(file.FullName);
        if (cwd is null)
            return null;
        _sessionHeads[file.FullName] = (cwd, entrypoint);
        return (cwd, entrypoint);
    }

    /// <summary>讀檔案開頭，取第一筆 cwd 與 entrypoint；cwd 通常在前幾行，先讀 32KB，找不到才讀 256KB。</summary>
    static (string? Cwd, string? Entrypoint) ReadHead(string path)
    {
        var quick = ReadHead(path, QuickHeadBytes);
        return quick.Cwd is not null ? quick : ReadHead(path, HeadBytes);
    }

    static (string? Cwd, string? Entrypoint) ReadHead(string path, int bytes)
    {
        try
        {
            using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            var buffer = new byte[Math.Min(bytes, fs.Length)];
            var read = fs.ReadAtLeast(buffer, buffer.Length, throwOnEndOfStream: false);
            var text = Encoding.UTF8.GetString(buffer, 0, read);
            string? cwd = null;
            var cwdMatch = CwdField().Match(text);
            if (cwdMatch.Success)
            {
                var raw = JsonSerializer.Deserialize<string>($"\"{cwdMatch.Groups[1].Value}\"");
                if (!string.IsNullOrWhiteSpace(raw))
                    cwd = Normalize(raw);
            }
            var ep = EntrypointField().Match(text);
            return (cwd, ep.Success ? ep.Groups[1].Value : null);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException or ArgumentException or NotSupportedException)
        {
            return (null, null);
        }
    }

    /// <summary>讀檔案開頭，取第一句有內容的指令摘要（新任務名稱的建議）。</summary>
    static string? ReadFirstPrompt(string path)
    {
        try
        {
            using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            var buffer = new byte[Math.Min(HeadBytes, fs.Length)];
            var read = fs.ReadAtLeast(buffer, buffer.Length, throwOnEndOfStream: false);
            foreach (var line in Encoding.UTF8.GetString(buffer, 0, read).Split('\n'))
            {
                if (line.Length < 2)
                    continue;
                try
                {
                    using var doc = JsonDocument.Parse(line);
                    if (UserPrompt(doc.RootElement) is { } text && SessionDigest.Summarize(text) is { } summary)
                        return summary;
                }
                catch (JsonException)
                {
                    // 讀到上限時最後一行不完整
                }
            }
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
        }
        return null;
    }

    /// <summary>使用者自己送出的指令文字（排除系統插入的 meta 訊息與子代理）；不是的話回 null。</summary>
    static string? UserPrompt(JsonElement r)
    {
        if (r.ValueKind != JsonValueKind.Object || Str(r, "type") != "user"
            || r.TryGetProperty("isMeta", out var meta) && meta.ValueKind == JsonValueKind.True
            || r.TryGetProperty("isSidechain", out var side) && side.ValueKind == JsonValueKind.True
            || !r.TryGetProperty("message", out var msg) || msg.ValueKind != JsonValueKind.Object
            || !msg.TryGetProperty("content", out var content))
            return null;
        return PromptText(content);
    }

    /// <summary>列出最近有 Claude Code session 的資料夾（CLI 與桌面版都算），排除桌面版的暫存工作區。</summary>
    public static List<DiscoveredProject> Discover(int max = 15)
    {
        var root = ProjectsRoot;
        var found = new Dictionary<string, DiscoveredProject>(StringComparer.OrdinalIgnoreCase);
        if (!Directory.Exists(root))
            return [];
        static bool IsDesktopScratch(string path) =>
            path.Contains("\\scratch-workspaces\\", StringComparison.OrdinalIgnoreCase)
            || path.Contains("\\AppData\\Roaming\\Claude\\", StringComparison.OrdinalIgnoreCase)
            || path.Contains("\\AppData\\Local\\Claude\\", StringComparison.OrdinalIgnoreCase);
        foreach (var dir in Directory.GetDirectories(root))
        {
            FileInfo? newest;
            try
            {
                newest = new DirectoryInfo(dir).EnumerateFiles("*.jsonl").MaxBy(f => f.LastWriteTime);
            }
            catch
            {
                continue;
            }
            if (newest is null)
                continue;
            var (cwd, entrypoint) = ReadHead(newest.FullName);
            if (cwd is null || IsDesktopScratch(cwd) || !Directory.Exists(cwd))
                continue;
            if (!found.TryGetValue(cwd, out var existing) || existing.LastActivity < newest.LastWriteTime)
                found[cwd] = new DiscoveredProject(cwd, newest.LastWriteTime, SourceText(entrypoint));
        }
        return found.Values.OrderByDescending(p => p.LastActivity).Take(max).ToList();
    }

    void ReadNewLines(RepoActivity activity, FileInfo file)
    {
        try
        {
            using var fs = new FileStream(file.FullName, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
            var length = fs.Length - activity.FileOffset;
            if (length > MaxReadBytes) // 落後太多（例如剛開工具）：只看最後一段
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
            if (r.TryGetProperty("entrypoint", out var ep) && ep.ValueKind == JsonValueKind.String && ep.GetString() is { Length: > 0 } entrypoint)
                activity.Entrypoint = entrypoint;
            if (UserPrompt(r) is { } prompt)
            {
                if (KeyFinder.FindInPrompt(prompt, IsKnownKey) is { } key)
                    activity.PromptKey = key;
                activity.FirstPrompt ??= SessionDigest.Summarize(prompt);
            }
            else if (EditTime(r, activity.Path) is { } edited && (activity.EditedAt is null || edited > activity.EditedAt))
                activity.EditedAt = edited;
        }
        catch (JsonException)
        {
            // 從檔案中段開始讀時第一行可能不完整
        }
    }

    /// <summary>Claude 在 root 底下改檔的那一筆紀錄 → 時間；改的是外面的檔案（例如 plan mode 寫的計畫檔）不算。</summary>
    internal static DateTime? EditTime(JsonElement r, string root)
    {
        if (Str(r, "type") != "assistant"
            || !r.TryGetProperty("message", out var msg) || msg.ValueKind != JsonValueKind.Object
            || !msg.TryGetProperty("content", out var content) || content.ValueKind != JsonValueKind.Array)
            return null;
        foreach (var part in content.EnumerateArray())
        {
            if (part.ValueKind != JsonValueKind.Object || Str(part, "type") != "tool_use"
                || Str(part, "name") is not { } name || !EditTools.Contains(name)
                || !part.TryGetProperty("input", out var input) || input.ValueKind != JsonValueKind.Object
                || (Str(input, "file_path") ?? Str(input, "notebook_path")) is not { Length: > 0 } file)
                continue;
            try
            {
                if (!IsUnder(Normalize(file), root))
                    continue;
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException or PathTooLongException)
            {
                continue;
            }
            return Str(r, "timestamp") is { } ts
                   && DateTimeOffset.TryParse(ts, CultureInfo.InvariantCulture, DateTimeStyles.None, out var at)
                ? at.LocalDateTime
                : DateTime.Now;
        }
        return null;
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

    /// <summary>處理 Claude Code hook 送來的事件；對應到監看單位才回傳 true。</summary>
    public bool OnHook(JsonElement payload, DateTime now)
    {
        if (payload.ValueKind != JsonValueKind.Object || Str(payload, "cwd") is not { } cwd)
            return false;
        string normalized;
        try { normalized = Normalize(cwd); }
        catch { return false; }

        var activity = _repos.Values.Where(u => IsUnder(normalized, u.Path)).MaxBy(u => u.Path.Length);
        if (activity is null)
            return false;

        if (Str(payload, "session_id") is { } sid && sid != activity.SessionId)
        {
            activity.SessionId = sid;
            activity.PromptKey = null;
            activity.EditedAt = null;
            activity.FirstPrompt = null;
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
                activity.FirstPrompt ??= SessionDigest.Summarize(Str(payload, "prompt"));
                break;
            default:
                activity.LastActivity = now;
                activity.EndedAt = null;
                break;
        }
        return true;
    }

    /// <summary>判斷各單位是否活動中，並決定對應的任務（手動指定 → 對話內容 → 分支 → 專案／主資料夾預設）。</summary>
    public void Resolve(DateTime now, int idleMinutes, IReadOnlyDictionary<string, string> sessionTasks)
    {
        var idle = TimeSpan.FromMinutes(Math.Max(1, idleMinutes));
        foreach (var a in _repos.Values)
        {
            a.Active = a.Enabled && a.EndedAt is null && a.LastActivity != DateTime.MinValue && now - a.LastActivity < idle;

            (string? task, string? source) = (null, null);
            if (a.SessionId is { } sid && sessionTasks.TryGetValue(sid, out var picked))
                (task, source) = (picked, "手動指定");
            else if (a.PromptKey is { } promptKey)
                (task, source) = (promptKey, "對話內容");
            else if (KeyFinder.FindInBranch(a.Branch, IsKnownKey) is { } branchKey)
                (task, source) = (branchKey, $"分支 {a.Branch}");
            // 沒有預設任務：偵測不到任務編號就不計時
            a.ResolvedTask = task;
            a.TaskSource = source;
        }
    }

    static string? Str(JsonElement e, string name) =>
        e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
}
