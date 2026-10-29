using HayakuBar.Infra;

namespace HayakuBar.Eip;

/// <summary>由同步建立的 Hayaku 請假；只有這些會因為 EIP 駁回、抽單而被取消。</summary>
public sealed class SyncedLeave
{
    /// <summary>EIP 單號。</summary>
    public string No { get; set; } = "";
    public string Type { get; set; } = "";
    public DateTime Start { get; set; }
    public DateTime End { get; set; }
    public List<string> HayakuIds { get; set; } = [];

    /// <summary>給人看的說明，例「11/3（一）整天 特休」。</summary>
    public string Summary { get; set; } = "";
}

/// <summary>EIP 請假同步的狀態，存在 %APPDATA%\HayakuBar\eip-sync.json。</summary>
public sealed class EipSyncState
{
    /// <summary>上次成功同步的日期（yyyy-MM-dd），一天只自動同步一次。</summary>
    public string LastSyncDay { get; set; } = "";
    public DateTime? LastSyncAt { get; set; }

    /// <summary>上次同步的結果（設定頁顯示）。</summary>
    public string LastResult { get; set; } = "";

    public List<SyncedLeave> Leaves { get; set; } = [];

    public bool IsSynced(string hayakuId) => Leaves.Any(l => l.HayakuIds.Contains(hayakuId));

    public static EipSyncState Load() => JsonFile.Load<EipSyncState>(AppPaths.EipSync);

    public void Save() => JsonFile.Save(AppPaths.EipSync, this);
}
