using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Threading;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>面板「任務」分頁的一列。</summary>
public sealed class MyTaskRowVm : INotifyPropertyChanged
{
    bool _busy;

    public required TaskSummary Item { get; init; }
    public required bool Running { get; init; }
    public string Key => Item.Key;
    public string Title => Item.Title;
    public string Status => Item.Status;

    public string DueText
    {
        get
        {
            if (Item.PlannedEnd is not { } end)
                return "";
            var days = end.DayNumber - DateOnly.FromDateTime(DateTime.Today).DayNumber;
            return days < 0 ? $"· 逾期 {-days} 天" : days == 0 ? "· 今天到期" : days == 1 ? "· 明天到期" : $"· {end:M/d} 到期";
        }
    }

    public Brush DueBrush
    {
        get
        {
            var days = Item.PlannedEnd is { } end ? end.DayNumber - DateOnly.FromDateTime(DateTime.Today).DayNumber : 99;
            return Brushes2.Get(days < 0 ? "ErrorBrush" : days <= 1 ? "WarnBrush" : "MutedBrush");
        }
    }

    public string ActionGlyph => _busy ? "…" : Running ? "❚❚" : "▶";
    public string ActionTip => Running ? "暫停計時" : "開始計時";
    public Brush ActionBrush => Brushes2.Get(Running ? "RunBrush" : "MutedBrush");
    public bool NotBusy => !_busy;

    public bool Busy
    {
        get => _busy;
        set
        {
            _busy = value;
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(ActionGlyph)));
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(NotBusy)));
        }
    }

    public event PropertyChangedEventHandler? PropertyChanged;
}

/// <summary>面板的任務分頁、任務右鍵選單、通知篩選與稍後／封存、今天分頁的摘要、開始任務的遠端搜尋。</summary>
public partial class PanelWindow
{
    static readonly (string Key, string Label)[] NotifyFilters =
    [
        ("all", "全部"), ("mention", "@提及"), ("assigned", "指派給我"), ("reminder", "提醒"), ("system", "系統"),
    ];

    static readonly HashSet<string> MentionTypes =
        ["MENTIONED", "DOC_MENTIONED", "THREAD_REPLY", "CHANNEL_MESSAGE", "COMMENT_ON_MY_TASK", "CHAT_REACTION"];

    static readonly HashSet<string> AssignedTypes =
        ["TASK_ASSIGNED", "DOC_TODO_ASSIGNED", "WATCHER_ADDED", "ADDED_TO_PROJECT", "MEETING_INVITED", "CHANGE_REQUESTED", "CHANGE_REVIEWED"];

    static readonly HashSet<string> ReminderTypes =
    [
        "TASK_REMINDER", "TASK_DUE_SOON", "TASK_OVERDUE", "MEETING_STARTING", "TIMER_RESUME_PROMPT", "HOURS_UNFILLED",
        "HOURS_UNFILLED_WEEKLY", "MILESTONE_DUE_SOON", "POLL_CLOSING", "DAILY_DIGEST",
    ];

    readonly Dictionary<string, RadioButton> _taskFilters = [];
    readonly Dictionary<string, RadioButton> _notifyFilters = [];
    readonly DispatcherTimer _searchDelay = new() { Interval = TimeSpan.FromMilliseconds(400) };
    readonly Dictionary<string, List<PickItem>> _searchCache = new(StringComparer.OrdinalIgnoreCase);
    string _taskTab = TimerEngine.AllTab;
    string _notifyFilter = "all";

    /// <summary>要開工作台：頁面、參數（任務編號、日期、"new"…）。</summary>
    public event Action<string, object?>? WorkspaceRequested;

    void InitWork()
    {
        foreach (var (tab, label) in TimerEngine.TaskViewTabs)
        {
            var chip = new RadioButton { Style = (Style)FindResource("ChipToggle"), GroupName = "PanelTaskTab", Tag = tab, Content = label, Margin = new Thickness(0, 0, 5, 4) };
            chip.Checked += (_, _) =>
            {
                _taskTab = tab;
                RenderTasks();
            };
            _taskFilters[tab] = chip;
            TaskFilterPanel.Children.Add(chip);
        }
        _taskFilters[_taskTab].IsChecked = true;

        foreach (var (key, label) in NotifyFilters)
        {
            var chip = new RadioButton { Style = (Style)FindResource("ChipToggle"), GroupName = "PanelNotifyTab", Tag = key, Content = label, Margin = new Thickness(0, 0, 5, 4) };
            chip.Checked += (_, _) =>
            {
                _notifyFilter = key;
                RenderNotifications();
            };
            _notifyFilters[key] = chip;
            NotifyFilterPanel.Children.Add(chip);
        }
        _notifyFilters[_notifyFilter].IsChecked = true;

        _searchDelay.Tick += async (_, _) =>
        {
            _searchDelay.Stop();
            await SearchRemoteAsync(StartBox.Text.Trim());
        };
    }

