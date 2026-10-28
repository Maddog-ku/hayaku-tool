using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Claude;
using HayakuBar.Config;
using HayakuBar.Core;
using HayakuBar.Infra;
using Microsoft.Win32;

namespace HayakuBar.UI;

/// <summary>有屬性變更通知的基底。</summary>
public abstract class Observable : INotifyPropertyChanged
{
    public event PropertyChangedEventHandler? PropertyChanged;

    protected void OnPropertyChanged([CallerMemberName] string? name = null) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));

    protected bool Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
            return false;
        field = value;
        OnPropertyChanged(name);
        return true;
    }
}

/// <summary>主資料夾裡的一個專案。</summary>
public sealed class ProjectItem : Observable
{
    bool _enabled = true;
    string _detail = "";
    Brush _detailBrush = Brushes2.Get("MutedBrush");
    Brush _dotBrush = Brushes2.Get("LineBrush");

    public required string Path { get; init; }
    public required string Name { get; init; }
    public bool IsGit { get; set; }
    public DateTime? LastSession { get; set; }
    public string Source { get; set; } = "";

    public event Action? Edited;

    public bool Enabled
    {
        get => _enabled;
        set
        {
            if (Set(ref _enabled, value))
                Edited?.Invoke();
        }
    }

    public string Detail { get => _detail; set => Set(ref _detail, value); }
    public Brush DetailBrush { get => _detailBrush; set => Set(ref _detailBrush, value); }
    public Brush DotBrush { get => _dotBrush; set => Set(ref _dotBrush, value); }

    public void Init(bool enabled) => _enabled = enabled;
}

/// <summary>一個主資料夾（例 D:\repo）與裡面偵測到的專案。</summary>
public sealed class RepoItem : Observable
{
    bool _enabled = true;
    bool _includeSubprojects = true;
    int _hooksDone;
    int _hooksTotal = 1;
    string _activityText = "";
    Brush _activityBrush = Brushes2.Get("MutedBrush");
    Visibility _rootActivityVisibility = Visibility.Visible;

    public required string Path { get; init; }
    public string Name => System.IO.Path.GetFileName(Path.TrimEnd('\\', '/'));
    public ObservableCollection<ProjectItem> Projects { get; } = [];

    /// <summary>設定裡有、但這次沒偵測到的專案設定；存檔時保留，資料夾回來時設定還在。</summary>
    public List<ProjectConfig> HiddenProjectConfigs { get; } = [];

    /// <summary>使用者改了任何設定時觸發，用來立即存檔。</summary>
    public event Action? Edited;

    public bool Enabled
    {
        get => _enabled;
        set
        {
            if (Set(ref _enabled, value))
                Edited?.Invoke();
        }
    }

    public bool IncludeSubprojects
    {
        get => _includeSubprojects;
        set
        {
            if (!Set(ref _includeSubprojects, value))
                return;
            RaiseProjectsChanged();
            Edited?.Invoke();
        }
    }

    public string ProjectsHeader => IncludeSubprojects ? $"裡面的專案（{Projects.Count}）" : "裡面的專案（未偵測）";
    public Visibility ProjectsVisibility => IncludeSubprojects && Projects.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    public Visibility ProjectsEmptyVisibility => IncludeSubprojects && Projects.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

    public bool HooksInstalled => _hooksDone >= _hooksTotal;
    public string HooksText => _hooksTotal <= 1
        ? (HooksInstalled ? "✓ Hooks 已安裝" : "Hooks 未安裝")
        : _hooksDone == 0 ? "Hooks 未安裝" : $"✓ Hooks {_hooksDone}/{_hooksTotal}";
    public Brush HooksBrush => Brushes2.Get(_hooksDone > 0 ? "RunBrush" : "MutedBrush");
    public string HooksAction => HooksInstalled ? "移除 hooks" : "安裝 hooks";

    public string ActivityText { get => _activityText; set => Set(ref _activityText, value); }
    public Brush ActivityBrush { get => _activityBrush; set => Set(ref _activityBrush, value); }
    public Visibility RootActivityVisibility { get => _rootActivityVisibility; set => Set(ref _rootActivityVisibility, value); }

