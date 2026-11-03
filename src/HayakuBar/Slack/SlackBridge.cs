using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.Slack;

/// <summary>
/// 把通知傳到 Slack 私訊、在 Slack 上按的按鈕回來處理。
/// 傳什麼：要動手處理的（待我審核、測試結果被退回或進待復驗）一律傳，其他通知只在螢幕鎖定時傳；電腦上的提示照樣跳。
/// 都用同步已經抓到的資料判斷，不會多跟 Hayaku 要資料。兩邊狀態會跟著變：在 Slack 處理了，電腦上那則提示會關掉；
/// 在 HayakuBar 或網頁處理了，Slack 那則訊息會改成「已在 HayakuBar 或網頁處理」、按鈕拿掉。
/// 全部在 UI 執行緒上跑（Socket Mode 收到的互動先轉回來），狀態不用另外上鎖。
/// </summary>
public sealed class SlackBridge : IDisposable
{
    const string DoneElsewhere = "✓ 已在 HayakuBar 或網頁處理";

    readonly TimerEngine _engine;
    readonly SynchronizationContext _ui;
    readonly SlackClient _client = new();
    readonly SlackState _state = SlackState.Load();
    readonly HashSet<string> _seen = [];
    readonly HashSet<string> _busy = [];
    readonly SemaphoreSlim _io = new(1, 1);
    readonly DateTimeOffset _startedAt = DateTimeOffset.Now;
    SlackCredentials _cred = new();
    SocketModeClient? _socket;
    string? _dm;
    DateTime? _lastSent;
    string? _lastError;

    public SlackBridge(TimerEngine engine)
    {
        _engine = engine;
        _ui = SynchronizationContext.Current ?? new SynchronizationContext();
        engine.Changed += OnChanged;
    }

    /// <summary>螢幕鎖定中（離開座位）：一般通知也傳。</summary>
    public bool Locked { get; set; }

    public bool Configured => !_cred.IsEmpty;

    bool Forwarding => Configured && _engine.Config.SlackForward;

    public event Action? StatusChanged;

    /// <summary>設定頁顯示的狀態。</summary>
    public string StatusText
    {
        get
        {
            if (!Configured)
                return "";
            var parts = new List<string> { _socket?.Problem is { } problem ? problem : "● 已連上，Slack 上的按鈕可以用" };
            if (_lastSent is { } sent)
                parts.Add($"上次傳送 {sent:M/d HH:mm}");
            if (_lastError is { } error)
                parts.Add($"上次傳送失敗：{error}");
            return string.Join(" · ", parts);
        }
    }

    public bool Healthy => Configured && _socket?.Problem is null && _lastError is null;

    /// <summary>讀金鑰、重新連線（設定頁連線或中斷連線後呼叫）。</summary>
    public void Restart()
    {
        _socket?.Dispose();
        _socket = null;
        _dm = null;
        _lastError = null;
        _cred = SlackCredentialStore.Load();
        if (Configured)
        {
            var cred = _cred;
            _socket = new SocketModeClient(ct => _client.OpenSocketUrlAsync(cred.AppToken, ct), OnInteractiveAsync);
            _socket.StatusChanged += () => _ui.Post(_ => RaiseStatus(), null);
            _socket.Start();
        }
        RaiseStatus();
    }

    void RaiseStatus() => StatusChanged?.Invoke();

    // ───────────── 傳出去 ─────────────