    /// <summary>定期清暫存：任務搜尋的快取。</summary>
    public void TrimCaches() => _searchCache.Clear();

    void RequestWorkspace(string page, object? arg = null)
    {
        HidePanel();
        WorkspaceRequested?.Invoke(page, arg);
    }

    // ───────────── 任務分頁 ─────────────

    void RenderTasks()
    {
        if (MyTaskList is null)
            return;
        var running = _engine.RunningKeys();
        foreach (var (tab, label) in TimerEngine.TaskViewTabs)
        {
            var count = _engine.TaskList(tab).Count;
            _taskFilters[tab].Content = count > 0 ? $"{label} {count}" : label;
        }
        var all = _engine.TaskList(_taskTab);
        var rows = all.Take(30).Select(t => new MyTaskRowVm { Item = t, Running = running.Contains(t.Key) || t.Timing }).ToList();
        MyTaskList.ItemsSource = rows;
        MyTaskEmpty.Text = _engine.TaskLists.Count == 0 ? "讀取中…" : _taskTab switch
        {
            "overdue" => "沒有逾期的任務 🎉",
            "should_start" => "沒有該開始還沒開始的任務",
            _ => "這裡沒有任務",
        };
        MyTaskEmpty.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        MoreTasksButton.Content = all.Count > rows.Count ? $"在工作台看全部 {all.Count} 個 ›" : "在工作台看全部 ›";

        var overdue = _engine.OverdueCount;
        TasksBadge.Visibility = overdue > 0 ? Visibility.Visible : Visibility.Collapsed;
        TasksBadgeText.Text = overdue > 99 ? "99+" : overdue.ToString();
    }

