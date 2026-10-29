using HayakuBar.Eip;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>
/// EIP 請假同步：每天下班時讀 EIP 的假單（待審＋核准），把跟「今天到下一個上班日」重疊的請假整張登記到 Hayaku；
/// 由同步建立的請假，EIP 那邊駁回、抽單、改時間時取消（手動登記的不動）。
/// </summary>
/// <summary>EIP 假單加上查詢時的狀態（待審、核准、駁回）。</summary>
public sealed record EipLeaveRecord(EipLeave Leave, string State);

public sealed partial class TimerEngine
{
    /// <summary>自動同步失敗（連不上 EIP、Hayaku）後多久再試。</summary>
    static readonly TimeSpan EipRetryDelay = TimeSpan.FromMinutes(30);

    /// <summary>找下一個上班日、算整天與否時要看幾天的班表（get_my_schedule 一次最多 31 天）。</summary>
    const int EipScheduleDays = 30;

    readonly EipSyncState _eipState = EipSyncState.Load();
    EipClient? _eip;

    /// <summary>同一時間只有一個流程用 EIP（登入、換頁共用同一個 session）。</summary>
    readonly SemaphoreSlim _eipGate = new(1, 1);

    /// <summary>「EIP 請假紀錄」頁讀過的年份（年 → 假單），切換月份不重讀。</summary>
    readonly Dictionary<int, List<EipLeaveRecord>> _eipYears = [];
    DateTime _eipRetryAt = DateTime.MinValue;

    public EipSyncState EipSync => _eipState;
    public bool EipSyncing { get; private set; }

    /// <summary>
    /// Claude 計時器每次 tick 都會問：有上班的日子下班了、今天還沒做過，就做一次下班後的工作——
    /// 同步 EIP 請假（有設定時），並記下接下來不用上班的日子。不用上班的日子什麼都不做。
    /// </summary>
    async Task AfterWorkTickAsync()
    {
        var now = DateTime.Now;
        if (!LoggedIn || EipSyncing || now < _eipRetryAt || Schedule is null || Schedule.Date != DateOnly.FromDateTime(now))
            return;
        if (_config.EipLeaveSync && File.Exists(AppPaths.EipAuth))
        {
            if (LeavePlanner.IsDue(Schedule, _state.ClockedOut, now, _eipState.LastSyncDay))
                await SyncEipLeavesAsync(manual: false); // 會順便更新不用上班的日子
            return;
        }
        if (LeavePlanner.IsDue(Schedule, _state.ClockedOut, now, _state.OffDaysUpdated))
            await RefreshOffDaysAsync();
    }

    /// <summary>沒開 EIP 同步時，下班時只讀接下來的班表，記下不用上班的日子。</summary>
    async Task RefreshOffDaysAsync()
    {
        var today = DateOnly.FromDateTime(DateTime.Now);
        EipSyncing = true;
        try
        {
            UpdateOffDays(await _api.GetScheduleRangeAsync(today, today.AddDays(EipScheduleDays), CancellationToken.None), today);
        }
        catch (Exception ex)
        {
            Log.Error("讀取接下來的班表失敗", ex);
            _eipRetryAt = DateTime.Now + EipRetryDelay;
        }
        finally
        {
            EipSyncing = false;
        }
    }

    void UpdateOffDays(IEnumerable<DaySchedule> days, DateOnly today)
    {
        _state.OffDays = OffDayCache.From(days, today);
        _state.OffDaysUpdated = CoveredDay(DateTime.Now);
        _state.Save();
        Log.Info($"接下來不用上班的日子：{(_state.OffDays.Count == 0 ? "無" : string.Join("、", _state.OffDays.Keys.Take(12)))}");
    }

    /// <summary>今天是記下的不用上班日、又還沒讀到今天的班表：直接當作今天的班表，不問 Hayaku。回傳有沒有套用。</summary>
    bool UseCachedOffDay(DateOnly today)
    {
        if (!_config.SyncOnlyDuringWork || Schedule?.Date == today || OffDayCache.Today(_state.OffDays, today) is not { } off)
            return false;
        Schedule = off;
        _scheduleFetched = DateTime.MinValue; // 按重新整理時會重抓真正的班表
        Log.Info($"今天{(off.OnLeave ? "請假" : off.Holiday is { } h ? $"放假（{h}）" : "不上班")}，不自動同步");
        return true;
    }

