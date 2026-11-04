using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>測試結果要人處理的動作：fix（開始修，可填預估工時）、reopen（退回，填說明）、add（在測試任務加入測試結果）。</summary>
public sealed record TestActionRequest(string Action, string Key, string Title);

/// <summary>
/// 測試流程：開始修、送復驗、通過、退回；通知（被退回、進待復驗）跳提示可以直接處理。
/// 都是按了才送請求；我要修、要驗的數量從「我的任務」那一份在本機算，不另外抓。
/// </summary>
public sealed partial class TimerEngine
{
    public const string StatusFixing = "修復中";
    public const string StatusRetest = "待復驗";
    public const string StatusWontFix = "不處理";

    FileReader? _files;

    /// <summary>讀檔案內容（附件、會議檔案、白板、空間檔案、問題回報截圖），有快取與限流。</summary>
    public FileReader Files => _files ??= new FileReader(_api);

    /// <summary>要開小視窗處理測試結果（開始修填預估工時、退回填說明、加入測試結果）。</summary>
    public event Action<TestActionRequest>? TestActionRequested;

    /// <summary>在工作台打開任務詳細。</summary>
    public void OpenTaskInWorkspace(string key) => WorkspaceRequested?.Invoke("tasks", key);

    public void RequestTestAction(string action, string key, string title) =>
        _ = Raise(TestActionRequested, new TestActionRequest(action, key, title));

    /// <summary>指派給我、還沒修好的測試結果（待修復、修復中），面板今天分頁的卡片用。</summary>
    public IReadOnlyList<TaskSummary> MyFixes =>
        TaskList("test").Where(t => t.Kind == TaskKind.TestResult && TaskKinds.Bucket(t.Status, t.StatusCategory) == "fixing").ToList();

    /// <summary>指派給我、還沒測完的測試任務。</summary>
    public IReadOnlyList<TaskSummary> MyTests => TaskList("test").Where(t => t.Kind == TaskKind.Test).ToList();

    /// <summary>我是回報的人（或測試任務的執行者）或專案管理員：可以通過、退回、結案。</summary>
    public bool CanRetest(string taskKey, string? reporter) =>
        reporter is { Length: > 0 } r && User is { } me && (r == me.Username || r == me.DisplayName)
        || Projects.Any(p => string.Equals(p.Key, KeyFinder.ProjectOf(taskKey), StringComparison.OrdinalIgnoreCase)
                             && p.Role is "ADMIN" or "MANAGER" or "OWNER");

    /// <summary>開始修：狀態改成修復中，可以順手填預估工時；startTimer 一起開始計時。</summary>
    public async Task<string> StartFixAsync(string key, double? hours, bool startTimer)
    {
        var fields = new Dictionary<string, object?> { ["status"] = StatusFixing };
        if (hours is { } h)
            fields["estimatedHours"] = h;
        var message = await _api.UpdateTaskAsync(key, fields, CancellationToken.None);
        if (startTimer && !RunningKeys().Contains(key))
            await StartTaskAsync(key);
        Log.Info($"測試結果 {key} 開始修" + (hours is { } est ? $"，預估 {est} 小時" : ""));
        await AfterTestChangeAsync(key);
        return message is { Length: > 0 } ? message : startTimer ? $"{key} 開始修，已開始計時" : $"{key} 開始修";
    }

    /// <summary>專案的測試子狀態（讀過的專案才有，不會打 Hayaku；沒設定子狀態是空的）。</summary>
    public IReadOnlyList<TestSubStatus> CachedSubStatuses(string taskKey) =>
        TryGetCachedProject(KeyFinder.ProjectOf(taskKey))?.TestSubStatuses ?? [];

    /// <summary>
    /// 修好了之後要到哪個狀態：修復中這一段還有下一個子狀態（例 待更新測試機）就到那個，沒有就是待復驗。
    /// 專案還沒讀過時當作沒有子狀態。
    /// </summary>
    public TestSubStatus? FixDoneStep(string taskKey, string status)
    {
        var subs = CachedSubStatuses(taskKey).Where(s => s.Stage == StatusFixing).ToList();
        var i = subs.FindIndex(s => s.Name == status);
        return i >= 0 && i + 1 < subs.Count ? subs[i + 1] : null;
    }

    /// <summary>復驗通過後要到的子狀態：待復驗這一段還有下一個（例 待上正式機）就是那個，沒有（直接完成）是 null。</summary>
    public TestSubStatus? PassStep(string taskKey, string status)
    {
        var subs = CachedSubStatuses(taskKey).Where(s => s.Stage == StatusRetest).ToList();
        var i = subs.FindIndex(s => s.Name == status);
        return i >= 0 && i + 1 < subs.Count ? subs[i + 1] : null;
    }

    /// <summary>這個狀態可以批次推到下一步（例 待更新測試機〔已更新到測試機〕）就回傳那個子狀態。</summary>
    public TestSubStatus? BatchStep(string taskKey, string status) =>
        CachedSubStatuses(taskKey).FirstOrDefault(s => s.Name == status && s.BatchText is not null);

