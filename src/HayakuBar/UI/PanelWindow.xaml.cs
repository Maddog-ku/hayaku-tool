using System.Collections.ObjectModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Threading;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.UI;

/// <summary>點工作列上的工時後往上彈出的面板（計時／今天／通知三個分頁）；失去焦點就收起來。</summary>
public partial class PanelWindow : Window
{
    const double ProgressWidth = 90;

    readonly TimerEngine _engine;
    readonly ObservableCollection<TaskRowVm> _running = [];
    readonly ObservableCollection<TaskRowVm> _paused = [];
    readonly ObservableCollection<RepoRowVm> _repos = [];
    readonly DispatcherTimer _tick = new() { Interval = TimeSpan.FromSeconds(1) };
    List<PickItem> _allTasks = [];
    Rect _anchor;
    bool _alignLeft;
    bool _dialogOpen;
    bool _hiding;
    bool _animating;
    bool _renderQueued;

    public event Action? SettingsRequested;
    public event Action? TimeEntriesRequested;

    public PanelWindow(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        RunningList.ItemsSource = _running;
        PausedList.ItemsSource = _paused;
        RepoList.ItemsSource = _repos;
        InitWork();

        _engine.Changed += QueueRender;
        ThemeManager.Changed += QueueRender; // 列表裡的顏色是建立時取的，換主題要重建
        _tick.Tick += (_, _) => UpdateNumbers();
        SizeChanged += (_, _) => Reposition();
        Deactivated += (_, _) =>
        {
            StartPopup.IsOpen = false;
            if (!_dialogOpen && !StayOpen)
                HidePanel();
        };
        PreviewKeyDown += (_, e) =>
        {
            if (e.Key != Key.Escape)
                return;
            if (StartPopup.IsOpen)
                StartPopup.IsOpen = false;
            else
                HidePanel();
            e.Handled = true;
        };
    }

    /// <summary>
    /// 資料變了要重畫：同一輪的多次變動合併成一次；滑出動畫進行中先不重建清單，等動畫結束再畫，避免動畫卡頓。
    /// </summary>
    void QueueRender()
    {
        if (!IsVisible || _renderQueued)
            return;
        _renderQueued = true;
        Dispatcher.InvokeAsync(() =>
        {
            if (_animating)
                return; // 動畫結束時會補畫
            _renderQueued = false;
            if (IsVisible)
                Render();
        }, DispatcherPriority.Background);
    }

    /// <summary>啟動後閒置時先把面板的畫面建好，第一次打開才不會因為建立畫面而卡一下。</summary>
    public void Prewarm()
    {
        if (IsVisible)
            return;
        new System.Windows.Interop.WindowInteropHelper(this).EnsureHandle();
        Render();
        Root.Measure(new Size(Width, double.PositiveInfinity));
        Root.Arrange(new Rect(Root.DesiredSize));
    }

    /// <summary>上一次因為失去焦點而收起的時間；用來避免「點工作列 → 收起 → 又被同一下點擊打開」。</summary>
    public DateTime HiddenAt { get; private set; }

    /// <summary>失去焦點也不收起（--panel 啟動時用，方便檢查畫面）。</summary>
    public bool StayOpen { get; set; }

    /// <summary>面板開著（收起動畫進行中不算）。</summary>
    public bool IsOpen => IsVisible && !_hiding;

    /// <summary>anchor：工作列上工時的位置（DIP）；面板從它正上方往上滑出來。</summary>
    public void ShowAt(Rect anchor, bool alignLeft)
    {
        _anchor = anchor;
        _alignLeft = alignLeft;
        _hiding = false;
        Render();
        Show();
        UpdateLayout();
        Reposition();
        Activate();
        AnimateIn(() =>
        {
            // 動畫結束才開始同步：同步一開始就會觸發重畫，跟動畫搶時間就會卡
            if (IsOpen)
                _ = _engine.RefreshAllAsync(userAction: false);
        });
        _tick.Start();
    }

    /// <summary>直接切到某個分頁：Timer、Today、Notify。</summary>
    public void SelectTab(string tab)
    {
        (tab switch
        {
            "Tasks" => TabTasksButton,
            "Today" => TabTodayButton,
            "Notify" => TabNotifyButton,
            _ => TabTimerButton,
        }).IsChecked = true;
    }