    /// <summary>今天不用上班（請假、放假、週末）、也沒有計時在跑：不自動送任何請求。</summary>
    bool OffToday(DateTime now) =>
        _config.SyncOnlyDuringWork && Schedule is { } s && s.Date == DateOnly.FromDateTime(now) && s.Working.Count == 0
        && !Entries.Any(e => e.Running);

    /// <summary>這次下班後的工作算哪一天做完的（yyyy-MM-dd）。</summary>
    string CoveredDay(DateTime now) =>
        (Schedule is { } s && s.Date == DateOnly.FromDateTime(now)
            ? LeavePlanner.CoveredDay(s, _state.ClockedOut, now)
            : DateOnly.FromDateTime(now)).ToString("yyyy-MM-dd");

    UserInfo? CachedUser() =>
        _state.UserName is { Length: > 0 } name ? new UserInfo(name, _state.UserDisplayName ?? name) : null;

    /// <summary>同步一次，回傳結果說明（設定頁的「立即同步」也用這個）。</summary>
    public async Task<string> SyncEipLeavesAsync(bool manual)
    {
        if (EipSyncing)
            return "同步中…";
        var cred = EipCredentialStore.Load();
        if (cred.IsEmpty)
            return "尚未設定 EIP 帳密";
        if (!LoggedIn)
            return "還沒登入 Hayaku";

        EipSyncing = true;
        Raise();
        var now = DateTime.Now;
        string result;
        try
        {
            var (added, cancelled, skipped) = await SyncEipCoreAsync(cred, now, CancellationToken.None);
            result = Describe(added, cancelled, skipped);
            if (!manual) // 手動同步不算：下班時照樣再讀一次（白天之後才請的假）
                _eipState.LastSyncDay = CoveredDay(now);
            _eipState.LastSyncAt = now;
            _eipState.LastResult = result;
            _eipState.Save();
            _eipRetryAt = DateTime.MinValue;
            Log.Info($"EIP 請假同步：{result}");
            if (added.Count + cancelled.Count > 0)
            {
                _scheduleFetched = DateTime.MinValue; // 班表的「請假」要跟著更新
                Toast?.Invoke(new ToastRequest("已同步 EIP 請假", result,
                    [new ToastAction("查看請假", () => { WorkspaceRequested?.Invoke("leaves", null); return Task.CompletedTask; })]));
            }
        }
        catch (EipLoginException ex)
        {
            // 帳密錯誤：今天不再自動重試，等使用者到設定更新
            result = $"{ex.Message}，請到設定更新";
            if (!manual)
                _eipState.LastSyncDay = CoveredDay(now);
            _eipState.LastResult = $"失敗：{result}";
            _eipState.Save();
            Log.Info($"EIP 請假同步失敗：{ex.Message}");
            if (!manual)
                Toast?.Invoke(new ToastRequest("EIP 請假同步失敗", result));
        }
        catch (Exception ex)
        {
            result = ex is HttpRequestException or TaskCanceledException ? $"連不上 EIP：{ex.Message}" : ex.Message;
            _eipState.LastResult = $"失敗：{result}";
            _eipState.Save();
            _eipRetryAt = now + EipRetryDelay;
            Log.Error("EIP 請假同步失敗", ex);
        }
        finally
        {
            EipSyncing = false;
            Raise();
        }
        return result;
    }

    static string Describe(List<string> added, List<string> cancelled, List<string> skipped)
    {
        var parts = new List<string>();
        if (added.Count > 0)
            parts.Add("登記 " + string.Join("、", added));
        if (cancelled.Count > 0)
            parts.Add("取消 " + string.Join("、", cancelled));
        if (skipped.Count > 0)
            parts.Add("Hayaku 已經有請假、略過 " + string.Join("、", skipped));
        return parts.Count > 0 ? string.Join("；", parts) : "沒有要同步的請假";
    }

