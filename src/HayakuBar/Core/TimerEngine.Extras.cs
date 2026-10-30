using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>右下角跳出提示上的按鈕。</summary>
public sealed record ToastAction(string Label, Func<Task> Run, bool Primary = false);

/// <summary>右下角跳出提示。</summary>
/// <summary>
/// Duration：停留多久自動收回（null 用預設）；FromBottom：從螢幕下緣滑上來、時間到滑回去（新通知用）；
/// Important：全螢幕、簡報時也照樣跳出（會議快開始）。
/// </summary>
public sealed record ToastRequest(string Title, string Message, IReadOnlyList<ToastAction>? Actions = null, bool Sticky = false,
    TimeSpan? Duration = null, bool FromBottom = false, bool Important = false);

/// <summary>會議、待辦、提醒、通知、工時修改，以及到點跳出的提示。</summary>
public sealed partial class TimerEngine
{
    // 通知要即時提示，2 分鐘；會議、提醒的到點提示用已抓的時間在本機判斷，10 分鐘就夠
    static readonly TimeSpan NotificationsInterval = TimeSpan.FromMinutes(2);
    static readonly TimeSpan MeetingsInterval = TimeSpan.FromMinutes(10);

    /// <summary>只在面板、工作台看得到的資料（待辦、我的任務分頁、團隊）：不在背景抓，打開時才抓，太久沒抓才重抓。</summary>
    static readonly TimeSpan PanelDataInterval = TimeSpan.FromMinutes(2);

    /// <summary>打開面板、工作台時，距離上次完整同步不到這麼久就不重抓。</summary>
    static readonly TimeSpan OpenThrottle = TimeSpan.FromSeconds(20);

    DateTime _notificationsFetched = DateTime.MinValue;
    DateTime _meetingsFetched = DateTime.MinValue;
    DateTime _panelDataFetched = DateTime.MinValue;
    DateTime _openRefreshed = DateTime.MinValue;
    readonly HashSet<string> _alerted = [];
    readonly DateTimeOffset _startedAt = DateTimeOffset.Now;

    public IReadOnlyList<MeetingItem> Meetings { get; private set; } = [];
    public IReadOnlyList<TodoItem> Todos { get; private set; } = [];
    public IReadOnlyList<ReminderItem> Reminders { get; private set; } = [];
    public IReadOnlyList<NotificationItem> Notifications { get; private set; } = [];
    public int UnreadCount => Notifications.Count(n => n.Unread);

    /// <summary>跳出提示（右下角）。</summary>
    public event Action<ToastRequest>? Toast;

    /// <summary>要打開「工時紀錄」視窗（例如下班時還沒滿應出勤）。</summary>
    public event Action<DateOnly>? TimeEntriesRequested;

    // ───────────── 網址 ─────────────

    public string WebUrl(string path) => _config.BaseUrl.TrimEnd('/') + path;

    public string TaskUrl(string key) =>
        WebUrl($"/projects/{Uri.EscapeDataString(KeyFinder.ProjectOf(key))}?task={Uri.EscapeDataString(key)}");

    public string MeetingUrl(string id) => WebUrl($"/meetings/{Uri.EscapeDataString(id)}");

    public string NotificationUrl(NotificationItem n) =>
        n.MeetingId is { } meeting ? MeetingUrl(meeting) : n.TaskKey is { } task ? TaskUrl(task) : WebUrl("/notifications");

    // ───────────── 同步 ─────────────

    /// <summary>
    /// 按重新整理（userAction）或打開面板、工作台時呼叫：同步計時，並馬上更新會議、待辦、提醒、通知、任務。
    /// 打開時距離上次不到 20 秒就略過，連續開關不會一直打 Hayaku。
    /// </summary>
    public Task RefreshAllAsync(bool userAction)
    {
        if (!LoggedIn || (!userAction && (DateTime.Now - _openRefreshed < OpenThrottle || OffToday(DateTime.Now))))
            return Task.CompletedTask; // 不用上班的日子打開面板也不同步，按重新整理才會
        _openRefreshed = DateTime.Now;
        InvalidateExtras(); // 就算這次被略過，下一次同步也會一起更新
        return RefreshAsync(userAction);
    }