    void OnChanged()
    {
        if (!_engine.LoggedIn)
            return;
        // 沒開轉送也照樣記「看過了」，之後打開才不會把之前的一次全傳出去
        var notices = SlackMessages.SelectNotifications(_engine.Notifications, _seen, _startedAt, Locked, _engine.CanReview);
        var reviews = SlackMessages.SelectReviews(_engine.PendingReviews, _seen, _startedAt);
        if (!Configured)
            return;
        ReconcileHandledElsewhere();
        if (!Forwarding)
            return;

        var now = DateTime.Now;
        var posts = reviews
            .Select(c => SlackMessages.FromReview(c, c.TaskKey is { } key ? _engine.TaskUrl(key) : _engine.WebUrl("/notifications"), now))
            .ToList();
        posts.AddRange(notices.Where(SlackMessages.IsActionable).Select(n => SlackMessages.FromNotification(n, _engine.NotificationUrl(n), now)));
        var plain = notices.Where(n => !SlackMessages.IsActionable(n)).ToList();
        if (plain.Count > SlackMessages.MaxSingle)
            posts.Add(SlackMessages.FromNotices(plain, _engine.WebUrl("/notifications"), now));
        else
            posts.AddRange(plain.Select(n => SlackMessages.FromNotification(n, _engine.NotificationUrl(n), now)));
        foreach (var p in posts)
            _ = PostAsync(p);
    }

    async Task PostAsync(SlackPost p)
    {
        await _io.WaitAsync();
        try
        {
            _dm ??= await _client.OpenDmAsync(_cred.BotToken, _cred.UserId, CancellationToken.None);
            p.Channel = _dm;
            p.Ts = await _client.PostMessageAsync(_cred.BotToken, _dm, SlackMessages.Fallback(p), SlackMessages.Blocks(p), CancellationToken.None);
            _state.Posts.Add(p);
            SaveState();
            _lastSent = DateTime.Now;
            _lastError = null;
        }
        catch (Exception ex)
        {
            Log.Error($"傳到 Slack 失敗（{p.Title}）", ex);
            _lastError = ex.Message;
        }
        finally
        {
            _io.Release();
            RaiseStatus();
        }
    }

    /// <summary>在 HayakuBar 或網頁處理掉的（通知已讀、申請不在待審清單了），Slack 那則訊息拿掉按鈕。</summary>
    void ReconcileHandledElsewhere()
    {
        var reviewsFetched = _engine.ReviewsFetchedAt;
        var changed = false;
        foreach (var p in _state.Posts)
        {
            if (p.Done is not null || p.Ts.Length == 0 || _busy.Contains(p.Key))
                continue;
            var handled = p.Kind == "review"
                ? _engine.CanReview && reviewsFetched > p.PostedAt && p.ChangeRequestId is { } id && _engine.PendingReviews.All(c => c.Id != id)
                : p.NotificationIds.Count > 0
                  && p.NotificationIds.All(id => _engine.Notifications.FirstOrDefault(n => n.Id == id) is { Unread: false });
            if (!handled)
                continue;
            p.Done = DoneElsewhere;
            changed = true;
            _ = UpdateAsync(p);
        }
        if (changed)
            SaveState();
    }