    async Task<(List<string> Added, List<string> Cancelled, List<string> Skipped)> SyncEipCoreAsync(
        EipCredentials cred, DateTime now, CancellationToken ct)
    {
        var today = DateOnly.FromDateTime(now);
        var days = await _api.GetScheduleRangeAsync(today, today.AddDays(EipScheduleDays), ct);
        UpdateOffDays(days, today); // EIP 連不上也先記下來
        var next = LeavePlanner.NextWorkday(days, today) ?? today.AddDays(1);
        var windowFrom = today.ToDateTime(TimeOnly.MinValue);
        var windowTo = next.AddDays(1).ToDateTime(TimeOnly.MinValue);

        // EIP：待審＋核准；年份涵蓋這次的範圍和之前同步過、還沒結束的假單
        var years = _eipState.Leaves.Select(l => l.Start.Year).Append(today.Year).Append(next.Year).ToHashSet();
        var rows = new List<EipLeave>();
        await _eipGate.WaitAsync(ct);
        try
        {
            _eip ??= new EipClient();
            foreach (var year in years.Order())
            {
                rows.AddRange(await _eip.ListLeavesAsync(year, EipClient.StatePending, cred, ct));
                rows.AddRange(await _eip.ListLeavesAsync(year, EipClient.StateApproved, cred, ct));
            }
        }
        finally
        {
            _eipGate.Release();
        }
        var leaves = LeavePlanner.Effective(rows);
        var byNo = leaves.ToDictionary(l => l.No);

        var added = new List<string>();
        var cancelled = new List<string>();
        var skipped = new List<string>();
        var dirty = false;

        // 1. 同步建立的：已經結束的不用再管；EIP 駁回、抽單、改了時間的取消
        foreach (var rec in _eipState.Leaves.ToList())
        {
            if (rec.End <= now)
            {
                _eipState.Leaves.Remove(rec);
                dirty = true;
                continue;
            }
            if (byNo.TryGetValue(rec.No, out var l) && l.Start == rec.Start && l.End == rec.End)
                continue;
            await CancelSyncedAsync(rec, ct);
            _eipState.Leaves.Remove(rec);
            dirty = true;
            if (l is null)
                cancelled.Add(rec.Summary); // 改時間的會在下面重新登記，不用列成取消
        }

        // 2. 跟「今天到下一個上班日」重疊、還沒登記的假單：整張登記
        var targets = leaves
            .Where(l => l.End > windowFrom && l.Start < windowTo && l.End > now)
            .Where(l => _eipState.Leaves.All(r => r.No != l.No))
            .ToList();
        if (targets.Count > 0)
        {
            var from = DateOnly.FromDateTime(targets.Min(l => l.Start));
            var to = DateOnly.FromDateTime(targets.Max(l => l.End));
            if (to > today.AddDays(EipScheduleDays))
                days = days.Concat(await _api.GetScheduleRangeAsync(today.AddDays(EipScheduleDays + 1),
                    Min(to, today.AddDays(EipScheduleDays * 2)), ct)).ToList();
            var working = LeavePlanner.Working(days);
            var mine = await _api.ListLeavesAsync(from, to, ct, "me");

            foreach (var leave in targets)
            {
                var pieces = LeavePlanner.Plan(leave, working);
                if (pieces.Count == 0)
                    continue;
                var summary = $"{string.Join("、", pieces.Select(p => p.Describe()))} {leave.Type}";
                // Hayaku 那段時間已經有請假（手動登記過、或別人幫忙登記）：不重複登記
                if (pieces.Any(p => mine.Any(m => LeavePlanner.Overlaps(p.From, p.To, m.StartAt.LocalDateTime, m.EndAt.LocalDateTime))))
                {
                    skipped.Add(summary);
                    continue;
                }
                var note = $"EIP {leave.No} {leave.Type}" + (leave.Reason.Length > 0 ? $"：{leave.Reason}" : "");
                // 先記下來再登記：中途失敗時已經登記的也找得到（之後抽單才取消得到）；
                // 找不到 id 也照樣記，避免下次重複登記（只是之後 EIP 抽單時沒辦法自動取消）
                var rec = new SyncedLeave { No = leave.No, Type = leave.Type, Start = leave.Start, End = leave.End, Summary = summary };
                _eipState.Leaves.Add(rec);
                dirty = true;
                try
                {
                    foreach (var piece in pieces)
                    {
                        var id = await _api.AddLeaveAsync(piece.Args(note), ct);
                        id ??= await FindLeaveIdAsync(piece, rec, ct);
                        if (id is not null)
                            rec.HayakuIds.Add(id);
                        _eipState.Save();
                    }
                }
                catch
                {
                    if (rec.HayakuIds.Count == 0)
                        _eipState.Leaves.Remove(rec); // 一筆都沒登記成功：下次再試
                    _eipState.Save();
                    throw;
                }
                added.Add(summary);
                if (rec.HayakuIds.Count < pieces.Count)
                    Log.Info($"EIP {leave.No} 登記後找不到 Hayaku 請假 id，之後無法自動取消");
            }
        }
        if (dirty)
            _eipState.Save();
        if (added.Count + cancelled.Count > 0)
        {
            try
            {
                UpdateOffDays(await _api.GetScheduleRangeAsync(today, today.AddDays(EipScheduleDays), ct), today); // 剛登記的請假也算
            }
            catch (Exception ex)
            {
                Log.Error("重新讀取班表失敗", ex);
            }
        }
        return (added, cancelled, skipped);
    }

