using HayakuBar.Infra;
using HayakuBar.Slack;

namespace HayakuBar.Core;

/// <summary>Slack 上按的按鈕：失敗時丟出例外（由 Slack 那則訊息顯示原因），不跳本機的錯誤提示。</summary>
public sealed partial class TimerEngine
{
    /// <summary>把通知傳到 Slack、處理 Slack 上的按鈕；App 啟動時建立。</summary>
    public SlackBridge? Slack { get; set; }

    /// <summary>要關掉某個標記的提示（在 Slack 上處理掉了）。</summary>
    public event Action<string>? ToastDismissRequested;

    public void DismissToast(string tag) => ToastDismissRequested?.Invoke(tag);

    /// <summary>上次抓待我審核清單的時間；比訊息晚還不在清單裡，就是已經在別處處理了。</summary>
    internal DateTime ReviewsFetchedAt => _reviewsFetched;

    public async Task MarkReadFromSlackAsync(IReadOnlyCollection<string> ids)
    {
        await _api.MarkNotificationsReadAsync(ids, CancellationToken.None);
        Notifications = Notifications.Select(n => ids.Contains(n.Id) ? n with { Unread = false } : n).ToList();
        Raise();
    }

    public async Task ArchiveFromSlackAsync(IReadOnlyCollection<string> ids)
    {
        await _api.ArchiveNotificationsAsync(ids, CancellationToken.None);
        Notifications = Notifications.Where(n => !ids.Contains(n.Id)).ToList();
        Raise();
    }

    /// <summary>核准或駁回；待審清單裡找不到就重抓一次，還是沒有表示已經處理過或被撤回了。</summary>
    public async Task ReviewFromSlackAsync(string id, bool approve, bool watch, string? note)
    {
        var c = PendingReviews.FirstOrDefault(r => r.Id == id);
        if (c is null)
        {
            await RefreshReviewsAsync();
            c = PendingReviews.FirstOrDefault(r => r.Id == id) ?? throw new InvalidOperationException("這筆申請已經處理過或被撤回了");
        }
        await _api.ReviewChangeRequestAsync(c.Id, approve, watch, note, CancellationToken.None,
            !approve && c.Type == "CREATE" ? "delete" : null);
        Log.Info($"已從 Slack {(approve ? "核准" : "駁回")}變更申請 {c.Id}（{c.TypeText} {c.TaskKey} {c.TaskTitle}）");
        await AfterReviewAsync(c);
    }
}
