using HayakuBar.Claude;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>Claude 整理的工作日誌：Markdown 草稿、建議的計時備註（計時段 id → 備註）、用到的指令與 commit 數。</summary>
public sealed record ClaudeDigest(string Markdown, IReadOnlyDictionary<string, string> Notes, int PromptCount, int CommitCount);

/// <summary>我的任務分頁、團隊請假、任務狀態、通知封存／稍後提醒。</summary>
public sealed partial class TimerEngine
{
    /// <summary>my_tasks 的分頁：進行中、逾期、該開始還沒開始、未開始。</summary>
    public static readonly IReadOnlyList<(string Tab, string Label)> TaskTabs =
    [
        ("in_progress", "進行中"),
        ("overdue", "逾期"),
        ("should_start", "該開始"),
        ("not_started", "未開始"),
    ];

    readonly Dictionary<string, ProjectDetail> _projects = new(StringComparer.OrdinalIgnoreCase);

    public IReadOnlyDictionary<string, IReadOnlyList<TaskSummary>> TaskLists { get; private set; } =
        new Dictionary<string, IReadOnlyList<TaskSummary>>();

    public IReadOnlyList<TeamMember> Team { get; private set; } = [];

    /// <summary>直接用 Hayaku API（工作台的查詢與表單用；錯誤由呼叫端處理）。</summary>
    public HayakuApi Api => _api;

    /// <summary>畫面上的分頁：「全部」（各分頁合起來、去掉重複）＋ my_tasks 的分頁。</summary>
    public static readonly IReadOnlyList<(string Tab, string Label)> TaskViewTabs = [(AllTab, "全部"), .. TaskTabs];

    public const string AllTab = "all";

    public IReadOnlyList<TaskSummary> TaskList(string tab) => tab == AllTab
        ? TaskTabs.SelectMany(t => TaskLists.GetValueOrDefault(t.Tab) ?? []).DistinctBy(t => t.Key, StringComparer.OrdinalIgnoreCase).ToList()
        : TaskLists.GetValueOrDefault(tab) ?? [];

    public int OverdueCount => TaskList("overdue").Count;

    /// <summary>任務目前的狀態（從我的任務清單找；不在清單裡回 null）。</summary>
    public string? StatusOf(string key) =>
        TaskLists.Values.SelectMany(l => l).FirstOrDefault(t => string.Equals(t.Key, key, StringComparison.OrdinalIgnoreCase))?.Status
        ?? MyTasks.FirstOrDefault(t => string.Equals(t.Key, key, StringComparison.OrdinalIgnoreCase))?.Status;

    /// <summary>今天到期（預計結束是今天、還沒完成）。</summary>
    public int DueTodayCount
    {
        get
        {
            var today = DateOnly.FromDateTime(DateTime.Today);
            return TaskLists.Values.SelectMany(l => l).Where(t => t.PlannedEnd == today).DistinctBy(t => t.Key).Count();
        }
    }

    public IEnumerable<TeamMember> OnLeaveToday => Team.Where(m => m.OnLeave);

    async Task RefreshWorkCoreAsync(CancellationToken ct)
    {
        var lists = new Dictionary<string, IReadOnlyList<TaskSummary>>();
        foreach (var (tab, _) in TaskTabs)
            lists[tab] = await Try(() => _api.MyTaskListAsync(tab, ct), TaskList(tab), "我的任務");
        TaskLists = lists;
        Team = await Try(() => _api.TeamScheduleAsync(ct), Team, "團隊狀況");
        Pins = await Try(() => _api.ListPinsAsync(ct), Pins, "釘選托盤");
    }

    /// <summary>專案的狀態清單、成員（快取）。</summary>
    public async Task<ProjectDetail> GetProjectAsync(string project)
    {
        if (_projects.TryGetValue(project, out var cached))
            return cached;
        var detail = await _api.GetProjectAsync(project, CancellationToken.None);
        _projects[project] = detail;
        return detail;
    }

    /// <summary>
    /// 停止計時並把任務改成完成類狀態（跟網頁的「停止並完成」一樣）。
    /// cascade：false＝這次不帶動上層、子任務的狀態（任務狀態連動）；null 照專案設定。
    /// </summary>
    public Task CompleteTaskAsync(string key, bool? cascade = null) => Guarded(async ct =>
    {
        var project = await GetProjectAsync(KeyFinder.ProjectOf(key));
        var done = project.Statuses.FirstOrDefault(s => s.Category == "DONE")
                   ?? throw new McpException($"{project.Name} 沒有完成類的狀態");
        if (RunningKeys().Contains(key))
        {
            _selfPaused.Add(key);
            await _api.PauseTimerAsync(key, ct);
            if (_claudeDesired.Contains(key))
                _suppressed.Add(key);
            RemoveClaudeOwned(key);
        }
        var fields = new Dictionary<string, object?> { ["status"] = done.Name };
        if (cascade is { } c)
            fields["cascade"] = c;
        var message = await _api.UpdateTaskAsync(key, fields, ct);
        Notify?.Invoke(message is { Length: > 0 } ? message : $"{key} 已停止並改成「{done.Name}」");
        InvalidateExtras();
        await RefreshCoreAsync(ct);
    });

