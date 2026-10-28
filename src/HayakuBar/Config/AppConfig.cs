using HayakuBar.Infra;

namespace HayakuBar.Config;

/// <summary>主資料夾（例 D:\repo）。裡面的專案會自動偵測。</summary>
public sealed class RepoConfig
{
    public string Path { get; set; } = "";

    public bool Enabled { get; set; } = true;

    /// <summary>自動偵測裡面的專案（子資料夾是 git repo，或 Claude 曾在裡面開過 session）。</summary>
    public bool IncludeSubprojects { get; set; } = true;

    /// <summary>主資料夾本身的 session 改了程式、又對應不到任務時，詢問要不要建立任務。</summary>
    public bool AutoCreate { get; set; }

    /// <summary>子專案的個別設定（只存有改過的）。</summary>
    public List<ProjectConfig> Projects { get; set; } = [];
}

/// <summary>主資料夾裡某個專案的個別設定，用資料夾名稱對應。</summary>
public sealed class ProjectConfig
{
    public string Name { get; set; } = "";
    public bool Enabled { get; set; } = true;

    /// <summary>Claude 在這個專案改了程式、又對應不到任務時，詢問要不要建立任務。</summary>
    public bool AutoCreate { get; set; }

    /// <summary>跟預設值一樣就不用存。</summary>
    [System.Text.Json.Serialization.JsonIgnore]
    public bool IsDefault => Enabled && !AutoCreate;
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

    /// <summary>外觀：System（跟隨 Windows）、Light、Dark。</summary>
    public string ThemeMode { get; set; } = "System";

    /// <summary>面板內容區的高度（DIP），切分頁時不會跳；0 表示跟著內容伸縮（最高 440）。</summary>
    public int PanelHeight { get; set; } = 440;

    /// <summary>工作台「我的任務」的排序：default、due、dueDesc、priority、status、key、timing。</summary>
    public string TaskSort { get; set; } = "default";

    /// <summary>工作台「我的任務」的顯示方式：list（清單）、tree（依專案分組的樹狀）或 kanban（看板）。</summary>
    public string TaskView { get; set; } = "list";

    /// <summary>工作台「我的任務」只看哪個專案（專案名稱）；空字串＝全部專案。</summary>
    public string TaskProject { get; set; } = "";

    public int PollSeconds { get; set; } = 30;

    /// <summary>只在上班時間（或有計時在跑）自動同步；下班、休息、放假不跟 Hayaku 要資料。</summary>
    public bool SyncOnlyDuringWork { get; set; } = true;

    /// <summary>開關面板的全域快捷鍵，例 Win+Shift+H；空字串表示不用。</summary>
    public string Hotkey { get; set; } = "Win+Shift+H";

    /// <summary>會議開始前幾分鐘跳出提示（可一鍵加入並計時）。</summary>
    public bool MeetingAlerts { get; set; } = true;
    public int MeetingLeadMinutes { get; set; } = 5;

    /// <summary>也提醒我專案裡、我不是參加者的會議（加入時 Hayaku 會自動把我加為參加者）。</summary>
    public bool MeetingAlertsAllProjects { get; set; } = true;

    /// <summary>提醒到點時跳出提示（可稍後再提醒）。</summary>
    public bool ReminderAlerts { get; set; } = true;

    /// <summary>有新通知時跳出提示。</summary>
    public bool NotificationAlerts { get; set; } = true;

    /// <summary>新通知的彈窗停留幾秒後自動收回；0 表示不自動收回。</summary>
    public int NotificationPopupSeconds { get; set; } = 3;

    /// <summary>全螢幕、簡報、Windows 安靜時段或鎖定時先不跳提示，結束後再一起顯示（會議快開始除外）。</summary>
    public bool QuietWhenBusy { get; set; } = true;

    /// <summary>下班時工時沒滿應出勤，提示補登。</summary>
    public bool PromptUnfilled { get; set; } = true;
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