    /// <summary>下次同步時重抓會議、提醒、通知、待辦、任務分頁、團隊。</summary>
    void InvalidateExtras()
    {
        _notificationsFetched = DateTime.MinValue;
        _meetingsFetched = DateTime.MinValue;
        _panelDataFetched = DateTime.MinValue;
    }

    /// <summary>background：背景輪詢時不抓只有面板、工作台才看得到的資料。</summary>
    async Task RefreshExtrasCoreAsync(CancellationToken ct, bool background = false)
    {
        var now = DateTime.Now;
        var today = DateOnly.FromDateTime(now);
        if (now - _meetingsFetched >= MeetingsInterval)
        {
            _meetingsFetched = now;
            Meetings = await Try(() => _api.ListMeetingsAsync(today, today, ct, mine: !_config.MeetingAlertsAllProjects), Meetings, "會議");
            Reminders = await Try(() => _api.ListRemindersAsync(ct), Reminders, "提醒");
        }
        if (now - _notificationsFetched >= NotificationsInterval)
        {
            _notificationsFetched = now;
            Notifications = await Try(() => _api.ListNotificationsAsync(ct), Notifications, "通知");
        }
        // 管理員、編輯者才抓待我審核的筆數：每 10 分鐘，或收到「有新的變更申請」通知時
        var newRequest = Notifications.Any(n => n.Type == "CHANGE_REQUESTED" && n.Unread && n.CreatedAt.LocalDateTime > _reviewsFetched);
        if (CanReview && (newRequest || now - _reviewsFetched >= ReviewsInterval))
            await RefreshReviewsCoreAsync(ct);
        if (!background && now - _panelDataFetched >= PanelDataInterval)
        {
            _panelDataFetched = now;
            Todos = await Try(() => _api.ListTodosAsync(ct), Todos, "待辦");
            await RefreshWorkCoreAsync(ct);
        }
        CheckAlerts();
    }

    static async Task<IReadOnlyList<T>> Try<T>(Func<Task<List<T>>> load, IReadOnlyList<T> fallback, string what)
    {
        try
        {
            return await load();
        }
        catch (Exception ex) when (ex is McpException or HttpRequestException or TaskCanceledException)
        {
            Log.Error($"讀取{what}失敗", ex);
            return fallback;
        }
    }

    // ───────────── 動作 ─────────────

    public Task JoinMeetingAsync(MeetingItem meeting) => Guarded(async ct =>
    {
        var before = RunningKeys();
        await _api.JoinMeetingAsync(meeting.Key, ct);
        InvalidateExtras();
        await RefreshCoreAsync(ct);
        var paused = _config.MeetingPausesOthers ? await PauseForMeetingAsync(meeting.Key, before, ct) : [];
        Notify?.Invoke($"已加入會議並開始計時：{meeting.Title}" + (paused.Count > 0 ? $"，暫停 {string.Join("、", paused)}" : ""));
    });

    public Task LeaveMeetingAsync(MeetingItem meeting) => Guarded(async ct =>
    {
        await _api.LeaveMeetingAsync(meeting.Key, ct);
        InvalidateExtras();
        await RefreshCoreAsync(ct);
        if (_state.MeetingPause?.Meeting == meeting.Key)
            await ResumeAfterMeetingAsync("已離開會議", ct);
        else
            Notify?.Invoke("已離開會議，停止計時");
    });