    public void HidePanel()
    {
        if (!IsVisible || _hiding)
            return;
        HiddenAt = DateTime.Now;
        _tick.Stop();
        _animating = false;
        _renderQueued = false;
        StartPopup.IsOpen = false;
        if (!SystemParameters.ClientAreaAnimation)
        {
            Hide();
            return;
        }
        _hiding = true;
        Root.CacheMode = new BitmapCache(VisualTreeHelper.GetDpi(this).DpiScaleX);
        var duration = TimeSpan.FromMilliseconds(150);
        var ease = new CubicEase { EasingMode = EasingMode.EaseIn };
        var slide = new DoubleAnimation(Slide.Y, SlideDistance * 0.5, duration) { EasingFunction = ease };
        var fade = new DoubleAnimation(Root.Opacity, 0, duration);
        fade.Completed += (_, _) =>
        {
            if (!_hiding)
                return; // 收起途中又被打開了
            _hiding = false;
            Root.CacheMode = null;
            Hide();
        };
        Slide.BeginAnimation(TranslateTransform.YProperty, slide);
        Root.BeginAnimation(OpacityProperty, fade);
    }

    void StartSpinner()
    {
        if (SpinnerRotate.HasAnimatedProperties)
            return;
        SpinnerRotate.BeginAnimation(RotateTransform.AngleProperty,
            new DoubleAnimation(0, 360, TimeSpan.FromSeconds(0.9)) { RepeatBehavior = RepeatBehavior.Forever });
    }

    double SlideDistance => Math.Clamp(ActualHeight * 0.45, 40, 220);

    /// <summary>
    /// 從工作列往上滑出＋淡入（Windows 關閉動畫效果時直接出現）。動畫期間把內容快取成點陣圖，
    /// 每一格只要移動圖片，不用重畫整個面板；結束後拿掉快取讓文字維持清晰，再呼叫 completed。
    /// </summary>
    void AnimateIn(Action completed)
    {
        if (!SystemParameters.ClientAreaAnimation)
        {
            Slide.BeginAnimation(TranslateTransform.YProperty, null);
            Root.BeginAnimation(OpacityProperty, null);
            Slide.Y = 0;
            Root.Opacity = 1;
            completed();
            return;
        }
        _animating = true;
        Root.CacheMode = new BitmapCache(VisualTreeHelper.GetDpi(this).DpiScaleX);
        var ease = new CubicEase { EasingMode = EasingMode.EaseOut };
        var slide = new DoubleAnimation(SlideDistance, 0, TimeSpan.FromMilliseconds(260)) { EasingFunction = ease };
        slide.Completed += (_, _) =>
        {
            if (!_animating)
                return; // 中途被收起了
            _animating = false;
            Root.CacheMode = null;
            if (_renderQueued)
            {
                _renderQueued = false;
                Render();
            }
            completed();
        };
        Slide.BeginAnimation(TranslateTransform.YProperty, slide);
        Root.BeginAnimation(OpacityProperty, new DoubleAnimation(0, 1, TimeSpan.FromMilliseconds(170)));
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
        var panelHeight = _engine.Config.PanelHeight;
        ContentScroll.Height = panelHeight > 0 ? panelHeight : double.NaN;
        ContentScroll.MaxHeight = panelHeight > 0 ? panelHeight : 440;

        var loggedIn = _engine.LoggedIn;
        var initialSyncing = _engine.InitialSyncing;
        SyncingPanel.Visibility = initialSyncing ? Visibility.Visible : Visibility.Collapsed;
        LoginButton.Visibility = LoginHint.Visibility = loggedIn || initialSyncing ? Visibility.Collapsed : Visibility.Visible;
        LoginButton.IsEnabled = !_engine.LoggingIn;
        LoginButton.Content = _engine.LoggingIn ? "等待瀏覽器登入…" : "登入 Hayaku";
        MainContent.Visibility = SummaryRow.Visibility = Timeline.Visibility = loggedIn && !initialSyncing ? Visibility.Visible : Visibility.Collapsed;
        ClockInButton.IsEnabled = ClockOutButton.IsEnabled = EntriesButton.IsEnabled = loggedIn && !initialSyncing;
        SyncText.Text = _engine.Syncing ? "同步中…" :
            _engine.AutoSyncPaused ? $"{_engine.LastSynced:HH:mm} 同步 · 暫停自動" :
            _engine.LastSynced is { } synced ? $"{synced:HH:mm} 已同步" : "";
        SyncText.ToolTip = _engine.AutoSyncPaused
            ? "不在上班時間，暫停自動同步（打開面板或按 ⟳ 仍會同步）；可在設定 › 一般關掉"
            : null;
        RefreshButton.IsEnabled = !_engine.Syncing;
        if (initialSyncing)
            StartSpinner();
        else
            SpinnerRotate.BeginAnimation(RotateTransform.AngleProperty, null);

        ErrorText.Text = _engine.Error ?? "";
        ErrorText.Visibility = _engine.Error is null ? Visibility.Collapsed : Visibility.Visible;

        var now = DateTime.Now;
        UserText.Text = _engine.User?.DisplayName ?? "Hayaku";
        StatusText.Text = WorkMath.Phase(_engine.Schedule, now, _engine.State.ClockedOut).Text;

        RenderTimer(now);
        RenderTasks();
        RenderToday(now);
        RenderTodayExtras();
        RenderNotifications();
        UpdateNumbers();
    }