    public void Init(RepoConfig config) =>
        (_enabled, _includeSubprojects) = (config.Enabled, config.IncludeSubprojects);

    public void SetHooks(int done, int total)
    {
        (_hooksDone, _hooksTotal) = (done, Math.Max(1, total));
        OnPropertyChanged(nameof(HooksText));
        OnPropertyChanged(nameof(HooksBrush));
        OnPropertyChanged(nameof(HooksAction));
    }

    public void RaiseProjectsChanged()
    {
        OnPropertyChanged(nameof(ProjectsHeader));
        OnPropertyChanged(nameof(ProjectsVisibility));
        OnPropertyChanged(nameof(ProjectsEmptyVisibility));
    }
}

/// <summary>設定頁：左側分頁、右側卡片，改了立刻生效。</summary>
public partial class SettingsWindow : Window
{
    readonly TimerEngine _engine;
    readonly ObservableCollection<RepoItem> _repos = [];
    bool _loading = true;

    public SettingsWindow(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        RepoList.ItemsSource = _repos;

        var c = engine.Config;
        AutoScheduleBox.IsChecked = c.AutoSchedule;
        ClaudeEnabledBox.IsChecked = c.ClaudeEnabled;
        ClaudeOnlyWorkBox.IsChecked = c.ClaudeOnlyDuringWork;
        StartupBox.IsChecked = StartupRegistration.IsEnabled();
        PollBox.SelectedValue = new[] { 15, 30, 60, 120 }.MinBy(v => Math.Abs(v - c.PollSeconds)).ToString();
        ThemeBox.SelectedValue = c.ThemeMode is "Light" or "Dark" ? c.ThemeMode : "System";
        PanelHeightBox.SelectedValue = (c.PanelHeight <= 0 ? 0 : new[] { 360, 440, 520, 600 }.MinBy(v => Math.Abs(v - c.PanelHeight))).ToString();
        HotkeyBox.Text = string.IsNullOrWhiteSpace(c.Hotkey) ? "（不使用）" : c.Hotkey.Replace("+", " + ");
        MeetingAlertsBox.IsChecked = c.MeetingAlerts;
        MeetingLeadBox.SelectedValue = new[] { 0, 1, 3, 5, 10, 15 }.MinBy(v => Math.Abs(v - c.MeetingLeadMinutes)).ToString();
        ReminderAlertsBox.IsChecked = c.ReminderAlerts;
        NotificationAlertsBox.IsChecked = c.NotificationAlerts;
        PopupSecondsBox.SelectedValue = (c.NotificationPopupSeconds <= 0 ? 0
            : new[] { 3, 5, 8, 15 }.MinBy(v => Math.Abs(v - c.NotificationPopupSeconds))).ToString();
        SyncOnlyWorkBox.IsChecked = c.SyncOnlyDuringWork;
        PopupSecondsBox.IsEnabled = c.NotificationAlerts;
        PromptUnfilledBox.IsChecked = c.PromptUnfilled;
        QuietWhenBusyBox.IsChecked = c.QuietWhenBusy;
        LoadTaskbar();
        foreach (var r in c.Repos)
            AddRepoItem(r);

        DataDirText.Text = AppPaths.Dir;
        var version = Assembly.GetExecutingAssembly().GetName().Version;
        VersionText.Text = $"版本 {version?.ToString(3)}";

        NavAccount.IsChecked = true;
        RefreshLive();
        _loading = false;

        _engine.Changed += RefreshLive;
        ThemeManager.Changed += RefreshLive;
        Closed += (_, _) =>
        {
            _engine.Changed -= RefreshLive;
            ThemeManager.Changed -= RefreshLive;
        };
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);
        Activated += (_, _) => LoadTaskbar(); // 右鍵選單也能改位置，回到這個視窗時同步
    }

    void Apply()
    {
        if (!_loading)
            _engine.ApplyConfig();
    }

    // ───────────── 導覽 ─────────────

    /// <summary>切到指定分頁：account、taskbar、schedule、repos、claude、general。</summary>
    public void ShowPage(string page)
    {
        var target = "Page" + page;
        foreach (var nav in NavPanel.Children.OfType<RadioButton>())
            if (string.Equals(nav.Tag as string, target, StringComparison.OrdinalIgnoreCase))
                nav.IsChecked = true;
    }

    void Nav_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string page })
            return;
        foreach (var p in new FrameworkElement[] { PageAccount, PageTaskbar, PageSchedule, PageRepos, PageClaude, PageGeneral })
            p.Visibility = p.Name == page ? Visibility.Visible : Visibility.Collapsed;
    }

    // ───────────── 會隨時間變的內容 ─────────────

    void RefreshLive()
    {
        var user = _engine.User;
        if (_engine.LoggedIn && user is not null)
        {
            AccountName.Text = user.DisplayName;
            AccountDetail.Text = $"{user.Username} · 已連線";
            AvatarText.Text = user.DisplayName.Length > 0 ? user.DisplayName[..1] : "?";
            NavUserText.Text = user.DisplayName;
        }
        else
        {
            AccountName.Text = _engine.LoggingIn ? "等待瀏覽器登入…" : "尚未登入";
            AccountDetail.Text = "登入後才能同步計時";
            AvatarText.Text = "?";
            NavUserText.Text = "未登入";
        }
        AccountButton.Content = _engine.LoggedIn ? "登出" : "登入";
        AccountButton.Style = (Style)FindResource(_engine.LoggedIn ? "FlatButton" : "PrimaryButton");
        AccountButton.IsEnabled = !_engine.LoggingIn;

        var schedule = _engine.Schedule;
        ScheduleText.Text = schedule is null
            ? "登入後顯示"
            : schedule.Holiday is { } holiday ? $"放假（{holiday}）"
            : schedule.Working.Count == 0 ? (schedule.OnLeave ? "請假" : "不上班")
            : $"{string.Join("、", schedule.Working.Select(s => $"{s.Start:HH:mm}–{s.End:HH:mm}"))}　應出勤 {WorkMath.HoursMinutes(schedule.ExpectedMinutes)}";

        var now = DateTime.Now;
        foreach (var item in _repos)
        {
            var root = Unit(item.Path);
            (item.ActivityText, item.ActivityBrush) = Describe(root, now);
            // 有專案、主資料夾本身又沒有 session 時，不顯示主資料夾那一行
            item.RootActivityVisibility = item.IncludeSubprojects && item.Projects.Count > 0 && root is not { LastActivity.Ticks: > 0 }
                ? Visibility.Collapsed
                : Visibility.Visible;
            foreach (var project in item.Projects)
                DescribeProject(project, Unit(project.Path), now);
        }
        RepoEmpty.Visibility = _repos.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    RepoActivity? Unit(string path) =>
        _engine.Claude.Repos.FirstOrDefault(a => string.Equals(a.Path, path, StringComparison.OrdinalIgnoreCase));

    static string Ago(DateTime time, DateTime now)
    {
        var span = now - time;
        return span.TotalMinutes < 1 ? "剛剛" : span.TotalHours < 1 ? $"{(int)span.TotalMinutes} 分鐘前" :
            span.TotalDays < 1 ? $"{(int)span.TotalHours} 小時前" : $"{(int)span.TotalDays} 天前";
    }

    static string TaskText(RepoActivity a) =>
        a.ResolvedTask is { } t ? $"任務 {t}（{a.TaskSource}）" : "偵測不到任務編號，不計時；可在面板上按「選擇」指定";

    static (string, Brush) Describe(RepoActivity? a, DateTime now)
    {
        if (a is null || a.LastActivity == DateTime.MinValue)
            return ("主資料夾本身還沒有 Claude session 紀錄。", Brushes2.Get("MutedBrush"));
        var source = a.SourceText.Length > 0 ? $"（{a.SourceText}）" : "";
        return a.Active
            ? ($"● 主資料夾本身：Claude 活動中{source} · {TaskText(a)}", Brushes2.Get("ClaudeBrush"))
            : ($"主資料夾本身：最近活動 {Ago(a.LastActivity, now)}{source} · {TaskText(a)}", Brushes2.Get("MutedBrush"));
    }

    static void DescribeProject(ProjectItem p, RepoActivity? a, DateTime now)
    {
        var parts = new List<string> { p.IsGit ? "Git" : "資料夾" };
        if (!p.Enabled)
        {
            p.Detail = string.Join(" · ", parts.Append("已停用"));
            (p.DetailBrush, p.DotBrush) = (Brushes2.Get("MutedBrush"), Brushes2.Get("LineBrush"));
            return;
        }
        if (a is { Active: true })
        {
            p.Detail = $"● Claude 活動中{(a.SourceText.Length > 0 ? $"（{a.SourceText}）" : "")} · {TaskText(a)}";
            (p.DetailBrush, p.DotBrush) = (Brushes2.Get("ClaudeBrush"), Brushes2.Get("ClaudeBrush"));
            return;
        }
        var last = a is { LastActivity.Ticks: > 0 } ? a.LastActivity : p.LastSession;
        var source = a?.SourceText is { Length: > 0 } s ? s : p.Source;
        if (last is { } time)
        {
            if (source.Length > 0)
                parts.Add(source);
            parts.Add(Ago(time, now));
            if (a?.ResolvedTask is { } task)
                parts.Add($"任務 {task}");
        }
        else
        {
            parts.Add("還沒有 Claude session");
        }
        p.Detail = string.Join(" · ", parts);
        (p.DetailBrush, p.DotBrush) = (Brushes2.Get("MutedBrush"), Brushes2.Get(last is null ? "LineBrush" : "RunBrush"));
    }

    // ───────────── 帳號 ─────────────

    async void Account_Click(object sender, RoutedEventArgs e)
    {
        if (_engine.LoggedIn)
            await _engine.LogoutAsync();
        else
            await _engine.LoginAsync();
    }

    void OpenWeb_Click(object sender, RoutedEventArgs e) => Open(_engine.Config.BaseUrl);

    // ───────────── 工作列 ─────────────

    void LoadTaskbar()
    {
        var wasLoading = _loading;
        _loading = true;
        var c = _engine.Config;
        AnchorBox.SelectedValue = c.TaskbarAnchor;
        if (AnchorBox.SelectedIndex < 0)
            AnchorBox.SelectedValue = "BeforeTray";
        if (string.Equals(c.TaskbarLayout, "OneLine", StringComparison.OrdinalIgnoreCase))
            OneLineRadio.IsChecked = true;
        else
            TwoLineRadio.IsChecked = true;
        if (!OffsetBox.IsKeyboardFocusWithin)
            OffsetBox.Text = Math.Round(c.TaskbarOffset).ToString();
        _loading = wasLoading;
    }

    void Anchor_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || AnchorBox.SelectedValue is not string anchor || anchor == _engine.Config.TaskbarAnchor)
            return;
        _engine.Config.TaskbarAnchor = anchor;
        _engine.Config.TaskbarOffset = 0; // 換基準位置時微調歸零
        OffsetBox.Text = "0";
        Apply();
    }

    void Layout_Checked(object sender, RoutedEventArgs e)
    {
        if (_loading)
            return;
        _engine.Config.TaskbarLayout = OneLineRadio.IsChecked == true ? "OneLine" : "TwoLine";
        Apply();
    }

    void Offset_LostFocus(object sender, RoutedEventArgs e)
    {
        if (!double.TryParse(OffsetBox.Text, out var offset))
        {
            OffsetBox.Text = Math.Round(_engine.Config.TaskbarOffset).ToString();
            return;
        }
        offset = Math.Clamp(offset, -3000, 3000);
        OffsetBox.Text = Math.Round(offset).ToString();
        if (Math.Abs(offset - _engine.Config.TaskbarOffset) < 0.5)
            return;
        _engine.Config.TaskbarOffset = offset;
        Apply();
    }

    void ResetOffset_Click(object sender, RoutedEventArgs e)
    {
        OffsetBox.Text = "0";
        _engine.Config.TaskbarOffset = 0;
        Apply();
    }

    /// <summary>輸入框按 Enter 就套用（等同離開輸入框）。</summary>
    void CommitOnEnter(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter || sender is not TextBox box)
            return;
        box.GetBindingExpression(TextBox.TextProperty)?.UpdateSource();
        if (box == OffsetBox)
            Offset_LostFocus(box, e);
        Keyboard.ClearFocus();
        e.Handled = true;
    }

    // ───────────── 開關 ─────────────

    void Toggle_Click(object sender, RoutedEventArgs e)
    {
        var c = _engine.Config;
        c.AutoSchedule = AutoScheduleBox.IsChecked == true;
        c.ClaudeEnabled = ClaudeEnabledBox.IsChecked == true;
        c.ClaudeOnlyDuringWork = ClaudeOnlyWorkBox.IsChecked == true;
        c.MeetingAlerts = MeetingAlertsBox.IsChecked == true;
        c.ReminderAlerts = ReminderAlertsBox.IsChecked == true;
        c.NotificationAlerts = NotificationAlertsBox.IsChecked == true;
        c.PromptUnfilled = PromptUnfilledBox.IsChecked == true;
        c.QuietWhenBusy = QuietWhenBusyBox.IsChecked == true;
        c.SyncOnlyDuringWork = SyncOnlyWorkBox.IsChecked == true;
        PopupSecondsBox.IsEnabled = c.NotificationAlerts;
        Apply();
    }

    void PopupSeconds_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || PopupSecondsBox.SelectedValue is not string value || !int.TryParse(value, out var seconds))
            return;
        _engine.Config.NotificationPopupSeconds = seconds;
        Apply();
    }

    void MeetingLead_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || MeetingLeadBox.SelectedValue is not string value || !int.TryParse(value, out var minutes))
            return;
        _engine.Config.MeetingLeadMinutes = minutes;
        Apply();
    }

    /// <summary>錄製快捷鍵：按下組合鍵就套用；Backspace／Delete 取消快捷鍵。</summary>
    void HotkeyBox_PreviewKeyDown(object sender, KeyEventArgs e)
    {
        e.Handled = true;
        var key = e.Key == Key.System ? e.SystemKey : e.Key;
        if (key is Key.Back or Key.Delete)
        {
            SetHotkey("");
            return;
        }
        if (key is Key.Tab or Key.Escape)
        {
            Keyboard.ClearFocus();
            return;
        }
        var win = Keyboard.IsKeyDown(Key.LWin) || Keyboard.IsKeyDown(Key.RWin);
        if (GlobalHotkey.Format(Keyboard.Modifiers, win, key) is { } text)
            SetHotkey(text);
    }

    void HotkeyDefault_Click(object sender, RoutedEventArgs e) => SetHotkey("Win+Shift+H");

    void SetHotkey(string text)
    {
        _engine.Config.Hotkey = text;
        HotkeyBox.Text = text.Length == 0 ? "（不使用）" : text.Replace("+", " + ");
        Apply(); // App 收到 ConfigApplied 會重新註冊；被占用時會跳提示
    }

    void Startup_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            StartupRegistration.Set(StartupBox.IsChecked == true);
        }
        catch (Exception ex)
        {
            Log.Error("設定開機啟動失敗", ex);
            StartupBox.IsChecked = StartupRegistration.IsEnabled();
            MessageBox.Show(this, $"設定失敗：{ex.Message}", "HayakuBar", MessageBoxButton.OK, MessageBoxImage.Error);
        }
    }

    void Poll_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || PollBox.SelectedValue is not string value || !int.TryParse(value, out var seconds))
            return;
        _engine.Config.PollSeconds = seconds;
        Apply();
    }

    void Theme_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || ThemeBox.SelectedValue is not string mode)
            return;
        _engine.Config.ThemeMode = mode;
        ThemeManager.SetMode(mode);
        Apply();
    }

    void PanelHeight_Changed(object sender, SelectionChangedEventArgs e)
    {
        if (_loading || PanelHeightBox.SelectedValue is not string value || !int.TryParse(value, out var height))
            return;
        _engine.Config.PanelHeight = height;
        Apply();
    }

    void OpenDataDir_Click(object sender, RoutedEventArgs e) => Open(AppPaths.Dir);

    // ───────────── 主資料夾與專案 ─────────────

    void AddRepoItem(RepoConfig config)
    {
        var item = new RepoItem { Path = ClaudeMonitor.Normalize(config.Path) };
        item.Init(config);
        LoadProjects(item, config.Projects, refresh: false);
        item.Edited += () => OnRepoEdited(item);
        _repos.Add(item);
    }

    /// <summary>偵測主資料夾裡的專案，套上已存的個別設定。</summary>
    void LoadProjects(RepoItem item, IEnumerable<ProjectConfig> saved, bool refresh)
    {
        var configs = saved.ToList();
        foreach (var old in item.Projects)
            old.Edited -= SaveRepos;
        item.Projects.Clear();
        item.HiddenProjectConfigs.Clear();
        var detected = _engine.Claude.DetectProjects(item.Path, refresh);
        foreach (var d in detected)
        {
            var custom = configs.FirstOrDefault(c => string.Equals(c.Name, d.Name, StringComparison.OrdinalIgnoreCase));
            var project = new ProjectItem { Path = d.Path, Name = d.Name, IsGit = d.IsGit, LastSession = d.LastSession, Source = d.Source };
            project.Init(custom?.Enabled ?? true);
            project.Edited += SaveRepos;
            item.Projects.Add(project);
        }
        item.HiddenProjectConfigs.AddRange(configs.Where(c =>
            !detected.Any(d => string.Equals(d.Name, c.Name, StringComparison.OrdinalIgnoreCase))));
        item.RaiseProjectsChanged();
        UpdateHooks(item);
    }

    void OnRepoEdited(RepoItem item)
    {
        SaveRepos();
        UpdateHooks(item);
    }

    void SaveRepos()
    {
        _engine.Config.Repos = _repos.Select(r => new RepoConfig
        {
            Path = r.Path,
            Enabled = r.Enabled,
            IncludeSubprojects = r.IncludeSubprojects,
            Projects = r.Projects
                .Where(p => !p.Enabled)
                .Select(p => new ProjectConfig { Name = p.Name, Enabled = p.Enabled })
                .Concat(r.HiddenProjectConfigs)
                .ToList(),
        }).ToList();
        Apply();
        RefreshLive();
    }

    /// <summary>hooks 要裝在每個會開 Claude 的資料夾：主資料夾本身＋啟用中的專案。</summary>
    static List<string> HookTargets(RepoItem item) =>
        new[] { item.Path }
            .Concat(item.IncludeSubprojects ? item.Projects.Where(p => p.Enabled).Select(p => p.Path) : [])
            .ToList();

    void UpdateHooks(RepoItem item)
    {
        var targets = HookTargets(item);
        var done = targets.Count(t =>
        {
            try { return HookInstaller.IsInstalled(t, _engine.Config.HookPort); }
            catch { return false; }
        });
        item.SetHooks(done, targets.Count);
    }

    void Redetect_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not RepoItem item)
            return;
        var saved = item.Projects
            .Where(p => !p.Enabled)
            .Select(p => new ProjectConfig { Name = p.Name, Enabled = p.Enabled })
            .Concat(item.HiddenProjectConfigs)
            .ToList();
        LoadProjects(item, saved, refresh: true);
        SaveRepos();
    }

    void AddRepo_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog { Title = "選擇放專案的主資料夾（例 D:\\repo），或單一專案資料夾" };
        if (dialog.ShowDialog(this) == true)
            AddFolder(dialog.FolderName);
    }

    void AddFolder(string folder)
    {
        var path = ClaudeMonitor.Normalize(folder);
        if (_repos.Any(r => string.Equals(r.Path, path, StringComparison.OrdinalIgnoreCase)))
            return;
        // 已經是某個主資料夾裡的專案：不用另外加
        if (_repos.FirstOrDefault(r => r.Projects.Any(p => string.Equals(p.Path, path, StringComparison.OrdinalIgnoreCase))) is { } parent)
        {
            MessageBox.Show(this, $"「{System.IO.Path.GetFileName(path)}」已經是「{parent.Name}」裡偵測到的專案，不用另外加。",
                "HayakuBar", MessageBoxButton.OK, MessageBoxImage.Information);
            return;
        }
        AddRepoItem(new RepoConfig { Path = path });
        SaveRepos();
    }

    void Discover_Click(object sender, RoutedEventArgs e)
    {
        var menu = new ContextMenu { PlacementTarget = DiscoverButton, Placement = System.Windows.Controls.Primitives.PlacementMode.Bottom };
        var now = DateTime.Now;
        var covered = _repos.SelectMany(r => r.Projects.Select(p => p.Path).Prepend(r.Path)).ToList();
        var projects = ClaudeMonitor.Discover()
            .Where(p => !covered.Any(c => string.Equals(c, p.Path, StringComparison.OrdinalIgnoreCase)))
            .ToList();
        foreach (var p in projects)
        {
            var item = new MenuItem
            {
                Header = new TextBlock
                {
                    Inlines =
                    {
                        new System.Windows.Documents.Run(p.Path),
                        new System.Windows.Documents.Run($"　{p.Source} · {Ago(p.LastActivity, now)}")
                        {
                            Foreground = Brushes2.Get("MutedBrush"),
                            FontSize = 12,
                        },
                    },
                },
            };
            item.Click += (_, _) => AddFolder(p.Path);
            menu.Items.Add(item);
        }
        if (projects.Count == 0)
            menu.Items.Add(new MenuItem { Header = "最近用過的資料夾都已經在清單裡了", IsEnabled = false });
        menu.IsOpen = true;
    }

    void RemoveRepo_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not RepoItem item)
            return;
        var ok = MessageBox.Show(this, $"從清單移除「{item.Name}」？（不會刪除任何檔案）", "移除資料夾",
            MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok != MessageBoxResult.OK)
            return;
        _repos.Remove(item);
        SaveRepos();
    }

    void Hooks_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not RepoItem item)
            return;
        var port = _engine.Config.HookPort;
        var targets = HookTargets(item);
        var uninstall = item.HooksInstalled;
        if (!uninstall)
        {
            var files = string.Join("\n", targets.Select(HookInstaller.SettingsPath));
            var ok = MessageBox.Show(this,
                $"會在下面 {targets.Count} 個檔案加入 UserPromptSubmit／Stop／SessionEnd 三個 hook（原檔會備份成 .hayakubar.bak）：\n\n{files}\n\n確定要安裝嗎？",
                "安裝 Claude hooks", MessageBoxButton.OKCancel, MessageBoxImage.Question);
            if (ok != MessageBoxResult.OK)
                return;
        }
        var failed = new List<string>();
        foreach (var target in targets)
        {
            try
            {
                if (uninstall)
                    HookInstaller.Uninstall(target, port);
                else
                    HookInstaller.Install(target, port);
            }
            catch (Exception ex)
            {
                Log.Error($"變更 hooks 失敗：{target}", ex);
                failed.Add($"{target}：{ex.Message}");
            }
        }
        UpdateHooks(item);
        if (failed.Count > 0)
            MessageBox.Show(this, "部分資料夾失敗：\n" + string.Join("\n", failed), "HayakuBar", MessageBoxButton.OK, MessageBoxImage.Warning);
    }

    static void Open(string target)
    {
        try
        {
            Process.Start(new ProcessStartInfo(target) { UseShellExecute = true });
        }
        catch (Exception ex)
        {
            Log.Error($"開啟 {target} 失敗", ex);
        }
    }
}