    /// <summary>加入會議前就在跑的計時，會議以外的都暫停，記下來等離開會議後恢復。</summary>
    async Task<List<string>> PauseForMeetingAsync(string meeting, HashSet<string> before, CancellationToken ct)
    {
        var (meetingTasks, others) = SplitForMeeting(meeting, before, RunningKeys());
        var paused = new List<string>();
        foreach (var key in others)
        {
            _selfPaused.Add(key); // 自己暫停的：保留 Claude 標記，恢復後照舊
            try
            {
                await _api.PauseTimerAsync(key, ct);
                paused.Add(key);
            }
            catch (McpException ex)
            {
                Log.Error($"加入會議時暫停 {key} 失敗", ex);
            }
        }
        if (paused.Count == 0)
            return paused;
        var pause = _state.MeetingPause ?? new Config.MeetingPause(); // 會議中又加入另一場：之前暫停的一起留著
        pause.Meeting = meeting;
        pause.MeetingTasks = meetingTasks;
        pause.Tasks = pause.Tasks.Union(paused, StringComparer.OrdinalIgnoreCase).ToList();
        _state.MeetingPause = pause;
        _state.Save();
        Log.Info($"加入會議 {meeting}，暫停 {string.Join("、", paused)}");
        await RefreshCoreAsync(ct);
        return paused;
    }

    /// <summary>
    /// 加入會議後在跑的計時分成「會議的」與「要暫停的」：會議本身是任務（例 REVIVAL-9），
    /// 找不到的話把加入後才出現的當成會議的；加入前就在跑的其他計時要暫停。
    /// </summary>
    internal static (List<string> MeetingTasks, List<string> Others) SplitForMeeting(
        string meeting, IReadOnlySet<string> before, IReadOnlySet<string> after)
    {
        var meetingTasks = after.Contains(meeting)
            ? [after.First(k => string.Equals(k, meeting, StringComparison.OrdinalIgnoreCase))]
            : after.Where(k => !before.Contains(k)).OrderBy(k => k).ToList();
        var others = after.Where(k => before.Contains(k) && !meetingTasks.Contains(k, StringComparer.OrdinalIgnoreCase))
            .OrderBy(k => k).ToList();
        return (meetingTasks, others);
    }

    /// <summary>不是按「離開會議」結束的（網頁上離開、手動停掉會議計時）：同步時發現會議的計時都停了，一樣恢復。</summary>
    async Task CheckMeetingLeftAsync(CancellationToken ct)
    {
        if (_state.MeetingPause is not { MeetingTasks.Count: > 0 } pause)
            return;
        var running = RunningKeys();
        if (pause.MeetingTasks.Any(running.Contains))
            return;
        await ResumeAfterMeetingAsync("會議的計時停了", ct);
    }

    async Task ResumeAfterMeetingAsync(string reason, CancellationToken ct)
    {
        if (_state.MeetingPause is not { } pause)
            return;
        _state.MeetingPause = null;
        _state.Save();
        var running = RunningKeys();
        var started = new List<string>();
        foreach (var key in pause.Tasks.Where(k => !running.Contains(k)))
        {
            try
            {
                await _api.StartTimerAsync(key, ct);
                started.Add(key);
            }
            catch (McpException ex)
            {
                Log.Error($"離開會議後恢復 {key} 失敗", ex);
            }
        }
        if (started.Count == 0)
        {
            if (reason == "已離開會議")
                Notify?.Invoke("已離開會議，停止計時");
            return;
        }
        Notify?.Invoke($"{reason}，恢復計時：{string.Join("、", started)}");
        await RefreshCoreAsync(ct);
    }

    public Task CompleteTodoAsync(TodoItem todo) => Guarded(async ct =>
    {
        await _api.CompleteTodoAsync(todo.Id, ct);
        Todos = Todos.Where(t => t.Id != todo.Id).ToList();
    });

    public Task SnoozeReminderAsync(ReminderItem reminder, int minutes) => Guarded(async ct =>
    {
        await _api.SnoozeReminderAsync(reminder.Id, minutes, ct);
        InvalidateExtras();
        await RefreshExtrasCoreAsync(ct);
    });

    public Task QuickCaptureAsync(string text, bool todo) => Guarded(async ct =>
    {
        await _api.QuickCaptureAsync(text, KeyFinder.Parse(text) is { } key && _knownProjects.Contains(KeyFinder.ProjectOf(key)) ? key : null, todo, ct);
        Notify?.Invoke(todo ? "已存成待辦" : "已存到隨手記");
        if (todo)
        {
            InvalidateExtras();
            await RefreshExtrasCoreAsync(ct);
        }
    });

