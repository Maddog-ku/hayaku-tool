using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>釘選托盤、切換計時、專案清單、待我審核、移動任務與工時。</summary>
public sealed partial class TimerEngine
{
    /// <summary>托盤最多釘幾個（Hayaku 的上限）。</summary>
    public const int MaxPins = 10;

    static readonly TimeSpan ReviewsInterval = TimeSpan.FromMinutes(10);
    DateTime _reviewsFetched = DateTime.MinValue;
    readonly HashSet<string> _projectLoadFailed = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>看得到的專案（含封存的、圖示與我的角色），跟我的任務一起每 30 分鐘更新。</summary>
    public IReadOnlyList<ProjectInfo> Projects { get; private set; } = [];

    /// <summary>釘在托盤上的任務（照順序）；打開面板、工作台時更新。</summary>
    public IReadOnlyList<PinItem> Pins { get; private set; } = [];

    /// <summary>等我審核的變更申請（不含自己送出的）；只有在某個專案是管理員或編輯者時才會去抓。</summary>
    public IReadOnlyList<ChangeRequest> PendingReviews { get; private set; } = [];

    /// <summary>某個專案是管理員或編輯者（才有東西要審核）。</summary>
    public bool CanReview => Projects.Any(p => p.CanReview && !p.IsArchived);

    public bool IsPinned(string key) => Pins.Any(p => string.Equals(p.Key, key, StringComparison.OrdinalIgnoreCase));

    /// <summary>專案名稱或代碼 → 專案（圖示用）。</summary>
    public ProjectInfo? ProjectOf(string nameOrKey) =>
        Projects.FirstOrDefault(p => p.Name == nameOrKey)
        ?? Projects.FirstOrDefault(p => string.Equals(p.Key, nameOrKey, StringComparison.OrdinalIgnoreCase));

    async Task RefreshReviewsCoreAsync(CancellationToken ct)
    {
        _reviewsFetched = DateTime.Now;
        var me = User?.Username;
        var pending = await Try(() => _api.ListChangeRequestsAsync("PENDING", ct), PendingReviews, "待審核的申請");
        PendingReviews = pending.Where(c => c.Requester?.Username != me).ToList();
    }

    /// <summary>審核頁核准、拒絕後呼叫：重抓待審核筆數。</summary>
    public Task RefreshReviewsAsync() => Guarded(RefreshReviewsCoreAsync);

    /// <summary>已經讀過的專案狀態清單（不會打 Hayaku）。</summary>
    public ProjectDetail? TryGetCachedProject(string project) => _projects.GetValueOrDefault(project);

    /// <summary>狀態在專案設定裡的順序（專案還沒讀過就是 0，讀過後重排）。</summary>
    public int StatusRank(string taskKey, string status) =>
        _projects.TryGetValue(KeyFinder.ProjectOf(taskKey), out var p) && p.Statuses.ToList().FindIndex(s => s.Name == status) is var i and >= 0
            ? i : 0;

    /// <summary>先讀這些任務所屬專案的狀態清單（排序用）；有讀到新的回傳 true。</summary>
    public async Task<bool> LoadProjectsOfAsync(IEnumerable<string> taskKeys)
    {
        var loaded = false;
        foreach (var project in taskKeys.Select(KeyFinder.ProjectOf).Distinct(StringComparer.OrdinalIgnoreCase).ToList())
        {
            if (_projects.ContainsKey(project) || !_projectLoadFailed.Add(project))
                continue; // 讀過、或讀失敗過（沒權限）的不再重讀
            try
            {
                await GetProjectAsync(project);
                _projectLoadFailed.Remove(project);
                loaded = true;
            }
            catch (Exception ex)
            {
                Log.Error($"讀取 {project} 的狀態清單失敗", ex);
            }
        }
        return loaded;
    }

    // ───────────── 釘選、切換 ─────────────

    public Task PinTaskAsync(string key) => Guarded(async ct =>
    {
        if (Pins.Count >= MaxPins && !IsPinned(key))
            throw new McpException($"托盤最多釘 {MaxPins} 個任務，先取消一個再釘");
        Pins = await _api.PinTaskAsync(key.ToUpperInvariant(), ct);
        Notify?.Invoke($"已釘選 {key}");
    });

    public Task UnpinTaskAsync(string key) => Guarded(async ct =>
    {
        Pins = await _api.UnpinTaskAsync(key, ct);
    });