    static DateOnly Min(DateOnly a, DateOnly b) => a < b ? a : b;

    /// <summary>add_leave 沒回 id 時，從自己的請假裡找時間對得上、還沒記下的那筆。</summary>
    async Task<string?> FindLeaveIdAsync(LeavePiece piece, SyncedLeave rec, CancellationToken ct)
    {
        var mine = await _api.ListLeavesAsync(piece.Date, DateOnly.FromDateTime(piece.To.AddTicks(-1)), ct, "me");
        return mine.FirstOrDefault(m =>
            Math.Abs((m.StartAt.LocalDateTime - piece.From).TotalMinutes) < 1
            && Math.Abs((m.EndAt.LocalDateTime - piece.To).TotalMinutes) < 1
            && !rec.HayakuIds.Contains(m.Id) && !_eipState.IsSynced(m.Id))?.Id;
    }

    async Task CancelSyncedAsync(SyncedLeave rec, CancellationToken ct)
    {
        foreach (var id in rec.HayakuIds)
        {
            try
            {
                await _api.CancelLeaveAsync(id, ct);
            }
            catch (McpException ex)
            {
                Log.Error($"取消 Hayaku 請假 {id}（EIP {rec.No}）失敗，可能已經手動取消", ex);
            }
        }
    }

    /// <summary>
    /// 「EIP 請假紀錄」頁用：某一年自己的假單（待審、核准、駁回都列），讀過就用暫存，reload 才重讀。
    /// 是使用者打開頁面才讀，不受「不上班的日子不送請求」限制。
    /// </summary>
    public async Task<IReadOnlyList<EipLeaveRecord>> LoadEipYearAsync(int year, bool reload)
    {
        if (!reload && _eipYears.TryGetValue(year, out var cached))
            return cached;
        var cred = EipCredentialStore.Load();
        if (cred.IsEmpty)
            throw new EipLoginException("還沒設定 EIP 帳號，請到「設定 › EIP 請假」登入");
        var list = new List<EipLeaveRecord>();
        await _eipGate.WaitAsync();
        try
        {
            _eip ??= new EipClient();
            foreach (var (state, name) in new[] { (EipClient.StatePending, "待審"), (EipClient.StateApproved, "核准"), (EipClient.StateRejected, "駁回") })
                foreach (var leave in await _eip.ListLeavesAsync(year, state, cred, CancellationToken.None))
                    list.Add(new EipLeaveRecord(leave, name));
        }
        finally
        {
            _eipGate.Release();
        }
        list = list.DistinctBy(r => r.Leave.No).OrderByDescending(r => r.Leave.Start).ToList();
        _eipYears[year] = list;
        Log.Info($"讀取 EIP {year} 年假單：{list.Count} 筆");
        return list;
    }

    /// <summary>這張 EIP 假單目前有沒有由同步登記在 Hayaku。</summary>
    public bool IsEipSynced(string no) => _eipState.Leaves.Any(l => l.No == no);

    /// <summary>換了 EIP 帳密：重新登入。</summary>
    public void ResetEip()
    {
        _eip?.Dispose();
        _eip = null;
        _eipYears.Clear();
        _eipRetryAt = DateTime.MinValue;
        if (_eipState.LastResult.StartsWith("失敗"))
        {
            _eipState.LastSyncDay = ""; // 之前因為帳密錯誤停掉的，馬上再試
            _eipState.Save();
        }
    }
}
