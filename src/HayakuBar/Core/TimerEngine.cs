using System.Text.Json;
using System.Windows.Threading;
using HayakuBar.Auth;
using HayakuBar.Claude;
using HayakuBar.Config;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>
/// 工具的大腦，全部在 UI 執行緒上跑：
/// 定期同步 Hayaku 計時 → 依班表自動上下班 → 依 Claude 活動自動計時。
/// 所有會打 API 的流程都經過同一個 gate，避免同時開始／暫停同一個任務。
/// </summary>
public sealed class TimerEngine
{
    static readonly TimeSpan EndActionWindow = TimeSpan.FromMinutes(30);

    readonly AppConfig _config;
    readonly AppState _state;
    readonly OAuthClient _auth;
    readonly HayakuApi _api;
    readonly ClaudeMonitor _claude;
    readonly SemaphoreSlim _gate = new(1, 1);
    readonly DispatcherTimer _poll = new();
    readonly DispatcherTimer _claudeTimer = new() { Interval = TimeSpan.FromSeconds(15) };
    readonly DispatcherTimer _hookDebounce = new() { Interval = TimeSpan.FromSeconds(1) };
    readonly HashSet<string> _knownProjects = new(StringComparer.OrdinalIgnoreCase);
    readonly HashSet<string> _suppressed = new(StringComparer.OrdinalIgnoreCase);
    readonly HashSet<string> _selfPaused = new(StringComparer.OrdinalIgnoreCase);
    HashSet<string> _prevRunning = new(StringComparer.OrdinalIgnoreCase);
    HashSet<string> _claudeDesired = new(StringComparer.OrdinalIgnoreCase);
    DateTime _scheduleFetched = DateTime.MinValue;
    DateTime _tasksFetched = DateTime.MinValue;

    public TimerEngine(AppConfig config, AppState state, OAuthClient auth, HayakuApi api, ClaudeMonitor claude)
    {
        _config = config;
        _state = state;
        _auth = auth;
        _api = api;
        _claude = claude;
        _claude.IsKnownKey = key => _knownProjects.Count == 0 || _knownProjects.Contains(KeyFinder.ProjectOf(key));

        _poll.Interval = TimeSpan.FromSeconds(Math.Max(10, config.PollSeconds));
        _poll.Tick += async (_, _) =>
        {
            if (_auth.HasSession && !LoggingIn)
                await RefreshAsync(userAction: false);
        };
        _claudeTimer.Tick += async (_, _) => await ClaudeTickAsync();
        _hookDebounce.Tick += async (_, _) =>
        {
            _hookDebounce.Stop();
            await ClaudeTickAsync();
        };
    }

    public UserInfo? User { get; private set; }
    public DaySchedule? Schedule { get; private set; }
    public IReadOnlyList<TimeEntry> Entries { get; private set; } = [];
    public IReadOnlyList<TaskItem> MyTasks { get; private set; } = [];
    public bool LoggedIn { get; private set; }
    public bool LoggingIn { get; private set; }
    public string? Error { get; private set; }

    public AppConfig Config => _config;
    public AppState State => _state;
    public ClaudeMonitor Claude => _claude;

    public event Action? Changed;
    public event Action<string>? Notify;

    public HashSet<string> RunningKeys() =>
        Entries.Where(e => e.Running).Select(e => e.Task).ToHashSet(StringComparer.OrdinalIgnoreCase);

    public bool IsClaudeOwned(string key) => _state.ClaudeOwned.Contains(key, StringComparer.OrdinalIgnoreCase);

    public async Task StartAsync()
    {
        _poll.Start();
        _claudeTimer.Start();
        if (!_auth.HasSession)
        {
            Raise();
            return;
        }
        await RefreshAsync(userAction: false);
    }

    public event Action? ConfigApplied;

    public void ApplyConfig()
    {
        _poll.Interval = TimeSpan.FromSeconds(Math.Max(10, _config.PollSeconds));
        _config.Save();
        ConfigApplied?.Invoke();
        _ = ClaudeTickAsync();
    }

    // ───────────── 登入 ─────────────

    public async Task LoginAsync()
    {
        if (LoggingIn)
            return;
        LoggingIn = true;
        Error = null;
        Raise();
        try
        {
            await _auth.LoginAsync(CancellationToken.None);
            User = null;
            _scheduleFetched = DateTime.MinValue;
            _tasksFetched = DateTime.MinValue;
            LoggedIn = true;
        }
        catch (Exception ex)
        {
            Log.Error("登入失敗", ex);
            Error = "登入失敗：" + ex.Message;
        }
        finally
        {
            LoggingIn = false;
            Raise();
        }
        if (LoggedIn)
        {
            await RefreshAsync(userAction: true);
            Notify?.Invoke($"已登入 {User?.DisplayName}");
        }
    }

