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

    /// <summary>修好了，送復驗（狀態改成待復驗，等回報的人驗）。</summary>
    public async Task<string> SubmitRetestAsync(string key)
    {
        var message = await _api.UpdateTaskAsync(key, new Dictionary<string, object?> { ["status"] = StatusRetest }, CancellationToken.None);
        if (RunningKeys().Contains(key))
            await PauseTaskAsync(key);
        Log.Info($"測試結果 {key} 送復驗");
        await AfterTestChangeAsync(key);
        return message is { Length: > 0 } ? message : $"{key} 已送復驗";
    }

    /// <summary>復驗：通過（已完成）或退回（待修復，輪數 +1，說明變成一則留言）。</summary>
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
