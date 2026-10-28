using HayakuBar.Infra;

namespace HayakuBar.Config;

public sealed class RepoConfig
{
    public string Path { get; set; } = "";
    public string? DefaultTask { get; set; }
    public bool Enabled { get; set; } = true;
}

/// <summary>使用者設定，存在 %APPDATA%\HayakuBar\config.json。</summary>
public sealed class AppConfig
{
    public string BaseUrl { get; set; } = "https://hayaku.revivalsi.com";

    /// <summary>依 Hayaku 班表自動上班／午休暫停／午休後恢復／下班停止。</summary>
    public bool AutoSchedule { get; set; } = true;

    public bool ClaudeEnabled { get; set; } = true;
    public bool ClaudeOnlyDuringWork { get; set; } = true;
    /// <summary>session 紀錄在幾分鐘內有變動才算「Claude 正在動」，用來決定要不要開始計時（不會因此暫停）。</summary>
    public int ClaudeIdleMinutes { get; set; } = 10;

    public int PollSeconds { get; set; } = 30;
    public int HookPort { get; set; } = 47823;

    public List<RepoConfig> Repos { get; set; } = [];

    /// <summary>在工作列上的基準位置：Left（最左邊）、BeforeStart（圖示左側）、AfterIcons（圖示右側）、BeforeTray（通知區域左側）。</summary>
    public string TaskbarAnchor { get; set; } = "BeforeTray";

    /// <summary>工作列上的顯示方式：TwoLine（兩排，較窄）或 OneLine（一排，較寬）。</summary>
    public string TaskbarLayout { get; set; } = "TwoLine";

    /// <summary>拖曳微調的偏移量（DIP，正數往右）。</summary>
    public double TaskbarOffset { get; set; }

    public static AppConfig Load() => JsonFile.Load<AppConfig>(AppPaths.Config);

    public void Save() => JsonFile.Save(AppPaths.Config, this);
}