    public Task SetTaskStatusAsync(string key, string status) => Guarded(async ct =>
    {
        var message = await _api.UpdateTaskAsync(key, new Dictionary<string, object?> { ["status"] = status }, ct);
        Notify?.Invoke(message is { Length: > 0 } ? message : $"{key} 改成「{status}」");
        InvalidateExtras();
        await RefreshExtrasCoreAsync(ct);
    });

    public Task ArchiveNotificationAsync(NotificationItem n) => Guarded(async ct =>
    {
        await _api.ArchiveNotificationsAsync([n.Id], ct);
        Notifications = Notifications.Where(x => x.Id != n.Id).ToList();
    });

    /// <summary>preset：1h、next_workday、next_week。</summary>
    public Task SnoozeNotificationAsync(NotificationItem n, string preset) => Guarded(async ct =>
    {
        await _api.SnoozeNotificationsAsync([n.Id], preset, ct);
        Notifications = Notifications.Where(x => x.Id != n.Id).ToList();
    });

    /// <summary>
    /// 工作台做了修改後呼叫：馬上同步一次，一併更新會議、待辦、提醒、通知、任務（改過資料，不受打開時的節流限制）。
    /// 有別的同步正在跑時等它跑完再同步，清單才會跟著變（不會被略過）。
    /// </summary>
    public Task AfterExternalChangeAsync()
    {
        InvalidateExtras();
        _tasksFetched = DateTime.MinValue;
        return LoggedIn ? RefreshAsync(userAction: true) : Task.CompletedTask;
    }

    // ───────────── Claude 整理的工作日誌 ─────────────

    /// <summary>要開工作台的某一頁（提示上的按鈕用）。</summary>
    public event Action<string, object?>? WorkspaceRequested;

    /// <summary>
    /// 把某天的 Claude Code 對話與 git commit 整理成工作日誌草稿，並替還沒寫備註的計時段產生備註。
    /// 讀檔與 git 在背景執行。
    /// </summary>
    public async Task<ClaudeDigest> BuildDigestAsync(DateOnly day, IReadOnlyList<WorkLogEntry> logEntries, IReadOnlyList<TimeEntry> entries)
    {
        var units = _claude.Repos.ToList();
        var sessionTasks = new Dictionary<string, string>(_state.SessionTasks);
        var isKnown = _claude.IsKnownKey;
        var now = DateTime.Now;
        var (prompts, commits) = await Task.Run(async () =>
        {
            var prompts = SessionDigest.Collect(day, units, sessionTasks, isKnown);
            var folders = prompts.Select(p => p.Folder).Concat(units.Where(u => u.Enabled).Select(u => u.Path));
            return (prompts, await SessionDigest.CommitsAsync(day, folders, prompts, isKnown));
        });
        return new ClaudeDigest(
            SessionDigest.Markdown(day, prompts, commits, logEntries, TitleOf),
            SessionDigest.NotesFor(entries, prompts, commits, now),
            prompts.Count,
            commits.Count);
    }

    /// <summary>一次寫入多段計時的備註，最後同步一次。</summary>
    public Task SetEntryNotesAsync(IReadOnlyDictionary<string, string> notes) => Guarded(async ct =>
    {
        foreach (var (id, note) in notes)
            await _api.UpdateTimeEntryAsync(id, null, null, note, ct);
        await RefreshCoreAsync(ct);
    });

    /// <summary>計時中或我的任務裡找得到的任務名稱。</summary>
    public string? TitleOf(string key) =>
        Entries.LastOrDefault(e => string.Equals(e.Task, key, StringComparison.OrdinalIgnoreCase))?.Title
        ?? TaskLists.Values.SelectMany(l => l).FirstOrDefault(t => string.Equals(t.Key, key, StringComparison.OrdinalIgnoreCase))?.Title
        ?? MyTasks.FirstOrDefault(t => string.Equals(t.Key, key, StringComparison.OrdinalIgnoreCase))?.Title;
}