    public Task MarkNotificationsReadAsync(IReadOnlyCollection<string>? ids) => Guarded(async ct =>
    {
        await _api.MarkNotificationsReadAsync(ids, ct);
        Notifications = Notifications
            .Select(n => ids is null || ids.Contains(n.Id) ? n with { Unread = false } : n)
            .ToList();
    });

    // ───────────── 工時紀錄 ─────────────

    public async Task<(List<TimeEntry> Entries, DaySchedule Schedule)> LoadDayAsync(DateOnly day)
    {
        var entries = await _api.ListDayEntriesAsync(day, CancellationToken.None);
        var schedule = day == Schedule?.Date ? Schedule : await _api.GetScheduleAsync(day, CancellationToken.None);
        return (entries.OrderBy(e => e.StartedAt ?? DateTimeOffset.MaxValue).ToList(), schedule);
    }

    public Task AddTimeEntryAsync(string task, DateTimeOffset start, DateTimeOffset end, string? note) => Guarded(async ct =>
    {
        await _api.AddTimeEntryAsync(task.ToUpperInvariant(), start, end, note, ct);
        await RefreshCoreAsync(ct);
    });

    public Task UpdateTimeEntryAsync(string id, DateTimeOffset? start, DateTimeOffset? end, string? note) => Guarded(async ct =>
    {
        await _api.UpdateTimeEntryAsync(id, start, end, note, ct);
        await RefreshCoreAsync(ct);
    });

    public Task DeleteTimeEntryAsync(string id) => Guarded(async ct =>
    {
        await _api.DeleteTimeEntryAsync(id, ct);
        await RefreshCoreAsync(ct);
    });

    // ───────────── 下班復原 ─────────────

    public Task UndoClockOutAsync(IReadOnlyList<string> tasks) => Guarded(async ct =>
    {
        _state.ClockedOut = false;
        _state.Save();
        foreach (var key in tasks)
        {
            try
            {
                await _api.StartTimerAsync(key, ct);
            }
            catch (McpException ex)
            {
                Log.Error($"復原 {key} 失敗", ex);
            }
        }
        Notify?.Invoke($"已復原，重新開始 {tasks.Count} 個計時");
        await RefreshCoreAsync(ct);
    });

    void AfterClockOut(IReadOnlyList<string> stopped, double worked, int expected)
    {
        var actions = new List<ToastAction>();
        if (stopped.Count > 0)
            actions.Add(new ToastAction("復原", () => UndoClockOutAsync(stopped), Primary: true));
        var unfilled = _config.PromptUnfilled && expected > 0 && worked < expected - 5;
        if (unfilled)
            actions.Add(new ToastAction("補登工時", () =>
            {
                TimeEntriesRequested?.Invoke(DateOnly.FromDateTime(DateTime.Today));
                return Task.CompletedTask;
            }));
        if (_config.ClaudeEnabled && _config.Repos.Count > 0)
            actions.Add(new ToastAction("整理日誌", () =>
            {
                WorkspaceRequested?.Invoke("worklog", "draft");
                return Task.CompletedTask;
            }));
        var message = $"停止 {stopped.Count} 個計時。今天 {WorkMath.HoursMinutes(worked)}" +
                      (expected > 0 ? $"／應出勤 {WorkMath.HoursMinutes(expected)}" : "") +
                      (unfilled ? $"，還差 {WorkMath.HoursMinutes(expected - worked)}" : "");
        Toast?.Invoke(new ToastRequest("已下班", message, actions, Sticky: unfilled));
    }

    // ───────────── 到點提示 ─────────────

