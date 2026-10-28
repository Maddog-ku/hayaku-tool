using System.Collections.ObjectModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using HayakuBar.Core;

namespace HayakuBar.UI;

/// <summary>點工作列上的工時後往上彈出的面板；失去焦點就收起來。</summary>
public partial class PanelWindow : Window
{
    const double ProgressWidth = 90;

    readonly TimerEngine _engine;
    readonly ObservableCollection<TaskRowVm> _running = [];
    readonly ObservableCollection<TaskRowVm> _paused = [];
    readonly ObservableCollection<RepoRowVm> _repos = [];
    readonly DispatcherTimer _tick = new() { Interval = TimeSpan.FromSeconds(1) };
    Rect _anchor;
    bool _alignLeft;
    bool _dialogOpen;

    public event Action? SettingsRequested;

    public PanelWindow(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        RunningList.ItemsSource = _running;
        PausedList.ItemsSource = _paused;
        RepoList.ItemsSource = _repos;

        _engine.Changed += () =>
        {
            if (IsVisible)
                Render();
        };
        _tick.Tick += (_, _) => UpdateNumbers();
        SizeChanged += (_, _) => Reposition();
        Deactivated += (_, _) =>
        {
            if (!_dialogOpen)
                HidePanel();
        };
        PreviewKeyDown += (_, e) =>
        {
            if (e.Key == Key.Escape)
                HidePanel();
        };
    }

    /// <summary>上一次因為失去焦點而收起的時間；用來避免「點工作列 → 收起 → 又被同一下點擊打開」。</summary>
    public DateTime HiddenAt { get; private set; }

    /// <summary>anchor：工作列上工時的位置（DIP）；面板出現在它正上方。</summary>
    public void ShowAt(Rect anchor, bool alignLeft)
    {
        _anchor = anchor;
        _alignLeft = alignLeft;
        Render();
        Show();
        Reposition();
        Activate();
        _tick.Start();
        if (_engine.LoggedIn)
            _ = _engine.RefreshAsync(userAction: false);
    }

    public void HidePanel()
    {
        if (!IsVisible)
            return;
        Hide();
        _tick.Stop();
        HiddenAt = DateTime.Now;
    }

    void Reposition()
    {
        var work = SystemParameters.WorkArea;
        var left = _alignLeft ? _anchor.Left - 6 : _anchor.Right - ActualWidth + 6;
        Left = Math.Clamp(left, work.Left, Math.Max(work.Left, work.Right - ActualWidth));
        Top = Math.Max(work.Top, Math.Min(_anchor.Top, work.Bottom) - ActualHeight - 2);
    }

    // ───────────── 顯示 ─────────────

    void Render()
    {
        var loggedIn = _engine.LoggedIn;
        LoginButton.Visibility = LoginHint.Visibility = loggedIn ? Visibility.Collapsed : Visibility.Visible;
        LoginButton.IsEnabled = !_engine.LoggingIn;
        LoginButton.Content = _engine.LoggingIn ? "等待瀏覽器登入…" : "登入 Hayaku";
        MainContent.Visibility = SummaryRow.Visibility = loggedIn ? Visibility.Visible : Visibility.Collapsed;
        ClockInButton.IsEnabled = ClockOutButton.IsEnabled = loggedIn;

        ErrorText.Text = _engine.Error ?? "";
        ErrorText.Visibility = _engine.Error is null ? Visibility.Collapsed : Visibility.Visible;

        var now = DateTime.Now;
        UserText.Text = _engine.User?.DisplayName ?? "Hayaku";
        StatusText.Text = WorkMath.Phase(_engine.Schedule, now, _engine.State.ClockedOut).Text;

        var running = _engine.RunningKeys();
        var tasks = _engine.Entries
            .GroupBy(e => e.Task, StringComparer.OrdinalIgnoreCase)
            .Select(g => new
            {
                Key = g.Key,
                Title = g.Last().Title,
                Running = running.Contains(g.Key),
                LastEnd = g.Max(e => e.EndedAt ?? DateTimeOffset.MaxValue),
            })
            .ToList();

        _running.Clear();
        foreach (var t in tasks.Where(t => t.Running).OrderBy(t => t.Key, StringComparer.OrdinalIgnoreCase))
            _running.Add(new TaskRowVm { Key = t.Key, Title = t.Title, Running = true, IsClaude = _engine.IsClaudeOwned(t.Key) });
        _paused.Clear();
        foreach (var t in tasks.Where(t => !t.Running).OrderByDescending(t => t.LastEnd))
            _paused.Add(new TaskRowVm { Key = t.Key, Title = t.Title });
        RunningEmpty.Visibility = _running.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        PausedSection.Visibility = _paused.Count == 0 ? Visibility.Collapsed : Visibility.Visible;

        if (!TaskCombo.IsKeyboardFocusWithin && !TaskCombo.IsDropDownOpen)
        {
            var text = TaskCombo.Text;
            TaskCombo.ItemsSource = _engine.MyTasks.Select(t => $"{t.Key}  {t.Title}").ToList();
            TaskCombo.Text = text;
        }

        _repos.Clear();
        foreach (var repo in _engine.Claude.Repos.OrderBy(r => r.Name))
            _repos.Add(RepoRowVm.From(repo, now, _engine.Config.ClaudeIdleMinutes));
        ClaudeSection.Visibility = _repos.Count == 0 ? Visibility.Collapsed : Visibility.Visible;

        UpdateNumbers();
    }

