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
public sealed partial class TimerEngine
{
    static readonly TimeSpan EndActionWindow = TimeSpan.FromMinutes(30);

    // 背景同步的頻率：每次輪詢只問「哪些在計時」，其他資料各有自己的間隔（計時列表見 SyncPolicy）
    static readonly TimeSpan ScheduleInterval = TimeSpan.FromMinutes(30);
    static readonly TimeSpan TasksInterval = TimeSpan.FromMinutes(30);

    readonly AppConfig _config;
    readonly AppState _state;
    readonly OAuthClient _auth;
    readonly HayakuApi _api;
    readonly ClaudeMonitor _claude;
    readonly SemaphoreSlim _gate = new(1, 1);
    readonly DispatcherTimer _poll = new();
    static readonly TimeSpan ClaudeActiveInterval = TimeSpan.FromSeconds(15);
    static readonly TimeSpan ClaudeIdleInterval = TimeSpan.FromSeconds(60);
    readonly DispatcherTimer _claudeTimer = new() { Interval = ClaudeActiveInterval };
    readonly DispatcherTimer _hookDebounce = new() { Interval = TimeSpan.FromSeconds(1) };
    readonly HashSet<string> _knownProjects = new(StringComparer.OrdinalIgnoreCase);
    readonly HashSet<string> _suppressed = new(StringComparer.OrdinalIgnoreCase);
    readonly HashSet<string> _selfPaused = new(StringComparer.OrdinalIgnoreCase);
    HashSet<string> _prevRunning = new(StringComparer.OrdinalIgnoreCase);
    HashSet<string> _claudeDesired = new(StringComparer.OrdinalIgnoreCase);
    DateTime _scheduleFetched = DateTime.MinValue;
    DateTime _tasksFetched = DateTime.MinValue;
    DateTime _entriesFetched = DateTime.MinValue;
    DateOnly _entriesDay;
    int _failures;
    DateTime _retryAt = DateTime.MinValue;

