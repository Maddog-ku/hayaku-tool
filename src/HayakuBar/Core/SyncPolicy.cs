namespace HayakuBar.Core;

/// <summary>背景同步什麼時候要打哪些請求（純函式，方便測試）。</summary>
public static class SyncPolicy
{
    /// <summary>今天的計時列表多久一定重抓一次（網頁上改了已結束的計時也會在這段時間內反映）。</summary>
    public static readonly TimeSpan EntriesInterval = TimeSpan.FromMinutes(5);

    /// <summary>連續同步失敗時，下一次自動同步要等多久（1 → 2 → 5 分鐘，之後都是 5 分鐘）。</summary>
    static readonly TimeSpan[] Backoff = [TimeSpan.FromMinutes(1), TimeSpan.FromMinutes(2), TimeSpan.FromMinutes(5)];

    /// <summary>
    /// 要不要重抓今天的計時列表：不是背景輪詢（動作後、使用者要求）、正在計時的跟上次不一樣、
    /// 換日、或距離上次超過 5 分鐘。
    /// </summary>
    public static bool NeedEntries(bool background, IEnumerable<string> runningIds, IEnumerable<string> knownRunningIds,
        DateOnly today, DateOnly fetchedDay, DateTime now, DateTime fetchedAt) =>
        !background
        || !runningIds.ToHashSet().SetEquals(knownRunningIds)
        || fetchedDay != today
        || now - fetchedAt >= EntriesInterval;

    /// <summary>
    /// 背景輪詢這一輪要不要真的同步：面板、工作台都沒開、Claude 也沒在動時，間隔放寬成兩倍
    /// （畫面上的計時時間是本機在跑，不受影響；只是網頁上的變動晚一點反映）。
    /// </summary>
    public static bool ShouldPoll(bool uiVisible, bool claudeActive, DateTime now, DateTime lastPolled, TimeSpan interval) =>
        uiVisible || claudeActive || now - lastPolled >= interval * 2 - TimeSpan.FromSeconds(1);

    /// <summary>第 failures 次失敗（從 1 開始）後要等多久。</summary>
    public static TimeSpan BackoffDelay(int failures) => Backoff[Math.Clamp(failures - 1, 0, Backoff.Length - 1)];
}