    public async Task LogoutAsync()
    {
        await _auth.LogoutAsync();
        LoggedIn = false;
        User = null;
        Schedule = null;
        Entries = [];
        MyTasks = [];
        Raise();
    }

    // ───────────── 對外動作 ─────────────

    public Task RefreshAsync(bool userAction = true) => Guarded(async ct =>
    {
        await RefreshCoreAsync(ct);
        await AutomationAsync(ct);
        await ClaudeCoreAsync(ct);
    }, wait: userAction);

    public Task StartTaskAsync(string key) => Guarded(async ct =>
    {
        key = key.Trim().ToUpperInvariant();
        await _api.StartTimerAsync(key, ct);
        _suppressed.Remove(key);
        RemoveClaudeOwned(key); // 使用者手動開始的，不再標成 Claude 的
        _knownProjects.Add(KeyFinder.ProjectOf(key));
        await RefreshCoreAsync(ct);
    });

    public Task PauseTaskAsync(string key) => Guarded(async ct =>
    {
        _selfPaused.Add(key);
        await _api.PauseTimerAsync(key, ct);
        if (_claudeDesired.Contains(key))
            _suppressed.Add(key); // Claude 還在動也不要馬上又開始，等它停下來一陣子再說
        RemoveClaudeOwned(key);
        await RefreshCoreAsync(ct);
    });

    /// <summary>可恢復的任務：上次記下的那組；沒有的話用今天被午休自動暫停（BREAK）的。</summary>
    public IReadOnlyList<string> ResumeCandidates()
    {
        if (_state.ResumeSet.Count > 0)
            return _state.ResumeSet;
        return Entries.Where(e => e.StopReason == "BREAK").Select(e => e.Task)
            .Distinct(StringComparer.OrdinalIgnoreCase).ToList();
    }

    public Task ClockInAsync(IReadOnlyCollection<string>? tasks = null) => Guarded(async ct =>
    {
        _state.ClockedOut = false;
        MarkPastBoundariesHandled(DateTime.Now);
        if (tasks is { Count: > 0 })
            _state.ResumeSet = tasks.Select(t => t.ToUpperInvariant()).Distinct().ToList();
        _state.Save();
        await RefreshCoreAsync(ct);
        await ResumeCoreAsync("上班", ct);
    });

    public Task ClockOutAsync() => Guarded(async ct =>
    {
        await RefreshCoreAsync(ct);
        var running = RunningKeys();
        if (running.Count > 0)
            _state.ResumeSet = running.OrderBy(k => k).ToList();
        _state.ClockedOut = true;
        _state.Save();
        if (running.Count > 0)
            await PauseAllCoreAsync(ct);
        var worked = WorkMath.UnionMinutes(Entries, DateTime.Today, DateTime.Now);
        var expected = Schedule?.ExpectedMinutes ?? 0;
        Notify?.Invoke($"已下班，停止 {running.Count} 個計時。今天 {WorkMath.HoursMinutes(worked)}" +
                       (expected > 0 ? $"／應出勤 {WorkMath.HoursMinutes(expected)}" : ""));
    });

    /// <summary>為 repo 目前的 Claude session 指定任務；makeDefault 時同時設成 repo 預設。</summary>
    public async Task PickTaskForRepoAsync(string repoPath, string key, bool makeDefault)
    {
        key = key.ToUpperInvariant();
        var activity = _claude.Repos.FirstOrDefault(r => string.Equals(r.Repo.Path, repoPath, StringComparison.OrdinalIgnoreCase));
        if (activity?.SessionId is { } sid)
        {
            _state.SessionTasks[sid] = key;
            _state.Save();
        }
        if (makeDefault || activity?.SessionId is null)
        {
            var repo = _config.Repos.FirstOrDefault(r => string.Equals(r.Path, repoPath, StringComparison.OrdinalIgnoreCase));
            if (repo is not null)
            {
                repo.DefaultTask = key;
                _config.Save();
            }
        }
        _suppressed.Remove(key);
        await ClaudeTickAsync(wait: true);
    }

    public void OnHook(JsonElement payload)
    {
        if (!_claude.OnHook(payload, DateTime.Now))
            return;
        _hookDebounce.Stop();
        _hookDebounce.Start();
    }

    // ───────────── 內部流程 ─────────────

    Task ClaudeTickAsync(bool wait = false) =>
        LoggedIn ? Guarded(ClaudeCoreAsync, wait) : Task.CompletedTask;