    /// <summary>會議快開始、提醒到了、有新通知時跳出提示；每 20 秒檢查一次。</summary>
    public void CheckAlerts()
    {
        if (!LoggedIn)
            return;
        var now = DateTimeOffset.Now;
        CheckReviewAlerts();

        if (_config.MeetingAlerts)
        {
            foreach (var m in Meetings)
            {
                if (m.InMeeting || now < m.StartAt.AddMinutes(-_config.MeetingLeadMinutes) || now > m.StartAt.AddMinutes(10))
                    continue;
                if (!_alerted.Add($"m:{m.Id}:{m.StartAt:O}"))
                    continue;
                Toast?.Invoke(MeetingToast(m, now));
            }
        }

        if (_config.ReminderAlerts)
        {
            foreach (var r in Reminders)
            {
                if (r.At is not { } at || at > now || at < now.AddMinutes(-30) || !_alerted.Add($"r:{r.Id}:{at:O}"))
                    continue;
                var actions = new List<ToastAction> { new("10 分鐘後再提醒", () => SnoozeReminderAsync(r, 10), Primary: true) };
                if (r.Task is { } task)
                    actions.Add(new ToastAction($"開啟 {task}", () => Open(TaskUrl(task))));
                Toast?.Invoke(new ToastRequest("提醒", r.Text, actions, Sticky: true));
            }
        }

        if (_config.NotificationAlerts)
        {
            var fresh = Notifications
                // 「有新的變更申請」由審核提示（可以直接核准、駁回）取代
                .Where(n => n.Unread && n.CreatedAt > _startedAt && !(n.Type == "CHANGE_REQUESTED" && CanReview) && _alerted.Add($"n:{n.Id}"))
                .ToList();
            // 從下面滑上來，停留幾秒（設定可調；滑鼠移上去會停住）後自動滑回去
            var seconds = _config.NotificationPopupSeconds;
            var duration = seconds > 0 ? TimeSpan.FromSeconds(seconds) : (TimeSpan?)null;
            if (fresh.Count > 3)
            {
                var ids = fresh.Select(n => n.Id).ToList();
                Toast?.Invoke(new ToastRequest($"有 {fresh.Count} 則新通知", string.Join("、", fresh.Take(3).Select(n => n.Label)) + "…",
                [
                    new ToastAction("全部標為已讀", () => MarkNotificationsReadAsync(ids), Primary: true),
                    new ToastAction("查看", () => Open(WebUrl("/notifications"))),
                ], Sticky: seconds == 0, Duration: duration, FromBottom: true));
            }
            else
            {
                foreach (var n in fresh)
                    Toast?.Invoke(new ToastRequest(n.Label, (n.Actor is { } actor ? $"{actor}：" : "") + n.Text,
                    [
                        new ToastAction("標為已讀", () => MarkNotificationsReadAsync([n.Id]), Primary: true),
                        new ToastAction("查看", async () =>
                        {
                            await Open(NotificationUrl(n));
                            await MarkNotificationsReadAsync([n.Id]);
                        }),
                    ], Sticky: seconds == 0, Duration: duration, FromBottom: true));
            }
        }
    }

    /// <summary>會議快開始的提示；demo＝本機模擬（--meeting-test），按鈕不會真的加入或開網頁。</summary>
    public ToastRequest MeetingToast(MeetingItem m, DateTimeOffset now, bool demo = false)
    {
        var when = m.StartAt > now ? $"{(int)Math.Ceiling((m.StartAt - now).TotalMinutes)} 分鐘後開始" : "已經開始了";
        Task Demo(string what)
        {
            Notify?.Invoke($"（測試）這是模擬的會議，沒有真的{what}");
            return Task.CompletedTask;
        }
        return new ToastRequest($"會議 {m.StartAt.ToLocalTime():HH:mm}・{when}",
            m.NotAttendee ? $"{m.Title}\n{m.Project}・你不是參加者，加入時會自動加為參加者" : m.Title,
        [
            new ToastAction("加入並計時", () => demo ? Demo("加入") : JoinMeetingAsync(m), Primary: true),
            new ToastAction("開啟", () => demo ? Demo("開啟") : Open(MeetingUrl(m.Id))),
        ], Sticky: true, Important: true);
    }

    static Task Open(string url)
    {
        Web.Open(url);
        return Task.CompletedTask;
    }
}