    public TimerEngine(AppConfig config, AppState state, OAuthClient auth, HayakuApi api, ClaudeMonitor claude)
    {
        _config = config;
        _state = state;
        _auth = auth;
        _api = api;
        _claude = claude;
        _claude.IsKnownKey = key => _knownProjects.Count == 0 || _knownProjects.Contains(KeyFinder.ProjectOf(key));

        _poll.Interval = TimeSpan.FromSeconds(Math.Max(10, config.PollSeconds));
        _poll.Tick += async (_, _) => await AutoRefreshAsync();
        _claudeTimer.Tick += async (_, _) =>
        {
            CheckAlerts();
            await ClaudeTickAsync();
            await AfterWorkTickAsync();
        };
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
            _initialSyncDone = true;
            Raise();
            return;
        }
        if (UseCachedOffDay(DateOnly.FromDateTime(DateTime.Now)))
        {
            // 今天不用上班（請假、放假）：不問 Hayaku，用上次記下的帳號顯示
            User ??= CachedUser();
            LoggedIn = true;
            _initialSyncDone = true;
            Raise();
            return;
        }
        Raise(); // 讓畫面先顯示「同步中」
        try
        {
            await RefreshAsync(userAction: true);
        }
        finally
        {
            _initialSyncDone = true;
            Raise();
        }
    }

    bool _initialSyncDone;

    /// <summary>有登入資料、第一次同步還沒完成：畫面顯示「同步中，請稍候」。</summary>
    public bool InitialSyncing => !_initialSyncDone && _auth.HasSession;

    /// <summary>正在跟 Hayaku 同步。</summary>
    public bool Syncing { get; private set; }

    /// <summary>上次同步成功的時間。</summary>
    public DateTime? LastSynced { get; private set; }

    public event Action? ConfigApplied;

    public void ApplyConfig()
    {
        _poll.Interval = TimeSpan.FromSeconds(Math.Max(10, _config.PollSeconds));
        _config.Save();
        if (!_config.SyncOnlyDuringWork)
            AutoSyncPaused = false;
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

    public Task RefreshAsync(bool userAction = true) => RoundAsync(background: false, wait: userAction);

    /// <summary>同步一輪：同步 → 依班表自動上下班 → Claude 自動計時。background：背景輪詢，只抓有變化或到期的資料。</summary>
    Task RoundAsync(bool background, bool wait) => Guarded(async ct =>
    {
        await RefreshCoreAsync(ct, background);
        await CheckMeetingLeftAsync(ct);
        await AutomationAsync(ct);
        await ClaudeCoreAsync(ct);
    }, wait);

    /// <summary>
    /// 定時、喚醒、解鎖時的自動同步（鎖定畫面時照常同步）。不打 Hayaku 的情況：
    /// 設定「只在上班時間自動同步」時的下班、休息、放假；連續失敗時退避等待中。
    /// </summary>
    public async Task AutoRefreshAsync()
    {
        if (!_auth.HasSession || LoggingIn)
            return;
        var now = DateTime.Now;
        var paused = !ShouldAutoSync(now);
        if (paused != AutoSyncPaused)
        {
            AutoSyncPaused = paused;
            Log.Info(paused ? "不在上班時間，暫停自動同步" : "恢復自動同步");
            Raise();
        }
        if (paused || now < _retryAt)
            return;

        var before = LastSynced;
        await RoundAsync(background: true, wait: false);
        if (LastSynced != before)
        {
            if (_failures > 0)
                Log.Info("同步恢復正常");
            _failures = 0;
            _retryAt = DateTime.MinValue;
        }
        else if (_syncFailed)
        {
            _failures++;
            var wait = SyncPolicy.BackoffDelay(_failures);
            _retryAt = DateTime.Now + wait;
            Log.Info($"同步失敗 {_failures} 次，{wait.TotalMinutes:0} 分鐘後再自動同步");
        }
    }

    /// <summary>目前不在上班時間，自動同步暫停中（打開面板、按重新整理仍會同步）。</summary>
    public bool AutoSyncPaused { get; private set; }

    bool ShouldAutoSync(DateTime now)
    {
        if (!_config.SyncOnlyDuringWork)
            return true;
        UseCachedOffDay(DateOnly.FromDateTime(now)); // 跨日後：快取知道今天不用上班就不用問 Hayaku
        if (Schedule is null || Schedule.Date != DateOnly.FromDateTime(now))
            return true; // 還不知道今天的班表（跨日後要先讀一次）
        if (Entries.Any(e => e.Running))
            return true; // 有計時在跑：加班中，照常同步
        if (BoundaryDue(now))
            return true;
        return !_state.ClockedOut && WorkMath.Phase(Schedule, now, false).Kind == DayPhaseKind.Working;
    }

    /// <summary>上班、休息、下班的自動動作到時間了、還沒做。</summary>
    bool BoundaryDue(DateTime now) =>
        _config.AutoSchedule && !_state.ClockedOut && Schedule is not null && Schedule.Date == DateOnly.FromDateTime(now)
        && Boundaries(Schedule.Working, now.Date).Any(b => b.At <= now && !_state.HandledBoundaries.Contains(b.Id));

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
        await CheckMeetingLeftAsync(ct); // 在面板上停掉會議的計時：恢復加入會議時暫停的
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
        var running = RunningKeys().OrderBy(k => k).ToList();
        if (running.Count > 0)
            _state.ResumeSet = running;
        _state.ClockedOut = true;
        _state.Save();
        if (running.Count > 0)
            await PauseAllCoreAsync(ct);
        var worked = WorkMath.UnionMinutes(Entries, DateTime.Today, DateTime.Now);
        AfterClockOut(running, worked, Schedule?.ExpectedMinutes ?? 0);
    });

    /// <summary>為 repo 目前的 Claude session 指定任務（只對這個 session 有效，沒有預設任務）。</summary>
    public async Task PickTaskForRepoAsync(string unitPath, string key)
    {
        key = key.ToUpperInvariant();
        var activity = _claude.Repos.FirstOrDefault(r => string.Equals(r.Path, unitPath, StringComparison.OrdinalIgnoreCase));
        if (activity?.SessionId is { } sid)
            await AssignSessionTaskAsync(sid, key);
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

    bool _syncFailed;

    /// <summary>background：背景輪詢（今天的計時列表只在計時有變化或每 5 分鐘才重抓）；否則是動作後或使用者要求的同步，一定重抓。</summary>
    async Task RefreshCoreAsync(CancellationToken ct, bool background = false)
    {
        Syncing = true;
        Raise();
        try
        {
            await SyncAsync(ct, background);
            LastSynced = DateTime.Now;
            _syncFailed = false;
        }
        catch
        {
            _syncFailed = true;
            throw;
        }
        finally
        {
            Syncing = false;
        }
    }

    async Task SyncAsync(CancellationToken ct, bool background)
    {
        var now = DateTime.Now;
        var today = DateOnly.FromDateTime(now);
        if (_state.RollDay(today.ToString("yyyy-MM-dd")))
        {
            _scheduleFetched = DateTime.MinValue;
            _state.Save();
        }

        if (User is null)
        {
            User = await _api.WhoAmIAsync(ct);
            if (_state.UserName != User.Username || _state.UserDisplayName != User.DisplayName)
            {
                (_state.UserName, _state.UserDisplayName) = (User.Username, User.DisplayName);
                _state.Save();
            }
        }
        if (Schedule is null || Schedule.Date != today || now - _scheduleFetched > ScheduleInterval)
        {
            Schedule = await _api.GetScheduleAsync(today, ct);
            _scheduleFetched = now;
        }
        if (now - _tasksFetched > TasksInterval)
            await RefreshTasksAsync(ct);

        // 跨日還在跑的計時不一定出現在今天的列表，所以每次都問；正在計時的跟上次一樣就不用重抓整天的列表
        var running = await _api.RunningTimersAsync(ct);
        if (SyncPolicy.NeedEntries(background, running.Select(e => e.Id), Entries.Where(e => e.Running).Select(e => e.Id),
                today, _entriesDay, now, _entriesFetched))
        {
            var entries = (await _api.ListTimeEntriesAsync(today, today, ct)).ToDictionary(e => e.Id);
            foreach (var e in running)
                entries[e.Id] = e;
            Entries = entries.Values.OrderBy(e => e.StartedAt ?? DateTimeOffset.MinValue).ToList();
            _entriesFetched = now;
            _entriesDay = today;
        }
        foreach (var e in Entries)
            _knownProjects.Add(KeyFinder.ProjectOf(e.Task));

        LoggedIn = true;
        Error = null;
        TrackExternalStops();
        await RefreshExtrasCoreAsync(ct, background);
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
        if (_state.MeetingPause is { } pause)
        {
            // 會議中到了休息、下班：之後恢復的是會議前在做的，不是會議
            _state.ResumeSet = pause.Tasks.Union(RunningKeys().Except(pause.MeetingTasks, StringComparer.OrdinalIgnoreCase),
                StringComparer.OrdinalIgnoreCase).OrderBy(k => k).ToList();
            _state.MeetingPause = null;
            _state.Save();
        }
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
        LearnFolderProjects();
        CheckNewWork(now);

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
            if (running.Contains(key) || _suppressed.Contains(key) || _state.MeetingPause is not null)
                continue; // 會議中先不自動開始，離開會議再說
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
        // 上班中或 Claude 正在動：15 秒掃一次；其他時候 60 秒（hooks 有事件時仍會馬上反應）
        var interval = IsWorking(now) || _claude.Repos.Any(r => r.Active) ? ClaudeActiveInterval : ClaudeIdleInterval;
        if (_claudeTimer.Interval != interval)
            _claudeTimer.Interval = interval;

        // Claude 開始的計時不會因為閒置而暫停，會一直算到休息、下班或手動暫停
        if (changed)
        {
            _state.Save();
            await RefreshCoreAsync(ct);
        }
    }

    /// <summary>
    /// 面板用的上班狀態：有計時在跑，或在班表的上班／休息時段內且今天沒按過下班。
    /// 上班狀態只顯示「下班」按鈕，否則只顯示「上班」。
    /// </summary>
    public bool OnDuty =>
        Entries.Any(e => e.Running)
        || WorkMath.Phase(Schedule, DateTime.Now, _state.ClockedOut).Kind is DayPhaseKind.Working or DayPhaseKind.Break;

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

    /// <summary>定期清暫存：專案狀態清單（順便讓狀態更新）、Claude session 的快取。</summary>
    public void TrimCaches()
    {
        _projects.Clear();
        _claude.TrimCaches();
    }

    void Raise() => Changed?.Invoke();
}