    void UpdateNumbers()
    {
        if (!_engine.LoggedIn)
            return;
        var now = DateTime.Now;
        var today = now.Date;
        var worked = WorkMath.UnionMinutes(_engine.Entries, today, now);
        var expected = _engine.Schedule?.ExpectedMinutes ?? 0;
        TotalText.Text = WorkMath.HoursText(worked);
        ExpectedText.Text = expected > 0 ? $" / {WorkMath.HoursText(expected)}" : "";
        ProgressFill.Width = expected > 0 ? ProgressWidth * Math.Min(1, worked / expected) : 0;
        SummaryDetail.Text = expected > 0 && worked < expected
            ? $"還差 {WorkMath.HoursMinutes(expected - worked)}"
            : WorkMath.HoursMinutes(worked);

        foreach (var vm in _running)
            vm.Elapsed = WorkMath.Clock(WorkMath.TaskMinutes(_engine.Entries, vm.Key, today, now));
        foreach (var vm in _paused)
            vm.Elapsed = WorkMath.Clock(WorkMath.TaskMinutes(_engine.Entries, vm.Key, today, now));
    }

    // ───────────── 動作 ─────────────

    async void TaskAction_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not TaskRowVm vm)
            return;
        if (vm.Running)
            await _engine.PauseTaskAsync(vm.Key);
        else
            await _engine.StartTaskAsync(vm.Key);
    }

    async void StartTask_Click(object sender, RoutedEventArgs e)
    {
        if (KeyFinder.Parse(TaskCombo.Text) is not { } key)
        {
            TaskCombo.Focus();
            return;
        }
        TaskCombo.Text = "";
        await _engine.StartTaskAsync(key);
    }

    async void ClockIn_Click(object sender, RoutedEventArgs e)
    {
        if (_engine.ResumeCandidates().Count > 0)
        {
            await _engine.ClockInAsync();
            return;
        }
        var picker = new TaskPickerWindow(_engine.MyTasks, "上班要開始計時哪些任務？（可多選）", multiSelect: true, showMakeDefault: false);
        if (ShowDialogSafe(picker) && picker.SelectedKeys.Count > 0)
            await _engine.ClockInAsync(picker.SelectedKeys);
    }

    async void ClockOut_Click(object sender, RoutedEventArgs e) => await _engine.ClockOutAsync();

    async void Refresh_Click(object sender, RoutedEventArgs e) => await _engine.RefreshAsync();

    void Settings_Click(object sender, RoutedEventArgs e)
    {
        HidePanel();
        SettingsRequested?.Invoke();
    }

    async void Login_Click(object sender, RoutedEventArgs e) => await _engine.LoginAsync();

    async void RepoPick_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not RepoRowVm vm)
            return;
        var picker = new TaskPickerWindow(_engine.MyTasks, $"「{vm.Name}」的 Claude session 在做哪個任務？", multiSelect: false, showMakeDefault: true);
        if (ShowDialogSafe(picker) && picker.SelectedKeys.Count > 0)
            await _engine.PickTaskForRepoAsync(vm.Path, picker.SelectedKeys[0], picker.MakeDefault);
    }

    bool ShowDialogSafe(Window dialog)
    {
        _dialogOpen = true;
        try
        {
            dialog.Owner = this;
            return dialog.ShowDialog() == true;
        }
        finally
        {
            _dialogOpen = false;
            Activate();
        }
    }
}