    async Task UpdateAsync(SlackPost p, string? error = null)
    {
        await _io.WaitAsync();
        try
        {
            await _client.UpdateMessageAsync(_cred.BotToken, p.Channel, p.Ts, SlackMessages.Fallback(p), SlackMessages.Blocks(p, error),
                CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error($"更新 Slack 訊息失敗（{p.Title}）", ex);
        }
        finally
        {
            _io.Release();
        }
    }

    void SaveState()
    {
        _state.Prune(DateTime.Now);
        _state.Save();
    }

    // ───────────── 按鈕回來 ─────────────

    /// <summary>Socket Mode 的執行緒收到的：轉回 UI 執行緒處理。</summary>
    Task OnInteractiveAsync(JsonElement payload)
    {
        var i = SlackMessages.ParseInteraction(payload);
        _ui.Post(async _ =>
        {
            try
            {
                await HandleAsync(i);
            }
            catch (Exception ex)
            {
                Log.Error("處理 Slack 按鈕失敗", ex);
            }
        }, null);
        return Task.CompletedTask;
    }

    async Task HandleAsync(SlackInteraction i)
    {
        if (!Configured || i.UserId != _cred.UserId)
        {
            Log.Info($"略過 Slack 互動：不是設定的成員（{i.UserId}）");
            return;
        }
        if (i.Type == "block_actions")
        {
            if (i.ActionId is null or SlackMessages.Open || i.Value is null)
                return;
            var p = _state.Find(i.Value);
            if (p is null)
            {
                if (i.Channel is { } channel && i.Ts is { } ts)
                    await _client.UpdateMessageAsync(_cred.BotToken, channel, ts, "這則訊息超過 7 天，請到 HayakuBar 或網頁處理",
                        Array.Empty<object>(), CancellationToken.None);
                return;
            }
            if (p.Done is not null)
            {
                await UpdateAsync(p);
                return;
            }
            if (i.ActionId is SlackMessages.Reject or SlackMessages.Reopen)
            {
                if (i.TriggerId is { } trigger)
                    await _client.OpenViewAsync(_cred.BotToken, trigger, SlackMessages.ReasonModal(p, i.ActionId), CancellationToken.None);
                return;
            }
            await RunAsync(p, i.ActionId, null);
        }
        else if (i.Type == "view_submission" && i.CallbackId == SlackMessages.ReasonCallback
                 && i.Metadata?.Split('|', 2) is [var action, var key] && _state.Find(key) is { Done: null } p)
        {
            await RunAsync(p, action, i.Reason?.Trim());
        }
    }

    async Task RunAsync(SlackPost p, string action, string? reason)
    {
        if (!_busy.Add(p.Key))
            return;
        try
        {
            var result = await DoAsync(p, action, reason);
            p.Done = $"{result}（{DateTime.Now:HH:mm}，從 Slack）";
            Log.Info($"Slack：{p.Title} {result}");
            if (p.ToastTag is { } tag)
                _engine.DismissToast(tag);
            SaveState();
            await UpdateAsync(p);
        }
        catch (Exception ex)
        {
            Log.Error($"Slack：{p.Title} {action} 失敗", ex);
            await UpdateAsync(p, ex.Message);
        }
        finally
        {
            _busy.Remove(p.Key);
        }
    }

    async Task<string> DoAsync(SlackPost p, string action, string? reason)
    {
        string Key() => p.TaskKey ?? throw new InvalidOperationException("這則訊息沒有任務編號");
        string Request() => p.ChangeRequestId ?? throw new InvalidOperationException("這則訊息沒有對應的申請");
        switch (action)
        {
            case SlackMessages.Read:
                await _engine.MarkReadFromSlackAsync(p.NotificationIds);
                return "✓ 已標為已讀";
            case SlackMessages.Archive:
                await _engine.ArchiveFromSlackAsync(p.NotificationIds);
                return "✓ 已封存";
            case SlackMessages.Approve:
                await _engine.ReviewFromSlackAsync(Request(), approve: true, watch: false, null);
                return "✓ 已核准";
            case SlackMessages.ApproveWatch:
                await _engine.ReviewFromSlackAsync(Request(), approve: true, watch: true, null);
                return "✓ 已核准並關注";
            case SlackMessages.Reject:
                await _engine.ReviewFromSlackAsync(Request(), approve: false, watch: false, reason is { Length: > 0 } ? reason : null);
                return reason is { Length: > 0 } ? $"✕ 已駁回：{reason}" : "✕ 已駁回";
            case SlackMessages.Pass:
                await _engine.RetestAsync(Key(), pass: true, null);
                return "✓ 復驗通過";
            case SlackMessages.Reopen:
                if (reason is not { Length: > 0 })
                    throw new InvalidOperationException("退回要填原因");
                await _engine.RetestAsync(Key(), pass: false, reason);
                return $"↩ 已退回：{reason}";
            case SlackMessages.Fix:
                await _engine.StartFixAsync(Key(), null, startTimer: true);
                return "▶ 開始修，已開始計時";
            default:
                throw new InvalidOperationException($"不認得的按鈕：{action}");
        }
    }

    public void Dispose()
    {
        _engine.Changed -= OnChanged;
        _socket?.Dispose();
        _socket = null;
        _client.Dispose();
    }
}