    void MyTaskRow_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is MyTaskRowVm vm)
            RequestWorkspace("tasks", vm.Key);
    }

    async void MyTaskAction_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not MyTaskRowVm vm)
            return;
        e.Handled = true;
        vm.Busy = true;
        if (vm.Running)
            await _engine.PauseTaskAsync(vm.Key);
        else
            await _engine.StartTaskAsync(vm.Key);
    }

    void AllTasks_Click(object sender, RoutedEventArgs e) => RequestWorkspace("tasks", _taskTab);

    void NewTask_Click(object sender, RoutedEventArgs e) => RequestWorkspace("tasks", "new");

    // ───────────── 任務選單（右鍵或 ⋯） ─────────────

    void TaskRow_RightClick(object sender, MouseButtonEventArgs e)
    {
        if (sender is FrameworkElement element && KeyOf(element.Tag) is { } key)
        {
            e.Handled = true;
            ShowTaskMenu(key, MenuAnchor(element));
        }
    }

    void TaskMore_Click(object sender, RoutedEventArgs e)
    {
        if (sender is Button button && KeyOf(button.Tag) is { } key)
        {
            e.Handled = true;
            ShowTaskMenu(key, MenuAnchor(button));
        }
    }

    void StatusPill_Click(object sender, MouseButtonEventArgs e)
    {
        if (sender is FrameworkElement element && KeyOf(element.Tag) is { } key)
        {
            e.Handled = true; // 不要再當成點整列（開詳細）
            _ = ShowStatusMenuAsync(key, MenuAnchor(element));
        }
    }

    static string? KeyOf(object? tag) => tag switch
    {
        TaskRowVm t => t.Key,
        MyTaskRowVm m => m.Key,
        _ => null,
    };

    /// <summary>按鈕在面板裡的位置（開選單當下記下來；之後清單重畫、按鈕換掉了也不影響）。</summary>
    Rect MenuAnchor(FrameworkElement element) =>
        element.IsDescendantOf(Root)
            ? element.TransformToAncestor(Root).TransformBounds(new Rect(element.RenderSize))
            : new Rect(Mouse.GetPosition(Root), new Size(1, 1));

    /// <summary>
    /// 選單擺在按鈕右側（螢幕右邊放不下改放左側），不會蓋住其他列的 ⋯ 與狀態；
    /// 位置以面板為基準，資料同步重畫清單時選單不會跳走。選單開著時直接點別列的 ⋯ 或狀態，會換成那一列的選單。
    /// </summary>
    void OpenMenu(ContextMenu menu, Rect anchor, string id)
    {
        menu.PlacementTarget = Root;
        menu.Placement = PlacementMode.Custom;
        menu.CustomPopupPlacementCallback = (popup, target, _) =>
        {
            var scale = Root.ActualWidth > 0 ? target.Width / Root.ActualWidth : 1; // 回呼給的大小是實體像素
            var top = anchor.Top * scale;
            return
            [
                new CustomPopupPlacement(new Point(anchor.Right * scale + 4 * scale, top), PopupPrimaryAxis.Horizontal),
                new CustomPopupPlacement(new Point(anchor.Left * scale - popup.Width - 4 * scale, top), PopupPrimaryAxis.Horizontal),
            ];
        };
        menu.Closed += (_, _) =>
        {
            // 點選單外面時，那一下會被選單吃掉（只用來關選單）：點的是別列的 ⋯ 或狀態就直接幫他打開
            if (Mouse.LeftButton != MouseButtonState.Pressed || !IsVisible)
                return;
            var point = Mouse.GetPosition(Root);
            Dispatcher.BeginInvoke(() => ReopenAt(point, id), DispatcherPriority.Input);
        };
        menu.IsOpen = true;
    }

    /// <summary>previousId：剛關掉的選單（"menu:F2P-5"、"status:F2P-5"）；再點同一個是想關掉，不重開。</summary>
    void ReopenAt(Point point, string previousId)
    {
        if (Root.InputHitTest(point) is not DependencyObject hit)
            return;
        for (var d = hit; d is not null && d != Root; d = VisualTreeHelper.GetParent(d))
        {
            if (d is not FrameworkElement { Name: "More" or "StatusPill" } element || KeyOf(element.Tag) is not { } key)
                continue;
            var status = element.Name == "StatusPill";
            if ((status ? "status:" : "menu:") + key == previousId)
                return;
            if (status)
                _ = ShowStatusMenuAsync(key, MenuAnchor(element));
            else
                ShowTaskMenu(key, MenuAnchor(element));
            return;
        }
    }

    void ShowTaskMenu(string key, Rect anchor)
    {
        var running = _engine.RunningKeys().Contains(key);
        var status = _engine.StatusOf(key);
        var menu = new ContextMenu();
        void Add(string header, Action action, bool enabled = true)
        {
            var item = new MenuItem { Header = header, IsEnabled = enabled };
            item.Click += (_, _) => action();
            menu.Items.Add(item);
        }

        Add(running ? "❚❚  暫停計時" : "▶  開始計時", () => _ = running ? _engine.PauseTaskAsync(key) : _engine.StartTaskAsync(key));
        Add("✓  停止並完成", () => _ = _engine.CompleteTaskAsync(key));
        Add(status is null ? "⇄  改狀態…" : $"⇄  改狀態…（{status}）", () => _ = ShowStatusMenuAsync(key, anchor));
        menu.Items.Add(new Separator { Margin = new Thickness(4), Background = Brushes2.Get("LineBrush") });
        Add("⏰  新增提醒…", () => RequestWorkspace("reminders", key));
        Add("💬  任務聊天", () => RequestWorkspace("chat", key));
        Add("☰  查看詳細", () => RequestWorkspace("tasks", key));
        Add("↗  在網頁開啟", () => Web.Open(_engine.TaskUrl(key)));
        OpenMenu(menu, anchor, "menu:" + key);
    }

    async Task ShowStatusMenuAsync(string key, Rect anchor)
    {
        ProjectDetail project;
        try
        {
            project = await _engine.GetProjectAsync(KeyFinder.ProjectOf(key));
        }
        catch (Exception ex)
        {
            Log.Error("讀取狀態清單失敗", ex);
            return;
        }
        var current = _engine.StatusOf(key);
        var menu = new ContextMenu();
        menu.Items.Add(new MenuItem { Header = $"{key} 改成…", IsEnabled = false });
        foreach (var status in project.Statuses)
        {
            var item = new MenuItem { Header = (status.Name == current ? "● " : "    ") + status.Name };
            item.Click += (_, _) => _ = _engine.SetTaskStatusAsync(key, status.Name);
            menu.Items.Add(item);
        }
        OpenMenu(menu, anchor, "status:" + key);
    }

    // ───────────── 今天分頁的摘要 ─────────────

    void RenderTodayExtras()
    {
        var due = _engine.DueTodayCount;
        var overdue = _engine.OverdueCount;
        DueText.Inlines.Clear();
        if (due == 0 && overdue == 0)
        {
            DueText.Inlines.Add(new System.Windows.Documents.Run("今天沒有到期或逾期的任務") { Foreground = Brushes2.Get("MutedBrush") });
        }
        else
        {
            DueText.Inlines.Add(new System.Windows.Documents.Run($"今天到期 {due}") { Foreground = Brushes2.Get(due > 0 ? "WarnBrush" : "MutedBrush") });
            DueText.Inlines.Add(new System.Windows.Documents.Run("　·　") { Foreground = Brushes2.Get("MutedBrush") });
            DueText.Inlines.Add(new System.Windows.Documents.Run($"逾期 {overdue}") { Foreground = Brushes2.Get(overdue > 0 ? "ErrorBrush" : "MutedBrush") });
        }

        var leaves = _engine.OnLeaveToday.ToList();
        LeaveText.Text = leaves.Count == 0 ? "" :
            "今天請假：" + string.Join("、", leaves.Select(m => m.Leaves.Count > 0 ? $"{m.User.DisplayName}（{string.Join("、", m.Leaves)}）" : m.User.DisplayName));
        LeaveText.Visibility = leaves.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
    }

    void DueCard_Click(object sender, MouseButtonEventArgs e) =>
        RequestWorkspace("tasks", _engine.OverdueCount > 0 ? "overdue" : "in_progress");

    void NewMeeting_Click(object sender, RoutedEventArgs e) => RequestWorkspace("meetings", "new");

    void NewReminder_Click(object sender, RoutedEventArgs e) => RequestWorkspace("reminders");

    void WorkLog_Click(object sender, RoutedEventArgs e) =>
        RequestWorkspace("worklog", DateOnly.FromDateTime(DateTime.Today).AddDays(-1));

    void Workspace_Click(object sender, RoutedEventArgs e) => RequestWorkspace("tasks");

    // ───────────── 通知篩選、稍後、封存 ─────────────

    static string CategoryOf(NotificationItem n) =>
        MentionTypes.Contains(n.Type) ? "mention" :
        AssignedTypes.Contains(n.Type) ? "assigned" :
        ReminderTypes.Contains(n.Type) ? "reminder" : "system";

    IEnumerable<NotificationItem> FilteredNotifications()
    {
        foreach (var (key, label) in NotifyFilters)
        {
            if (key == "all")
                continue;
            var unread = _engine.Notifications.Count(n => n.Unread && CategoryOf(n) == key);
            _notifyFilters[key].Content = unread > 0 ? $"{label} {unread}" : label;
        }
        return _notifyFilter == "all" ? _engine.Notifications : _engine.Notifications.Where(n => CategoryOf(n) == _notifyFilter);
    }

    void NotificationSnooze_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not NotificationRowVm vm)
            return;
        e.Handled = true;
        var menu = new ContextMenu { PlacementTarget = (Button)sender, Placement = PlacementMode.Bottom };
        foreach (var (preset, label) in new[] { ("1h", "1 小時後"), ("next_workday", "下個工作日上班時"), ("next_week", "下週第一個上班日") })
        {
            var item = new MenuItem { Header = label };
            item.Click += (_, _) => _ = _engine.SnoozeNotificationAsync(vm.Item, preset);
            menu.Items.Add(item);
        }
        menu.IsOpen = true;
    }

    async void NotificationArchive_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not NotificationRowVm vm)
            return;
        e.Handled = true;
        ((Button)sender).IsEnabled = false;
        await _engine.ArchiveNotificationAsync(vm.Item);
    }

    // ───────────── 開始任務：遠端搜尋 ─────────────

    /// <summary>本機清單找不到幾筆時，停頓一下就去 Hayaku 搜尋（任務編號或名稱）。</summary>
    void QueueRemoteSearch(string q, int localCount)
    {
        _searchDelay.Stop();
        if (q.Length < 2 || localCount >= 6 || _searchCache.ContainsKey(q))
            return;
        _searchDelay.Start();
    }

    async Task SearchRemoteAsync(string q)
    {
        if (q.Length < 2 || _searchCache.ContainsKey(q))
            return;
        try
        {
            var hits = await _engine.Api.SearchTasksAsync(q, CancellationToken.None);
            _searchCache[q] = hits.Select(h => new PickItem(h.Key, h.Title, $"{h.Project} · 搜尋結果")).ToList();
        }
        catch (Exception ex)
        {
            Log.Error("搜尋任務失敗", ex);
            _searchCache[q] = [];
        }
        if (StartBox.Text.Trim() == q && StartBox.IsKeyboardFocused)
            FilterStart();
    }

    List<PickItem> RemoteResults(string q) => _searchCache.GetValueOrDefault(q) ?? [];
}
