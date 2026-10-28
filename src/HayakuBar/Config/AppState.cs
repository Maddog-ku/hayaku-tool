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

    /// <summary>問過「要不要建立任務」的 Claude session，不再重複問。</summary>
    public List<string> AskedSessions { get; set; } = [];

    /// <summary>資料夾 → 在那裡做事時用的 Hayaku 專案與上次新任務的上層，建立任務時自動帶入。</summary>
    public Dictionary<string, FolderHint> FolderHints
    {
        get => _folderHints;
        set => _folderHints = new Dictionary<string, FolderHint>(value, StringComparer.OrdinalIgnoreCase);
    }

    Dictionary<string, FolderHint> _folderHints = new(StringComparer.OrdinalIgnoreCase);

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
        if (AskedSessions.Count > 200)
            AskedSessions.RemoveRange(0, AskedSessions.Count - 100);
        return true;
    }

    public static AppState Load() => JsonFile.Load<AppState>(AppPaths.State);

    public void Save() => JsonFile.Save(AppPaths.State, this);
}

/// <summary>某個資料夾對應的 Hayaku 專案；Parent 是上次在這裡建立新任務時選的上層（null＝最上層）。</summary>
public sealed class FolderHint
{
    public string Project { get; set; } = "";
    public string? Parent { get; set; }
}
