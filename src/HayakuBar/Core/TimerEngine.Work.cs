using HayakuBar.Mcp;

namespace HayakuBar.Core;

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

    public IReadOnlyList<TaskSummary> TaskList(string tab) => TaskLists.GetValueOrDefault(tab) ?? [];

    public int OverdueCount => TaskList("overdue").Count;

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

    /// <summary>停止計時並把任務改成完成類狀態（跟網頁的「停止並完成」一樣）。</summary>
    public Task CompleteTaskAsync(string key) => Guarded(async ct =>
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
        var message = await _api.UpdateTaskAsync(key, new Dictionary<string, object?> { ["status"] = done.Name }, ct);
        Notify?.Invoke(message is { Length: > 0 } ? message : $"{key} 已停止並改成「{done.Name}」");
        _extrasFetched = DateTime.MinValue;
        await RefreshCoreAsync(ct);
    });

    public Task SetTaskStatusAsync(string key, string status) => Guarded(async ct =>
    {
        var message = await _api.UpdateTaskAsync(key, new Dictionary<string, object?> { ["status"] = status }, ct);
        Notify?.Invoke(message is { Length: > 0 } ? message : $"{key} 改成「{status}」");
        _extrasFetched = DateTime.MinValue;
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

    /// <summary>工作台做了修改後呼叫：下次同步時一併更新會議、待辦、提醒、通知、任務。</summary>
    public Task AfterExternalChangeAsync()
    {
        _extrasFetched = DateTime.MinValue;
        _tasksFetched = DateTime.MinValue;
        return RefreshAllAsync(userAction: false);
    }

    /// <summary>計時中或我的任務裡找得到的任務名稱。</summary>
    public string? TitleOf(string key) =>
        Entries.LastOrDefault(e => string.Equals(e.Task, key, StringComparison.OrdinalIgnoreCase))?.Title
        ?? TaskLists.Values.SelectMany(l => l).FirstOrDefault(t => string.Equals(t.Key, key, StringComparison.OrdinalIgnoreCase))?.Title
        ?? MyTasks.FirstOrDefault(t => string.Equals(t.Key, key, StringComparison.OrdinalIgnoreCase))?.Title;
}
