using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>審核提示：有新的變更申請等我審核時，從工作列旁跳提示，可以直接核准或駁回。</summary>
public sealed partial class TimerEngine
{
    /// <summary>使用者在審核提示按了「駁回」：開填說明的小視窗。</summary>
    public event Action<ChangeRequest>? RejectRequested;

    /// <summary>
    /// 待我審核的清單裡有新的申請（這次開程式之後送出的）就跳提示；一次超過 3 筆合成一則。
    /// 待審清單每 10 分鐘，或收到「有新的變更申請」通知（約 2 分鐘輪詢一次）時重抓。
    /// </summary>
    void CheckReviewAlerts()
    {
        if (!_config.NotificationAlerts)
            return;
        var fresh = PendingReviews.Where(c => c.CreatedAt > _startedAt && _alerted.Add($"cr:{c.Id}")).ToList();
        if (fresh.Count > 3)
        {
            Toast?.Invoke(new ToastRequest($"有 {fresh.Count} 筆申請等你審核",
                string.Join("、", fresh.Take(3).Select(c => $"{c.TypeText} {c.TaskKey}".Trim())) + "…",
                [new ToastAction("查看", () => OpenReviews(), Primary: true)], Sticky: true, FromBottom: true));
            return;
        }
        foreach (var c in fresh)
            Toast?.Invoke(ReviewToast(c));
    }

    /// <summary>審核提示；demo：截圖、檢查外觀用，按鈕不會真的送出。</summary>
    public ToastRequest ReviewToast(ChangeRequest c, bool demo = false)
    {
        var lines = new List<string> { $"{c.Requester?.DisplayName ?? "有人"}：{c.TaskKey} {c.TaskTitle}".TrimEnd() };
        if (c.Changes.Count > 0)
            lines.Add(string.Join("、", c.Changes.Take(3)) + (c.Changes.Count > 3 ? "…" : ""));
        if (!string.IsNullOrWhiteSpace(c.Reason))
            lines.Add($"理由：{c.Reason}");
        return new ToastRequest($"待審核・{c.TypeText}", string.Join("\n", lines),
        [
            new ToastAction("核准", () => demo ? Demo() : ReviewAsync(c, approve: true, note: null), Primary: true),
            new ToastAction("駁回", () => demo ? Demo() : Raise(RejectRequested, c)),
            new ToastAction("查看", () => OpenReviews()),
        ], Sticky: true, FromBottom: true);

        Task Demo()
        {
            Notify?.Invoke("（測試）這是模擬的申請，沒有真的送出");
            return Task.CompletedTask;
        }
    }

    Task OpenReviews()
    {
        WorkspaceRequested?.Invoke("reviews", null);
        return Task.CompletedTask;
    }

    /// <summary>
    /// 從審核提示核准或駁回（跟審核頁一樣：駁回新增任務的申請會刪掉那個任務）。
    /// 結果用提示告知；成功後把這個任務「有新的變更申請」的通知標為已讀、重抓待審清單。
    /// </summary>
    public async Task ReviewAsync(ChangeRequest c, bool approve, string? note)
    {
        var what = $"{c.TaskKey} {c.TaskTitle}".Trim();
        var verb = approve ? "核准" : "駁回";
        try
        {
            await _api.ReviewChangeRequestAsync(c.Id, approve, watch: false, note, CancellationToken.None,
                !approve && c.Type == "CREATE" ? "delete" : null);
        }
        catch (Exception ex)
        {
            Log.Error($"{verb}變更申請 {c.Id}（{what}）失敗", ex);
            Toast?.Invoke(new ToastRequest($"{verb}失敗", $"{c.TypeText}・{what}：{ex.Message}",
                [new ToastAction("到審核頁看看", () => OpenReviews(), Primary: true)]));
            return;
        }
        Log.Info($"已{verb}變更申請 {c.Id}（{c.TypeText} {what}）");
        Toast?.Invoke(new ToastRequest($"已{verb}", $"{c.TypeText}・{what}"));
        var read = Notifications
            .Where(n => n.Unread && n.Type == "CHANGE_REQUESTED" && c.TaskKey is not null
                        && string.Equals(n.TaskKey, c.TaskKey, StringComparison.OrdinalIgnoreCase))
            .Select(n => n.Id)
            .ToList();
        if (read.Count > 0)
            await MarkNotificationsReadAsync(read);
        await RefreshReviewsAsync();
        _ = AfterExternalChangeAsync();
    }
}