    /// <summary>
    /// 修好了：專案在修復中後面還有子狀態（例 待更新測試機）就改到那個，等更新完測試機再批次送復驗；
    /// 沒有就直接送復驗（待復驗，等回報的人驗）。status：目前的狀態。
    /// </summary>
    public async Task<string> SubmitRetestAsync(string key, string? status = null)
    {
        var next = status is null ? null : FixDoneStep(key, status);
        var target = next?.Name ?? StatusRetest;
        var message = await _api.UpdateTaskAsync(key, new Dictionary<string, object?> { ["status"] = target }, CancellationToken.None);
        if (RunningKeys().Contains(key))
            await PauseTaskAsync(key);
        Log.Info(next is null ? $"測試結果 {key} 送復驗" : $"測試結果 {key} 修好了，改成{target}");
        await AfterTestChangeAsync(key);
        return message is { Length: > 0 } ? message : next is null ? $"{key} 已送復驗" : $"{key} 修好了，改成{target}";
    }

    /// <summary>
    /// 批次推到下一步（例 已更新到測試機：待更新測試機 → 待復驗，通知回報的人復驗）。
    /// tasks 為 null 是專案裡那個子狀態的全部；comment 是更新說明（例版本號）。
    /// </summary>
    public async Task<string> AdvanceTestResultsAsync(string project, TestSubStatus from, IReadOnlyList<string>? tasks, string? comment)
    {
        var message = await _api.AdvanceTestResultsAsync(project, from.Name, tasks, comment, CancellationToken.None);
        Log.Info($"{project} 測試結果批次{from.BatchText}（{(tasks is null ? "全部" : string.Join("、", tasks))}）");
        foreach (var key in tasks ?? [])
        {
            var read = Notifications
                .Where(n => n.Unread && n.IsTestResult && string.Equals(n.TaskKey, key, StringComparison.OrdinalIgnoreCase))
                .Select(n => n.Id)
                .ToList();
            if (read.Count > 0)
                await MarkNotificationsReadAsync(read);
        }
        await AfterExternalChangeAsync();
        return message is { Length: > 0 } ? message
            : tasks is { Count: 1 } ? $"{tasks[0]} {from.BatchText}" : $"{from.Name}的測試結果{from.BatchText}";
    }

    /// <summary>
    /// 復驗：通過（已完成；待復驗後面還有子狀態時是到下一個，例 待上正式機）或退回（待修復，輪數 +1，說明變成一則留言）。
    /// </summary>
    public async Task<string> RetestAsync(string key, bool pass, string? comment)
    {
        var message = await _api.RetestAsync(key, pass, comment, null, null, CancellationToken.None);
        Log.Info($"測試結果 {key} 復驗：{(pass ? "通過" : "退回")}");
        await AfterTestChangeAsync(key);
        return message is { Length: > 0 } ? message : pass ? $"{key} 復驗通過" : $"{key} 已退回，等工程師再修";
    }

    /// <summary>不處理（結案，不修了）：只有回報的人或專案管理員能用。</summary>
    public async Task<string> WontFixAsync(string key)
    {
        var message = await _api.UpdateTaskAsync(key, new Dictionary<string, object?> { ["status"] = StatusWontFix }, CancellationToken.None);
        await AfterTestChangeAsync(key);
        return message is { Length: > 0 } ? message : $"{key} 標為不處理";
    }

    /// <summary>處理完一筆測試結果：它的未讀通知標為已讀，任務清單重抓。</summary>
    async Task AfterTestChangeAsync(string key)
    {
        var read = Notifications
            .Where(n => n.Unread && n.IsTestResult && string.Equals(n.TaskKey, key, StringComparison.OrdinalIgnoreCase))
            .Select(n => n.Id)
            .ToList();
        if (read.Count > 0)
            await MarkNotificationsReadAsync(read);
        await AfterExternalChangeAsync();
    }

    /// <summary>從提示按的動作：結果用提示告知（成功或失敗）。</summary>
    async Task RunFromToastAsync(Func<Task<string>> action, string what)
    {
        try
        {
            Toast?.Invoke(new ToastRequest(what, await action()));
        }
        catch (Exception ex)
        {
            Log.Error($"{what}失敗", ex);
            Toast?.Invoke(new ToastRequest($"{what}失敗", ex.Message));
        }
    }

    /// <summary>
    /// 測試結果的通知提示：被退回 →〔開始修〕；進待復驗 →〔通過〕〔退回〕；其他 →〔查看〕。
    /// 查看是在工作台打開那個測試結果（詳細裡也有同樣的按鈕）。
    /// </summary>
    ToastRequest TestToast(NotificationItem n, TimeSpan? duration, bool sticky)
    {
        var key = n.TaskKey!;
        var title = n.Text;
        var actions = new List<ToastAction>();
        if (n.IsReopened)
            actions.Add(new ToastAction("開始修", () =>
            {
                RequestTestAction("fix", key, title);
                return Task.CompletedTask;
            }, Primary: true));
        else if (n.IsReadyToRetest)
        {
            actions.Add(new ToastAction("通過", () => RunFromToastAsync(() => RetestAsync(key, pass: true, null), "復驗通過"), Primary: true));
            actions.Add(new ToastAction("退回", () =>
            {
                RequestTestAction("reopen", key, title);
                return Task.CompletedTask;
            }));
        }
        actions.Add(new ToastAction("查看", () => OpenNotificationAsync(n), Primary: actions.Count == 0));
        var who = n.Actor is { } actor ? $"{actor}：" : "";
        return new ToastRequest($"{n.Label}・{key}", $"{who}{title}", actions, Sticky: sticky || n.IsReopened || n.IsReadyToRetest,
            Duration: duration, FromBottom: true, Tag: $"n:{n.Id}");
    }
}