    void RenderTimer(DateTime now)
    {
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
            _running.Add(new TaskRowVm { Key = t.Key, Title = t.Title, Running = true, IsClaude = _engine.IsClaudeOwned(t.Key), Status = _engine.StatusOf(t.Key) });
        _paused.Clear();
        foreach (var t in tasks.Where(t => !t.Running).OrderByDescending(t => t.LastEnd))
            _paused.Add(new TaskRowVm { Key = t.Key, Title = t.Title, Status = _engine.StatusOf(t.Key) });
        RunningEmpty.Visibility = _running.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        PausedSection.Visibility = _paused.Count == 0 ? Visibility.Collapsed : Visibility.Visible;

        _allTasks = _engine.MyTasks.Select(t => new PickItem(t.Key, t.Title, $"{t.Project} · {t.Status}")).ToList();

        // 主資料夾裡可能有很多專案：只列活動中或今天有用過的，都沒有就列主資料夾
        _repos.Clear();
        var units = _engine.Claude.Repos
            .Where(r => r.Active || r.LastActivity.Date == now.Date)
            .OrderByDescending(r => r.Active)
            .ThenByDescending(r => r.LastActivity)
            .ToList();
        if (units.Count == 0)
            units = _engine.Claude.Repos.Where(r => r.RootPath is null).OrderBy(r => r.Name).ToList();
        foreach (var repo in units)
            _repos.Add(RepoRowVm.From(repo, now, _engine.Config.ClaudeIdleMinutes));
        ClaudeSection.Visibility = _repos.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
    }

    void RenderToday(DateTime now)
    {
        var offsetNow = DateTimeOffset.Now;
        var meetings = _engine.Meetings.Select(m => MeetingRowVm.From(m, offsetNow)).ToList();
        MeetingList.ItemsSource = meetings;
        MeetingEmpty.Visibility = meetings.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

        var todos = _engine.Todos.Select(t => new TodoRowVm { Item = t }).ToList();
        TodoList.ItemsSource = todos;
        TodoEmpty.Visibility = todos.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

        var reminders = _engine.Reminders.Where(r => r.At is null || r.At > offsetNow).Take(5).Select(r => new ReminderRowVm { Item = r }).ToList();
        ReminderList.ItemsSource = reminders;
        ReminderEmpty.Visibility = reminders.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

        var pending = _engine.Meetings.Count(m => m.EndAt > offsetNow) + todos.Count;
        TodayBadge.Visibility = pending > 0 ? Visibility.Visible : Visibility.Collapsed;
        TodayBadgeText.Text = pending > 99 ? "99+" : pending.ToString();
    }

    void RenderNotifications()
    {
        var rows = FilteredNotifications().Take(30).Select(n => new NotificationRowVm { Item = n }).ToList();
        NotificationList.ItemsSource = rows;
        NotificationEmpty.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        var unread = _engine.UnreadCount;
        UnreadText.Text = unread > 0 ? $"{unread} 則未讀" : "都已讀";
        MarkAllButton.IsEnabled = unread > 0;
        NotifyBadge.Visibility = unread > 0 ? Visibility.Visible : Visibility.Collapsed;
        NotifyBadgeText.Text = unread > 99 ? "99+" : unread.ToString();
    }