    async Task Guarded(Func<CancellationToken, Task> work, bool wait = true)
    {
        if (!await _gate.WaitAsync(wait ? Timeout.Infinite : 0))
            return;
        try
        {
            await work(CancellationToken.None);
        }
        catch (AuthRequiredException ex)
        {
            LoggedIn = false;
            Error = ex.Message;
        }
        catch (Exception ex)
        {
            Log.Error("執行失敗", ex);
            Error = ex is McpException or HttpRequestException or TaskCanceledException
                ? ex.Message
                : $"發生錯誤：{ex.Message}";
        }
        finally
        {
            _gate.Release();
            Raise();
        }
    }

    async Task RefreshCoreAsync(CancellationToken ct)
    {
        var now = DateTime.Now;
        var today = DateOnly.FromDateTime(now);
        if (_state.RollDay(today.ToString("yyyy-MM-dd")))
        {
            _scheduleFetched = DateTime.MinValue;
            _state.Save();
        }

        User ??= await _api.WhoAmIAsync(ct);
        if (Schedule is null || Schedule.Date != today || now - _scheduleFetched > TimeSpan.FromMinutes(30))
        {
            Schedule = await _api.GetScheduleAsync(today, ct);
            _scheduleFetched = now;
        }
        if (now - _tasksFetched > TimeSpan.FromMinutes(10))
            await RefreshTasksAsync(ct);

        var entries = (await _api.ListTimeEntriesAsync(today, today, ct)).ToDictionary(e => e.Id);
        foreach (var running in await _api.RunningTimersAsync(ct))
            entries[running.Id] = running; // 跨日還在跑的計時不一定出現在今天的列表
        Entries = entries.Values.OrderBy(e => e.StartedAt ?? DateTimeOffset.MinValue).ToList();
        foreach (var e in Entries)
            _knownProjects.Add(KeyFinder.ProjectOf(e.Task));

        LoggedIn = true;
        Error = null;
        TrackExternalStops();
    }

    async Task RefreshTasksAsync(CancellationToken ct)
    {
        try
        {
            foreach (var key in await _api.ProjectKeysAsync(ct))
                _knownProjects.Add(key);
            var tasks = new List<TaskItem>();
            tasks.AddRange(await _api.MyTasksAsync("in_progress", ct));
            tasks.AddRange(await _api.MyTasksAsync("not_started", ct));
            MyTasks = tasks.DistinctBy(t => t.Key).ToList();
            foreach (var t in MyTasks)
                _knownProjects.Add(KeyFinder.ProjectOf(t.Key));
            _tasksFetched = DateTime.Now;
        }
        catch (McpException ex)
        {
            Log.Error("讀取任務清單失敗", ex);
        }
    }

    /// <summary>不是這個工具停掉的計時（網頁上手動、伺服器自動暫停）：不再視為 Claude 的，Claude 也先別重開。</summary>
    void TrackExternalStops()
    {
        var running = RunningKeys();
        foreach (var key in _prevRunning)
        {
            if (running.Contains(key) || _selfPaused.Contains(key))
                continue;
            RemoveClaudeOwned(key);
            if (_claudeDesired.Contains(key))
                _suppressed.Add(key);
        }
        _selfPaused.Clear();
        _prevRunning = running;
    }

    async Task AutomationAsync(CancellationToken ct)
    {
        if (!_config.AutoSchedule || Schedule is null || _state.ClockedOut)
            return;
        var segs = Schedule.Working;
        if (segs.Count == 0)
            return;

        var now = DateTime.Now;
        var day = now.Date;
        var bounds = Boundaries(segs, day);
        var due = bounds.Where(b => b.At <= now && !_state.HandledBoundaries.Contains(b.Id)).ToList();
        if (due.Count > 0)
        {
            // 睡眠或剛開機時可能一次錯過好幾個時間點，只執行最後一個
            foreach (var b in due)
                _state.HandledBoundaries.Add(b.Id);
            _state.Save();

            var last = due[^1];
            if (last.IsStart && now < day + segs[last.Segment].End.ToTimeSpan())
            {
                await ResumeCoreAsync(last.Segment == 0 ? "上班時間到" : "休息結束", ct);
            }
            else if (!last.IsStart && now - last.At < EndActionWindow)
            {
                var running = RunningKeys();
                if (running.Count > 0)
                {
                    await PauseAllCoreAsync(ct);
                    Notify?.Invoke(last.Segment == segs.Count - 1
                        ? $"下班時間到，已停止 {running.Count} 個計時"
                        : $"休息時間，已暫停 {running.Count} 個計時，{segs[last.Segment + 1].Start:HH:mm} 會自動恢復");
                }
            }
        }

        // 上班時段內持續記下「正在做哪些」（含 Claude 自動開的），休息／下班後就恢復這組
        var inSegment = segs.Any(s => day + s.Start.ToTimeSpan() <= now && now < day + s.End.ToTimeSpan() - TimeSpan.FromMinutes(1));
        if (inSegment)
        {
            var snapshot = RunningKeys().OrderBy(k => k).ToList();
            if (!snapshot.SequenceEqual(_state.ResumeSet))
            {
                _state.ResumeSet = snapshot;
                _state.Save();
            }
        }
    }

