using HayakuBar.Infra;

namespace HayakuBar.Slack;

/// <summary>
/// 傳到 Slack 的一則訊息：對應哪些通知或哪筆變更申請、按鈕要做什麼，處理了沒。
/// Kind：notice（一般通知，可以標為已讀、封存）、notices（一次很多則，合成一則）、review（待我審核）、
/// retest（測試結果待復驗）、fix（測試結果被退回）。
/// </summary>
public sealed class SlackPost
{
    /// <summary>這則訊息的代號（n:通知 id、cr:申請 id、ns:時間），按鈕的 value 就是它。</summary>
    public string Key { get; set; } = "";
    public string Kind { get; set; } = "";
    public string Channel { get; set; } = "";
    public string Ts { get; set; } = "";
    public string Title { get; set; } = "";
    public string Body { get; set; } = "";
    public string? Url { get; set; }
    public List<string> NotificationIds { get; set; } = [];
    public string? ChangeRequestId { get; set; }

    /// <summary>駁回新增任務的申請會刪掉那個任務，表單上要先講。</summary>
    public string? RejectWarning { get; set; }

    /// <summary>可以「核准並關注」（刪除任務的申請不行）。</summary>
    public bool CanWatch { get; set; }

    public string? TaskKey { get; set; }

    /// <summary>本機提示的標記（Slack 上處理了就把那則提示關掉）。</summary>
    public string? ToastTag { get; set; }

    public DateTime PostedAt { get; set; }

    /// <summary>處理結果（例「✓ 已核准（10:32，從 Slack）」）；null＝還沒處理，訊息上還有按鈕。</summary>
    public string? Done { get; set; }
}

/// <summary>傳過的訊息存在 %APPDATA%\HayakuBar\slack.json，重開程式後舊訊息的按鈕也能用；超過 7 天的不留。</summary>
public sealed class SlackState
{
    public static readonly TimeSpan Keep = TimeSpan.FromDays(7);

    public List<SlackPost> Posts { get; set; } = [];

    public SlackPost? Find(string key) => Posts.LastOrDefault(p => p.Key == key);

    public void Prune(DateTime now) => Posts.RemoveAll(p => now - p.PostedAt > Keep);

    public static SlackState Load()
    {
        var s = JsonFile.Load<SlackState>(AppPaths.SlackState);
        s.Prune(DateTime.Now);
        return s;
    }

    public void Save() => JsonFile.Save(AppPaths.SlackState, this);
}