    void UpdateNumbers()
    {
        if (!_engine.LoggedIn)
            return;
        // 上班中只給「下班」，下班後（或還沒上班）只給「上班」；放在每秒更新裡，跨過上下班時間會自己切換
        var onDuty = _engine.OnDuty;
        ClockInButton.Visibility = onDuty ? Visibility.Collapsed : Visibility.Visible;
        ClockOutButton.Visibility = onDuty ? Visibility.Visible : Visibility.Collapsed;

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

        Timeline.SetData(today, _engine.Schedule?.Working, _engine.Entries, now);

        foreach (var vm in _running)
            vm.Elapsed = WorkMath.Clock(WorkMath.TaskMinutes(_engine.Entries, vm.Key, today, now));
        foreach (var vm in _paused)
            vm.Elapsed = WorkMath.Clock(WorkMath.TaskMinutes(_engine.Entries, vm.Key, today, now));
    }

    void Tab_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string tab } || TabTimer is null)
            return;
        TabTimer.Visibility = tab == "Timer" ? Visibility.Visible : Visibility.Collapsed;
        TabTasks.Visibility = tab == "Tasks" ? Visibility.Visible : Visibility.Collapsed;
        TabToday.Visibility = tab == "Today" ? Visibility.Visible : Visibility.Collapsed;
        TabNotify.Visibility = tab == "Notify" ? Visibility.Visible : Visibility.Collapsed;
        StartPopup.IsOpen = false;
    }

    // ───────────── 計時 ─────────────

    async void TaskAction_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not TaskRowVm vm)
            return;
        vm.Busy = true;
        if (vm.Running)
            await _engine.PauseTaskAsync(vm.Key);
        else
            await _engine.StartTaskAsync(vm.Key);
        vm.Busy = false;
    }

    void TaskRow_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is TaskRowVm vm)
            RequestWorkspace("tasks", vm.Key);
    }

    void StartBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        StartHint.Visibility = StartBox.Text.Length == 0 ? Visibility.Visible : Visibility.Collapsed;
        FilterStart();
    }

    void StartBox_GotFocus(object sender, KeyboardFocusChangedEventArgs e) => FilterStart();

    void StartBox_LostFocus(object sender, KeyboardFocusChangedEventArgs e)
    {
        if (!StartList.IsMouseOver && !StartList.IsKeyboardFocusWithin)
            StartPopup.IsOpen = false;
    }

    /// <summary>依輸入篩選我的任務；輸入的編號不在清單裡時，也提供「直接開始」。</summary>
    void FilterStart()
    {
        var q = StartBox.Text.Trim();
        var items = (q.Length == 0
                ? _allTasks
                : _allTasks.Where(t => t.Key.Contains(q, StringComparison.OrdinalIgnoreCase)
                                       || t.Title.Contains(q, StringComparison.OrdinalIgnoreCase)
                                       || t.Detail.Contains(q, StringComparison.OrdinalIgnoreCase)))
            .Take(12)
            .ToList();
        if (q.Length >= 2)
            items.AddRange(RemoteResults(q)
                .Where(r => !items.Any(i => string.Equals(i.Key, r.Key, StringComparison.OrdinalIgnoreCase)))
                .Take(Math.Max(0, 12 - items.Count)));
        QueueRemoteSearch(q, items.Count);
        if (KeyFinder.Parse(q) is { } typed && !items.Any(i => string.Equals(i.Key, typed, StringComparison.OrdinalIgnoreCase)))
            items.Insert(0, new PickItem(typed, "直接開始這個任務", "不在我的任務清單裡"));
        StartList.ItemsSource = items;
        StartList.SelectedIndex = items.Count > 0 ? 0 : -1;
        StartPopup.IsOpen = StartBox.IsKeyboardFocused && items.Count > 0;
    }

    async void StartBox_PreviewKeyDown(object sender, KeyEventArgs e)
    {
        switch (e.Key)
        {
            case Key.Down when StartList.Items.Count > 0:
                StartList.SelectedIndex = Math.Min(StartList.Items.Count - 1, StartList.SelectedIndex + 1);
                StartList.ScrollIntoView(StartList.SelectedItem);
                e.Handled = true;
                break;
            case Key.Up when StartList.Items.Count > 0:
                StartList.SelectedIndex = Math.Max(0, StartList.SelectedIndex - 1);
                StartList.ScrollIntoView(StartList.SelectedItem);
                e.Handled = true;
                break;
            case Key.Enter:
                e.Handled = true;
                await StartSelectedAsync();
                break;
        }
    }

    async void StartList_Click(object sender, MouseButtonEventArgs e)
    {
        if ((e.OriginalSource as FrameworkElement)?.DataContext is PickItem item)
        {
            StartList.SelectedItem = item;
            await StartSelectedAsync();
        }
    }

    async Task StartSelectedAsync()
    {
        var key = (StartList.SelectedItem as PickItem)?.Key ?? KeyFinder.Parse(StartBox.Text);
        if (key is null)
            return;
        StartPopup.IsOpen = false;
        StartBox.Text = "";
        Keyboard.ClearFocus();
        await _engine.StartTaskAsync(key);
    }

    // ───────────── 今天 ─────────────

    void MeetingRow_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is MeetingRowVm vm)
            Web.Open(_engine.MeetingUrl(vm.Item.Id));
    }

    async void JoinMeeting_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not MeetingRowVm vm)
            return;
        e.Handled = true;
        ((Button)sender).IsEnabled = false;
        await _engine.JoinMeetingAsync(vm.Item);
    }

    async void Todo_Checked(object sender, RoutedEventArgs e)
    {
        if ((sender as CheckBox)?.Tag is not TodoRowVm vm)
            return;
        vm.Busy = true;
        await _engine.CompleteTodoAsync(vm.Item);
    }

    void CaptureBox_TextChanged(object sender, TextChangedEventArgs e) =>
        CaptureHint.Visibility = CaptureBox.Text.Length == 0 ? Visibility.Visible : Visibility.Collapsed;

    async void CaptureBox_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        await CaptureAsync();
    }

    async void Capture_Click(object sender, RoutedEventArgs e) => await CaptureAsync();

    async Task CaptureAsync()
    {
        var text = CaptureBox.Text.Trim();
        if (text.Length == 0)
            return;
        CaptureBox.IsEnabled = false;
        await _engine.QuickCaptureAsync(text, CaptureTodo.IsChecked == true);
        CaptureBox.IsEnabled = true;
        if (_engine.Error is null)
            CaptureBox.Text = "";
    }

    // ───────────── 通知 ─────────────

    async void NotificationRow_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is not NotificationRowVm vm)
            return;
        Web.Open(_engine.NotificationUrl(vm.Item));
        if (vm.Item.Unread)
            await _engine.MarkNotificationsReadAsync([vm.Item.Id]);
    }

    async void MarkAll_Click(object sender, RoutedEventArgs e)
    {
        MarkAllButton.IsEnabled = false;
        await _engine.MarkNotificationsReadAsync(null);
    }

    void OpenNotifications_Click(object sender, RoutedEventArgs e) => Web.Open(_engine.WebUrl("/notifications"));

    // ───────────── 底部 ─────────────

    async void ClockIn_Click(object sender, RoutedEventArgs e)
    {
        if (_engine.ResumeCandidates().Count == 0)
        {
            var picker = new TaskPickerWindow(_engine.MyTasks, "上班要開始計時哪些任務？（可多選）", multiSelect: true);
            if (!ShowDialogSafe(picker) || picker.SelectedKeys.Count == 0)
                return;
            await WithBusyButton(ClockInButton, () => _engine.ClockInAsync(picker.SelectedKeys));
            return;
        }
        await WithBusyButton(ClockInButton, () => _engine.ClockInAsync());
    }

    async void ClockOut_Click(object sender, RoutedEventArgs e) =>
        await WithBusyButton(ClockOutButton, _engine.ClockOutAsync);

    static async Task WithBusyButton(Button button, Func<Task> action)
    {
        var content = button.Content;
        button.Content = "處理中…";
        button.IsEnabled = false;
        try
        {
            await action();
        }
        finally
        {
            button.Content = content;
            button.IsEnabled = true;
        }
    }

    void Entries_Click(object sender, RoutedEventArgs e)
    {
        HidePanel();
        TimeEntriesRequested?.Invoke();
    }

    async void Refresh_Click(object sender, RoutedEventArgs e) => await _engine.RefreshAllAsync(userAction: true);

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
        var picker = new TaskPickerWindow(_engine.MyTasks, $"「{vm.Name}」這個 Claude session 在做哪個任務？", multiSelect: false);
        if (ShowDialogSafe(picker) && picker.SelectedKeys.Count > 0)
            await _engine.PickTaskForRepoAsync(vm.Path, picker.SelectedKeys[0]);
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