    async Task ResumeCoreAsync(string reason, CancellationToken ct)
    {
        var running = RunningKeys();
        var toStart = ResumeCandidates().Where(k => !running.Contains(k)).ToList();
        if (toStart.Count == 0)
        {
            if (running.Count == 0)
                Notify?.Invoke($"{reason}，還沒有要計時的任務，點工作列小條選一個");
            return;
        }
        var started = new List<string>();
        foreach (var key in toStart)
        {
            try
            {
                await _api.StartTimerAsync(key, ct);
                started.Add(key);
            }
            catch (McpException ex)
            {
                Log.Error($"開始 {key} 失敗", ex);
            }
        }
        if (started.Count > 0)
            Notify?.Invoke($"{reason}，已開始計時：{string.Join("、", started)}");
        await RefreshCoreAsync(ct);
    }

    async Task PauseAllCoreAsync(CancellationToken ct)
    {
        _selfPaused.UnionWith(RunningKeys());
        await _api.PauseTimerAsync(null, ct);
        await RefreshCoreAsync(ct);
    }

    async Task ClaudeCoreAsync(CancellationToken ct)
    {
        var now = DateTime.Now;
        _claude.Sync(_config.Repos);
        _claude.Scan();
        _claude.Resolve(now, _config.ClaudeIdleMinutes, _state.SessionTasks);

        var desired = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        if (_config.ClaudeEnabled && LoggedIn && (!_config.ClaudeOnlyDuringWork || IsWorking(now)))
            foreach (var repo in _claude.Repos)
                if (repo.Active && repo.ResolvedTask is { } task)
                    desired.Add(task);
        _claudeDesired = desired;
        _suppressed.RemoveWhere(k => !desired.Contains(k));

        var running = RunningKeys();
        var changed = false;
        foreach (var key in desired)
        {
            if (running.Contains(key) || _suppressed.Contains(key))
                continue;
            try
            {
                await _api.StartTimerAsync(key, ct);
                if (!IsClaudeOwned(key))
                    _state.ClaudeOwned.Add(key);
                Log.Info($"Claude 活動，開始計時 {key}");
                changed = true;
            }
            catch (McpException ex)
            {
                Log.Error($"Claude 開始計時 {key} 失敗", ex);
                _suppressed.Add(key);
                Error = $"Claude 無法開始 {key}：{ex.Message}";
            }
        }
        // Claude 開始的計時不會因為閒置而暫停，會一直算到休息、下班或手動暫停
        if (changed)
        {
            _state.Save();
            await RefreshCoreAsync(ct);
        }
    }

    /// <summary>「上班中」：在班表時段內（且今天沒手動下班），或有計時在跑（加班）。</summary>
    bool IsWorking(DateTime now)
    {
        if (Entries.Any(e => e.Running))
            return true;
        return !_state.ClockedOut && WorkMath.Phase(Schedule, now, false).Kind == DayPhaseKind.Working;
    }

    void MarkPastBoundariesHandled(DateTime now)
    {
        if (Schedule is null)
            return;
        foreach (var b in Boundaries(Schedule.Working, now.Date))
            if (b.At <= now && !_state.HandledBoundaries.Contains(b.Id))
                _state.HandledBoundaries.Add(b.Id);
    }

    static List<(DateTime At, bool IsStart, int Segment, string Id)> Boundaries(IReadOnlyList<WorkSegment> segs, DateTime day)
    {
        var list = new List<(DateTime At, bool IsStart, int Segment, string Id)>();
        for (var i = 0; i < segs.Count; i++)
        {
            list.Add((day + segs[i].Start.ToTimeSpan(), true, i, $"S{i}"));
            list.Add((day + segs[i].End.ToTimeSpan(), false, i, $"E{i}"));
        }
        list.Sort((a, b) => a.At.CompareTo(b.At));
        return list;
    }

    void RemoveClaudeOwned(string key)
    {
        if (_state.ClaudeOwned.RemoveAll(k => string.Equals(k, key, StringComparison.OrdinalIgnoreCase)) > 0)
            _state.Save();
    }

    void Raise() => Changed?.Invoke();
}