    /// <summary>
    /// 切換到這個任務（同網頁托盤的 ▶）：暫停其他所有計時、開始這個；keepOthers 是同時計時。
    /// 被暫停的 Claude 計時跟手動暫停一樣，Claude 還在動也先不重開。
    /// </summary>
    public Task SwitchTaskAsync(string key, bool keepOthers = false) => Guarded(async ct =>
    {
        key = key.Trim().ToUpperInvariant();
        var before = RunningKeys();
        await _api.SwitchTimerAsync(key, keepOthers, ct);
        if (!keepOthers)
        {
            foreach (var other in before.Where(k => !string.Equals(k, key, StringComparison.OrdinalIgnoreCase)))
            {
                _selfPaused.Add(other);
                if (_claudeDesired.Contains(other))
                    _suppressed.Add(other);
                RemoveClaudeOwned(other);
            }
        }
        _suppressed.Remove(key);
        RemoveClaudeOwned(key);
        _knownProjects.Add(KeyFinder.ProjectOf(key));
        await RefreshCoreAsync(ct);
        Pins = await Try(() => _api.ListPinsAsync(ct), Pins, "釘選托盤");
    });

    // ───────────── 移動任務、工時 ─────────────

    /// <summary>換上層（null＝移到最上層），只能在同一個專案裡。</summary>
    public Task MoveTaskAsync(string key, string? parent) => Guarded(async ct =>
    {
        await _api.MoveTaskAsync(key, parent, ct);
        Notify?.Invoke(parent is null ? $"{key} 已移到最上層" : $"{key} 已移到 {parent} 底下");
        InvalidateExtras();
        _tasksFetched = DateTime.MinValue;
        await RefreshCoreAsync(ct);
    });

    /// <summary>
    /// 把一段已結束的工時移到別的任務：在新任務補登同一段（起訖、備註不變），成功後才刪掉原本那段。
    /// 回傳新那段的 id（找不到就是 null），復原時用。MCP 沒有直接改計時段任務的工具，所以這樣做。
    /// </summary>
    public async Task<string?> MoveTimeEntryAsync(TimeEntry entry, string toTask)
    {
        if (entry.Running || entry.StartedAt is not { } start || entry.EndedAt is not { } end)
            throw new McpException("正在計時的那段不能移，先暫停再移");
        toTask = toTask.Trim().ToUpperInvariant();
        if (string.Equals(entry.Task, toTask, StringComparison.OrdinalIgnoreCase))
            throw new McpException($"這段已經是 {toTask} 的工時");
        string? newId = null;
        Exception? error = null;
        await Guarded(async ct =>
        {
            try
            {
                newId = await _api.AddTimeEntryWithIdAsync(toTask, start, end, entry.Note, ct);
                newId ??= await FindEntryIdAsync(toTask, start, end, ct);
                await _api.DeleteTimeEntryAsync(entry.Id, ct);
                Log.Info($"工時 {entry.Task} {start.ToLocalTime():HH:mm}–{end.ToLocalTime():HH:mm} 移到 {toTask}");
            }
            catch (Exception ex)
            {
                error = ex;
                // 新的補上了、舊的刪不掉：把新的刪掉，不要變成兩段
                if (newId is not null)
                {
                    try
                    {
                        await _api.DeleteTimeEntryAsync(newId, ct);
                    }
                    catch (Exception undo)
                    {
                        Log.Error("移動工時失敗後刪除新補登的那段也失敗", undo);
                    }
                }
            }
            await RefreshCoreAsync(ct);
        });
        if (error is not null)
            throw error;
        return newId;
    }

    /// <summary>補登後回應裡沒有 id：從那天的計時段找同一個任務、同樣起訖的。</summary>
    async Task<string?> FindEntryIdAsync(string task, DateTimeOffset start, DateTimeOffset end, CancellationToken ct)
    {
        var day = DateOnly.FromDateTime(start.LocalDateTime);
        var entries = await _api.ListTimeEntriesAsync(day, day, ct);
        return entries.FirstOrDefault(e => string.Equals(e.Task, task, StringComparison.OrdinalIgnoreCase)
                                           && e.StartedAt is { } s && Math.Abs((s - start).TotalSeconds) < 60
                                           && e.EndedAt is { } en && Math.Abs((en - end).TotalSeconds) < 60)?.Id;
    }
}
