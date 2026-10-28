using HayakuBar.Infra;

namespace HayakuBar.Config;

/// <summary>執行狀態，存在 %APPDATA%\HayakuBar\state.json，重開工具後延續。</summary>
public sealed class AppState
{
    /// <summary>下面幾個「當天」欄位所屬的日期（yyyy-MM-dd）。</summary>
    public string Date { get; set; } = "";

    /// <summary>今天已處理過的班表時間點（S0＝第一段上班開始、E0＝第一段結束…）。</summary>
    public List<string> HandledBoundaries { get; set; } = [];

    /// <summary>今天手動按了下班：當天不再自動上下班。</summary>
    public bool ClockedOut { get; set; }

    /// <summary>下一次上班／午休結束時要恢復的任務（跨日保留）。</summary>
    public List<string> ResumeSet { get; set; } = [];

    /// <summary>由 Claude 偵測自動開始的計時（面板上標 ✦）。</summary>
    public List<string> ClaudeOwned { get; set; } = [];

    /// <summary>Claude session → 使用者手動指定的任務。</summary>
    public Dictionary<string, string> SessionTasks { get; set; } = [];

    /// <summary>換日時重設當天欄位；有換日回傳 true。</summary>
    public bool RollDay(string today)
    {
        if (Date == today)
            return false;
        Date = today;
        HandledBoundaries.Clear();
        ClockedOut = false;
        ClaudeOwned.Clear();
        if (SessionTasks.Count > 200)
            SessionTasks.Clear();
        return true;
    }

    public static AppState Load() => JsonFile.Load<AppState>(AppPaths.State);

    public void Save() => JsonFile.Save(AppPaths.State, this);
}
