using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Animation;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

public sealed class TaskListRowVm : System.ComponentModel.INotifyPropertyChanged
{
    string _elapsed = "";
    object? _projectBadge;

    public required string Key { get; init; }
    public required string Title { get; init; }
    public string Context { get; init; } = "";
    public string Status { get; init; } = "";
    public DateOnly? PlannedStart { get; init; }
    public DateOnly? PlannedEnd { get; init; }
    public bool Timing { get; init; }
    public bool Done { get; init; }
    public string Priority { get; init; } = "";

    /// <summary>專案欄的文字（名稱，或設定成只顯示代碼）；Project 是用來找圖示的專案。</summary>
    public string ProjectText { get; init; } = "";
    public ProjectInfo? Project { get; init; }

    public Visibility StatusVisibility => Status.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    public Visibility ContextVisibility => Context.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    public string PriorityText => Priority is "" or "無" ? "" : Priority;
    public string StartText => PlannedStart is { } s ? Inputs.ShortDate(s) : PlannedEnd is not null ? "未指定" : "";
    public Brush StartBrush => Brushes2.Get(PlannedStart is null ? "HintBrush" : "MutedBrush");

    /// <summary>專案圖示＋文字；每列各自一份（同一個元素不能放在兩個地方）。</summary>
    public object ProjectBadge => _projectBadge ??= ProjectText.Length == 0
        ? new TextBlock()
        : ProjectIcons.Badge(Project, ProjectText, 16);

    /// <summary>今天計時（計時中的每秒更新）。</summary>
    public string Elapsed
    {
        get => _elapsed;
        set
        {
            if (_elapsed == value)
                return;
            _elapsed = value;
            PropertyChanged?.Invoke(this, new(nameof(Elapsed)));
        }
    }

    public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;
    public string TimerGlyph => Timing ? "❚❚" : "▶";
    public string TimerTip => Timing ? "暫停計時" : "開始計時";
    public Brush TimerBrush => Brushes2.Get(Timing ? "RunBrush" : "MutedBrush");

    public string DueText
    {
        get
        {
            if (PlannedEnd is not { } end)
                return PlannedStart is not null ? "結束未指定" : "";
            var today = DateOnly.FromDateTime(DateTime.Today);
            var days = end.DayNumber - today.DayNumber;
            return Done ? Inputs.ShortDate(end) :
                days < 0 ? $"逾期 {-days} 天" : days == 0 ? "今天到期" : days == 1 ? "明天到期" : $"{Inputs.ShortDate(end)} 到期";
        }
    }

    public Brush DueBrush
    {
        get
        {
            if (Done || PlannedEnd is not { } end)
                return Brushes2.Get("MutedBrush");
            var days = end.DayNumber - DateOnly.FromDateTime(DateTime.Today).DayNumber;
            return Brushes2.Get(days < 0 ? "ErrorBrush" : days <= 1 ? "WarnBrush" : "MutedBrush");
        }
    }
}

public sealed record ChatRowVm(string Author, string Time, string Body);

/// <summary>我的任務：分頁清單、搜尋、詳細（直接改狀態、日期、工時）、子任務、聊天、新增任務。</summary>
public partial class TasksPage : UserControl, IWorkPage
{
    const string SearchTab = "search";

    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;
    readonly Dictionary<string, RadioButton> _tabs = [];
    string _tab = TimerEngine.AllTab;
    string _view = "list"; // list、tree、kanban
    string? _project; // 只看這個專案（名稱）；null＝全部
    List<TaskSummary> _doneTasks = [];
    DateTime _doneFetched = DateTime.MinValue;
    bool _loadingDone;
    readonly System.Windows.Threading.DispatcherTimer _kanbanTick = new() { Interval = TimeSpan.FromSeconds(1) };
    List<KanbanCardVm> _runningCards = [];
    List<MyTreeProjectVm> _treeRoots = [];
    readonly HashSet<string> _collapsed = new(StringComparer.OrdinalIgnoreCase); // 樹狀檢視裡收合的專案（p:名稱）與任務
    readonly StatusFilter _statusFilter = new(); // 都沒選：全部狀態
    bool _buildingFilters;
    List<SearchHit> _searchHits = [];
    TaskDetail? _detail;
    string? _creatingChildOf;
    string? _pickedParent; // 新任務的上層，null＝最上層
    string? _lastCreated;
    bool _keepParentOnProjectChange;
    int _treeVersion;
    List<ProjectTask> _projectTasks = [];
    List<TreeNodeVm> _treeNodes = [];
    bool _loadingDetail;
    int _detailVersion;

    public TasksPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        InitializeComponent();
        foreach (var (tab, label) in TimerEngine.TaskViewTabs.Append((SearchTab, "搜尋結果")))
        {
            var button = new RadioButton
            {
                Style = (Style)FindResource("Segment"),
                GroupName = "TaskTab",
                Tag = tab,
                Content = label,
                Visibility = tab == SearchTab ? Visibility.Collapsed : Visibility.Visible,
            };
            button.Checked += (_, _) =>
            {
                _tab = tab;
                RenderList();
            };
            _tabs[tab] = button;
            TabPanel.Children.Add(button);
        }
        _tabs[_tab].IsChecked = true;

        _buildingFilters = true;
        SortBox.ItemsSource = TaskOrder.Sorts.Select(s => s.Label).ToList();
        var sortIndex = TaskOrder.Sorts.ToList().FindIndex(s => s.Key == _engine.Config.TaskSort);
        SortBox.SelectedIndex = Math.Max(0, sortIndex);
        _view = _engine.Config.TaskView is "tree" or "kanban" ? _engine.Config.TaskView : "list";
        (_view switch { "tree" => TreeViewButton, "kanban" => KanbanViewButton, _ => ListViewButton }).IsChecked = true;
        _project = string.IsNullOrEmpty(_engine.Config.TaskProject) ? null : _engine.Config.TaskProject;
        LeafOnlyBox.IsChecked = _engine.Config.TaskLeafOnly;
        HideDoneBox.IsChecked = _engine.Config.TaskHideDone;
        TreeHideDoneBox.IsChecked = _engine.Config.TreeHideDone;
        InitColumns();
        _kanbanTick.Tick += (_, _) => UpdateElapsed();
        IsVisibleChanged += (_, _) => UpdateKanbanTick();
        _buildingFilters = false;

        PreviewKeyDown += (_, e) =>
        {
            if (e.Key == Key.Escape && Overlay.Visibility == Visibility.Visible)
            {
                CloseDialog();
                e.Handled = true;
            }
        };
        Loaded += (_, _) => _engine.Changed += OnEngineChanged;
        Unloaded += (_, _) => _engine.Changed -= OnEngineChanged;
    }

    void OnEngineChanged()
    {
        if (IsVisible)
            RenderList(keepSelection: true);
    }

    /// <summary>arg：任務編號（直接打開那個任務）、"new"（新增）、"new:F2P-5"（新增 F2P-5 的子任務）、分頁名稱（overdue…）或顯示方式（list、tree、kanban）。</summary>
    public async Task ActivateAsync(object? arg)
    {
        RenderList(keepSelection: true);
        switch (arg)
        {
            case "new":
                ShowCreate(null);
                break;
            case string s when s.StartsWith("new:", StringComparison.OrdinalIgnoreCase) && KeyFinder.Parse(s[4..]) is { } parent:
                ShowCreate(parent);
                break;
            case "list" or "tree" or "kanban":
                (arg switch { "tree" => TreeViewButton, "kanban" => KanbanViewButton, _ => ListViewButton }).IsChecked = true;
                break;
            case string tab when _tabs.ContainsKey(tab):
                _tabs[tab].IsChecked = true;
                break;
            case string key when KeyFinder.Parse(key) is { } k:
                await OpenTaskAsync(k);
                break;
        }
    }

    // ───────────── 清單 ─────────────

    void RenderList(bool keepSelection = false)
    {
        var running = _engine.RunningKeys();
        var leafOnly = LeafOnlyBox.IsChecked == true;
        foreach (var (tab, label) in TimerEngine.TaskViewTabs)
        {
            var count = _engine.TaskList(tab).Count(t => !leafOnly || t.IsLeaf);
            _tabs[tab].Content = count > 0 ? $"{label} {count}" : label;
        }
        List<TaskListRowVm> rows;
        List<TaskSummary> shown = [];
        var sort = TaskOrder.Sorts[Math.Max(0, SortBox.SelectedIndex)].Key;
        if (_tab == SearchTab)
        {
            RenderStatusFilters([]);
            var hits = sort switch
            {
                "key" => _searchHits.OrderBy(h => KeyFinder.ProjectOf(h.Key), StringComparer.OrdinalIgnoreCase)
                    .ThenBy(h => int.TryParse(h.Key[(h.Key.LastIndexOf('-') + 1)..], out var n) ? n : int.MaxValue).ToList(),
                "timing" => _searchHits.OrderByDescending(h => running.Contains(h.Key)).ToList(),
                _ => _searchHits,
            };
            rows = hits.Select(h => new TaskListRowVm
            {
                Key = h.Key,
                Title = h.Title,
                ProjectText = ProjectText(h.Project, h.Key),
                Project = _engine.ProjectOf(h.Project) ?? _engine.ProjectOf(KeyFinder.ProjectOf(h.Key)),
                Timing = running.Contains(h.Key),
            }).ToList();
            ListMessage.Text = rows.Count == 0 ? "找不到符合的任務" : "";
        }
        else
        {
            // 看板看全部（待處理＋進行中＋已完成），其他照分頁；再套用專案、狀態、只看沒有子任務的篩選
            var hideDone = HideDoneBox.IsChecked == true;
            IReadOnlyList<TaskSummary> all = Kanban_On
                ? _engine.TaskLists.Values.SelectMany(l => l).Concat(hideDone ? [] : _doneTasks)
                    .DistinctBy(t => t.Key, StringComparer.OrdinalIgnoreCase).ToList()
                : _engine.TaskList(_tab);
            RenderProjectBox(_engine.TaskLists.Values.SelectMany(l => l).Concat(_doneTasks));
            var tasks = all.Where(t => (_project is null || t.Project == _project) && (!leafOnly || t.IsLeaf)).ToList();
            RenderStatusFilters(tasks);
            shown = TaskOrder.Sort(tasks.Where(_statusFilter.Matches), sort, running, t => _engine.StatusRank(t.Key, t.Status));
            if (sort == "status")
                _ = LoadStatusOrderAsync(shown.Select(t => t.Key));
            var projectColumn = ColumnVisible("project");
            rows = shown.Select(t => new TaskListRowVm
            {
                Key = t.Key,
                Title = t.Title,
                Context = _view == "list" && projectColumn ? t.Path : t.Path.Length > 0 ? $"{t.Project} › {t.Path}" : t.Project,
                Status = t.Status,
                PlannedStart = t.PlannedStart,
                PlannedEnd = t.PlannedEnd,
                Priority = t.Priority,
                ProjectText = ProjectText(t.Project, t.Key),
                Project = _engine.ProjectOf(t.Project) ?? _engine.ProjectOf(KeyFinder.ProjectOf(t.Key)),
                Timing = running.Contains(t.Key) || t.Timing,
                Done = t.StatusCategory is "DONE" or "CANCELLED",
            }).ToList();
            ListMessage.Text = rows.Count > 0 ? "" : _engine.TaskLists.Count == 0 ? "讀取中…" : !_statusFilter.IsEmpty
                ? "沒有符合狀態篩選的任務" : leafOnly && all.Count > 0 ? "沒有「沒有子任務」的任務（取消勾選看上層任務）" : _tab switch
            {
                "overdue" => "沒有逾期的任務 🎉",
                "should_start" => "沒有該開始還沒開始的任務",
                _ => "這裡沒有任務",
            };
        }
        var selected = (TaskList.SelectedItem as TaskListRowVm)?.Key ?? _detail?.Key;
        _listRows = rows;
        UpdateRowElapsed(DateTime.Now);
        TaskList.ItemsSource = rows;
        if (keepSelection && selected is not null)
        {
            _loadingDetail = true;
            TaskList.SelectedItem = rows.FirstOrDefault(r => r.Key == selected);
            _loadingDetail = false;
        }

        // 樹狀、看板：搜尋結果沒有上下層與狀態類別，照樣用清單
        var tree = _view == "tree" && _tab != SearchTab;
        var kanban = Kanban_On;
        TaskList.Visibility = tree || kanban ? Visibility.Collapsed : Visibility.Visible;
        ColumnHeader.Visibility = TaskList.Visibility;
        ColumnsButton.Visibility = TaskList.Visibility;
        CollapseAllButton.Visibility = tree ? Visibility.Visible : Visibility.Collapsed;
        HideDoneBox.Visibility = kanban ? Visibility.Visible : Visibility.Collapsed;
        TreeList.Visibility = tree ? Visibility.Visible : Visibility.Collapsed;
        Kanban.Visibility = kanban ? Visibility.Visible : Visibility.Collapsed;
        TabTrack.Visibility = kanban ? Visibility.Collapsed : Visibility.Visible; // 看板不分分頁
        SearchBox.HorizontalAlignment = kanban ? HorizontalAlignment.Right : HorizontalAlignment.Stretch;
        ListCard.Background = kanban ? Brushes.Transparent : Brushes2.Get("BgBrush");
        ListCard.BorderThickness = new Thickness(kanban ? 0 : 1);
        if (tree)
            RenderTree(shown, rows);
        else
            _treeRows.Clear();
        if (kanban)
            RenderKanban(shown, rows);
        else
            TodoColumn.ItemsSource = DoingColumn.ItemsSource = DoneColumn.ItemsSource = null;
        UpdateFilterHighlights();
        UpdateKanbanTick();
    }

    /// <summary>專案欄的文字：名稱，或設定成只顯示代碼。</summary>
    string ProjectText(string projectName, string key) =>
        _engine.Config.TaskProjectCodeOnly ? KeyFinder.ProjectOf(key) : projectName;

    bool _loadingStatusOrder;

    /// <summary>依狀態排序要用專案設定的狀態順序：還沒讀過的專案讀完再重排一次。</summary>
    async Task LoadStatusOrderAsync(IEnumerable<string> keys)
    {
        if (_loadingStatusOrder)
            return;
        _loadingStatusOrder = true;
        try
        {
            if (await _engine.LoadProjectsOfAsync(keys.ToList()))
                RenderList(keepSelection: true);
        }
        finally
        {
            _loadingStatusOrder = false;
        }
    }

    /// <summary>看板模式（搜尋時除外）。</summary>
    bool Kanban_On => _view == "kanban" && _tab != SearchTab;

    // ───────────── 看板 ─────────────

    void RenderKanban(List<TaskSummary> shown, List<TaskListRowVm> rows)
    {
        var rowByKey = rows.ToDictionary(r => r.Key, StringComparer.OrdinalIgnoreCase);
        var today = DateTime.Today;
        var now = DateTime.Now;
        var cards = shown.Select(t => new KanbanCardVm(t, rowByKey[t.Key])).ToList();
        List<KanbanCardVm> Column(params string[] categories) =>
            cards.Where(c => categories.Contains(c.Task.StatusCategory)).ToList();
        var todo = cards.Where(c => c.Task.StatusCategory is not ("IN_PROGRESS" or "DONE" or "CANCELLED")).ToList();
        var doing = Column("IN_PROGRESS");
        var done = Column("DONE", "CANCELLED");
        var hideDone = HideDoneBox.IsChecked == true;
        DoneGapColumn.Width = new GridLength(hideDone ? 0 : 12);
        DoneColumnDef.Width = hideDone ? new GridLength(0) : new GridLength(1, GridUnitType.Star);
        TodoColumn.ItemsSource = todo;
        DoingColumn.ItemsSource = doing;
        DoneColumn.ItemsSource = done;
        TodoCount.Text = todo.Count.ToString();
        DoingCount.Text = doing.Count.ToString();
        DoneCount.Text = _loadingDone && _doneTasks.Count == 0 ? "…" : done.Count.ToString();
        _runningCards = cards.Where(c => c.Running).ToList();
        foreach (var c in _runningCards)
            c.Elapsed = WorkMath.Clock(WorkMath.TaskMinutes(_engine.Entries, c.Key, today, now));
        if (shown.Count == 0)
            ListMessage.Text = _project is not null ? $"「{_project}」沒有指派給你的任務" : ListMessage.Text;
        if (!hideDone && now - _doneFetched > TimeSpan.FromMinutes(5))
            _ = LoadDoneAsync();
    }

    /// <summary>已完成欄：my_tasks 的 DONE（最多 3 頁），5 分鐘內不重讀。</summary>
    async Task LoadDoneAsync()
    {
        if (_loadingDone)
            return;
        _loadingDone = true;
        _doneFetched = DateTime.Now;
        try
        {
            _doneTasks = await _engine.Api.MyTasksByCategoryAsync("DONE", 3, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error("讀取已完成的任務失敗", ex);
        }
        finally
        {
            _loadingDone = false;
        }
        RenderList(keepSelection: true);
    }

    /// <summary>看板的計時中卡片、清單的「今天計時」欄：有計時中的才每秒更新。</summary>
    void UpdateKanbanTick()
    {
        var listTicking = TaskList.Visibility == Visibility.Visible && ColumnVisible("elapsed") && _listRows.Any(r => r.Timing);
        if (IsVisible && (Kanban_On && _runningCards.Count > 0 || listTicking))
            _kanbanTick.Start();
        else
            _kanbanTick.Stop();
    }

    void UpdateElapsed()
    {
        var now = DateTime.Now;
        foreach (var c in _runningCards)
            c.Elapsed = WorkMath.Clock(WorkMath.TaskMinutes(_engine.Entries, c.Key, now.Date, now));
        UpdateRowElapsed(now);
    }

    List<TaskListRowVm> _listRows = [];

    void UpdateRowElapsed(DateTime now)
    {
        if (!ColumnVisible("elapsed"))
            return;
        foreach (var r in _listRows)
        {
            var minutes = WorkMath.TaskMinutes(_engine.Entries, r.Key, now.Date, now);
            r.Elapsed = minutes > 0 ? WorkMath.Clock(minutes) : "";
        }
    }

    async void KanbanCard_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is KanbanCardVm card)
            await OpenTaskAsync(card.Key);
    }

    // ───────────── 專案篩選 ─────────────

    List<string> _projectNames = [];
    bool _projectIconsShown;

    /// <summary>
    /// 專案下拉：「全部專案」＋指派給我的任務出現過的專案（圖示＋名稱）；選的專案不見了就回到全部。
    /// </summary>
    void RenderProjectBox(IEnumerable<TaskSummary> tasks)
    {
        var names = tasks.Select(t => t.Project).Where(p => p.Length > 0).Distinct()
            .OrderBy(p => p, StringComparer.CurrentCulture).ToList();
        if (_project is not null && !names.Contains(_project) && _engine.TaskLists.Count > 0)
            _project = null;
        var index = _project is null ? 0 : names.IndexOf(_project) + 1;
        var iconsReady = _engine.Projects.Count > 0;
        var same = names.SequenceEqual(_projectNames) && iconsReady == _projectIconsShown && ProjectBox.Items.Count > 0;
        if (same && ProjectBox.SelectedIndex == index)
            return;
        _buildingFilters = true;
        if (!same)
        {
            _projectNames = names;
            _projectIconsShown = iconsReady;
            var items = new List<object> { new ComboBoxItem { Content = "全部專案", Tag = null } };
            items.AddRange(names.Select(n => new ComboBoxItem { Content = ProjectIcons.Badge(_engine.ProjectOf(n), n, 16), Tag = n }));
            ProjectBox.ItemsSource = items;
        }
        ProjectBox.SelectedIndex = index;
        _buildingFilters = false;
    }

    void ProjectBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_buildingFilters || ProjectBox.SelectedItem is not ComboBoxItem item)
            return;
        _project = item.Tag as string;
        _engine.Config.TaskProject = _project ?? "";
        _engine.Config.Save();
        RenderList(keepSelection: true);
    }

    void LeafOnly_Click(object sender, RoutedEventArgs e)
    {
        _engine.Config.TaskLeafOnly = LeafOnlyBox.IsChecked == true;
        _engine.Config.Save();
        RenderList(keepSelection: true);
    }

    void HideDone_Click(object sender, RoutedEventArgs e)
    {
        _engine.Config.TaskHideDone = HideDoneBox.IsChecked == true;
        _engine.Config.Save();
        RenderList(keepSelection: true);
    }

    /// <summary>有套用條件的篩選控制項亮起來，一眼看得出清單被篩過；多選寫出選了幾個。</summary>
    void UpdateFilterHighlights()
    {
        Highlight(StatusFilterButton, !_statusFilter.IsEmpty);
        Highlight(ProjectBox, _project is not null);
        LeafOnlyBox.Foreground = Brushes2.Get(LeafOnlyBox.IsChecked == true ? "AccentBrush" : "TextBrush");
        HideDoneBox.Foreground = Brushes2.Get(HideDoneBox.IsChecked == true ? "AccentBrush" : "TextBrush");
        StatusFilterButton.Content = _statusFilter.IsEmpty ? "狀態：全部 ▾" : $"狀態 · 選了 {_statusFilter.Count} 個 ▾";
    }

    static void Highlight(Control control, bool active)
    {
        if (active)
        {
            control.BorderBrush = Brushes2.Get("AccentBrush");
            control.Background = Brushes2.Get("SelectedBrush");
        }
        else
        {
            control.ClearValue(BorderBrushProperty);
            control.ClearValue(BackgroundProperty);
        }
    }

    // ───────────── 樹狀檢視 ─────────────

    readonly System.Collections.ObjectModel.ObservableCollection<object> _treeRows = [];

    /// <summary>依專案分組、照上下層排成樹狀清單；收合過的節點重畫時維持收合。</summary>
    void RenderTree(List<TaskSummary> shown, List<TaskListRowVm> rows)
    {
        var rowByKey = rows.ToDictionary(r => r.Key, StringComparer.OrdinalIgnoreCase);
        _treeRoots = MyTaskTree.Build(shown)
            .Select(p => new MyTreeProjectVm(p.Project, p.Count, _collapsed)
            {
                Children = p.Roots.Select(n => MyTreeTaskVm.From(n, rowByKey, _collapsed, 1)).ToList(),
            })
            .ToList();
        TreeList.ItemsSource ??= _treeRows;
        _treeRows.Clear();
        CollapseAllButton.Content = _treeRoots.Any(p => p.IsExpanded) ? "⊟ 全部收合" : "⊞ 全部展開";
        foreach (var project in _treeRoots)
        {
            _treeRows.Add(project);
            if (project.IsExpanded)
                AddVisible(project.Children, _treeRows.Count);
        }
    }

    /// <summary>把展開中的節點依序插進清單（從 index 開始），回傳下一個位置。</summary>
    int AddVisible(IEnumerable<MyTreeTaskVm> nodes, int index)
    {
        foreach (var node in nodes)
        {
            _treeRows.Insert(index++, node);
            if (node.IsExpanded)
                index = AddVisible(node.Children, index);
        }
        return index;
    }

    /// <summary>展開或收合：只插入／移除底下那幾列，捲動位置不會跳。</summary>
    void ToggleTreeNode(object node)
    {
        var index = _treeRows.IndexOf(node);
        if (index < 0)
            return;
        var (level, children) = node switch
        {
            MyTreeProjectVm p => (0, p.Children),
            MyTreeTaskVm t => (t.Depth, t.Children),
            _ => (0, new List<MyTreeTaskVm>()),
        };
        if (children.Count == 0)
            return;
        var expand = node is MyTreeProjectVm proj ? !proj.IsExpanded : !((MyTreeTaskVm)node).IsExpanded;
        if (node is MyTreeProjectVm pp)
            pp.IsExpanded = expand;
        else
            ((MyTreeTaskVm)node).IsExpanded = expand;
        if (expand)
        {
            AddVisible(children, index + 1);
            return;
        }
        while (index + 1 < _treeRows.Count && _treeRows[index + 1] is MyTreeTaskVm next && next.Depth > level)
            _treeRows.RemoveAt(index + 1);
    }

    /// <summary>全部收合（有展開的就全部收起來）或全部展開；收合狀態記著，別人改了任務重畫也不會又整個展開。</summary>
    void CollapseAll_Click(object sender, RoutedEventArgs e)
    {
        var anyExpanded = _treeRoots.Any(p => p.IsExpanded);
        if (anyExpanded)
        {
            foreach (var p in _treeRoots)
            {
                _collapsed.Add("p:" + p.Name);
                CollapseTasks(p.Children);
            }
        }
        else
            _collapsed.Clear();
        RenderList(keepSelection: true);
    }

    void CollapseTasks(IEnumerable<MyTreeTaskVm> nodes)
    {
        foreach (var n in nodes.Where(n => n.Children.Count > 0))
        {
            _collapsed.Add(n.Key);
            CollapseTasks(n.Children);
        }
    }

    void TreeToggle_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is { } node)
            ToggleTreeNode(node);
    }

    /// <summary>點任務列開彈窗、點專案列展開／收合；點到列上的按鈕不算。</summary>
    async void TreeList_Click(object sender, MouseButtonEventArgs e)
    {
        for (var d = e.OriginalSource as DependencyObject; d is not null && d != TreeList; d = VisualTreeHelper.GetParent(d))
            if (d is ButtonBase)
                return;
        switch ((e.OriginalSource as FrameworkElement)?.DataContext)
        {
            case MyTreeTaskVm task:
                await OpenTaskAsync(task.Key);
                break;
            case MyTreeProjectVm project:
                ToggleTreeNode(project);
                break;
        }
    }

    /// <summary>Enter 開啟、← 收合、→ 展開。</summary>
    async void TreeList_KeyDown(object sender, KeyEventArgs e)
    {
        var item = TreeList.SelectedItem;
        var expanded = item switch { MyTreeProjectVm p => p.IsExpanded, MyTreeTaskVm t => t.IsExpanded, _ => false };
        switch (e.Key)
        {
            case Key.Enter when item is MyTreeTaskVm task:
                e.Handled = true;
                await OpenTaskAsync(task.Key);
                break;
            case Key.Enter when item is MyTreeProjectVm:
            case Key.Left when item is not null && expanded:
            case Key.Right when item is not null && !expanded:
                e.Handled = true;
                ToggleTreeNode(item);
                break;
        }
    }

    void View_Checked(object sender, RoutedEventArgs e)
    {
        if (_buildingFilters)
            return;
        _view = sender == TreeViewButton ? "tree" : sender == KanbanViewButton ? "kanban" : "list";
        _engine.Config.TaskView = _view;
        _engine.Config.Save();
        RenderList(keepSelection: true);
    }

    List<(string Category, string Label, int Count)> _categoryOptions = [];
    List<(string Status, int Count)> _statusOptions = [];
    int _statusTotal;

    /// <summary>
    /// 狀態篩選的選項：這個分頁裡出現的狀態類別，加上各專案自己的狀態（處理中、待客戶回應…）與數量。
    /// 選的項目在新清單裡不見了（換分頁、任務改狀態）就拿掉。搜尋結果沒有狀態，不顯示。
    /// </summary>
    void RenderStatusFilters(List<TaskSummary> tasks)
    {
        _categoryOptions = TaskOrder.Categories(tasks);
        _statusOptions = TaskOrder.Statuses(tasks);
        _statusTotal = tasks.Count;
        _statusFilter.KeepOnly(_categoryOptions.Select(c => c.Category), _statusOptions.Select(s => s.Status));
        StatusFilterButton.Visibility = tasks.Count > 0 || !_statusFilter.IsEmpty ? Visibility.Visible : Visibility.Collapsed;
    }

    /// <summary>狀態篩選的選單：類別與各專案的狀態可以混著勾，勾了馬上套用、選單不關。</summary>
    void StatusFilter_Click(object sender, RoutedEventArgs e)
    {
        var menu = new ContextMenu { PlacementTarget = StatusFilterButton, Placement = PlacementMode.Bottom };
        MenuItem Check(string header, bool isChecked, Action<bool> toggled)
        {
            var item = new MenuItem { Header = header, IsCheckable = true, IsChecked = isChecked, StaysOpenOnClick = true };
            item.Click += (_, _) =>
            {
                toggled(item.IsChecked);
                RenderList(keepSelection: true);
            };
            return item;
        }
        menu.Items.Add(new MenuItem { Header = $"狀態類別（共 {_statusTotal} 個任務）", IsEnabled = false });
        foreach (var (category, label, count) in _categoryOptions)
            menu.Items.Add(Check($"{label}　{count}", _statusFilter.Categories.Contains(category), on =>
            {
                if (on)
                    _statusFilter.Categories.Add(category);
                else
                    _statusFilter.Categories.Remove(category);
            }));
        if (_statusOptions.Count > 0)
        {
            menu.Items.Add(new Separator());
            menu.Items.Add(new MenuItem { Header = "各專案的狀態", IsEnabled = false });
            foreach (var (status, count) in _statusOptions)
                menu.Items.Add(Check($"{status}　{count}", _statusFilter.Statuses.Contains(status), on =>
                {
                    if (on)
                        _statusFilter.Statuses.Add(status);
                    else
                        _statusFilter.Statuses.Remove(status);
                }));
        }
        menu.Items.Add(new Separator());
        var clear = new MenuItem { Header = "清除篩選（全部狀態）", IsEnabled = !_statusFilter.IsEmpty };
        clear.Click += (_, _) =>
        {
            _statusFilter.Clear();
            RenderList(keepSelection: true);
        };
        menu.Items.Add(clear);
        menu.IsOpen = true;
    }

    void SortBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_buildingFilters || SortBox.SelectedIndex < 0)
            return;
        _engine.Config.TaskSort = TaskOrder.Sorts[SortBox.SelectedIndex].Key;
        _engine.Config.Save();
        RenderList(keepSelection: true);
    }

    async void Reload_Click(object sender, RoutedEventArgs e)
    {
        _doneFetched = DateTime.MinValue; // 看板的已完成欄也重讀
        await _engine.AfterExternalChangeAsync();
    }

    async void SearchBox_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Escape)
        {
            SearchBox.Text = "";
            _tabs[TimerEngine.AllTab].IsChecked = true;
            _tabs[SearchTab].Visibility = Visibility.Collapsed;
            return;
        }
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        var q = SearchBox.Text.Trim();
        if (q.Length == 0)
            return;
        if (KeyFinder.Parse(q) is { } key && string.Equals(key, q, StringComparison.OrdinalIgnoreCase))
        {
            await OpenTaskAsync(key);
            return;
        }
        ListMessage.Text = "搜尋中…";
        try
        {
            _searchHits = await _engine.Api.SearchTasksAsync(q, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error("搜尋任務失敗", ex);
            _searchHits = [];
        }
        _tabs[SearchTab].Visibility = Visibility.Visible;
        _tabs[SearchTab].Content = $"搜尋「{(q.Length > 8 ? q[..8] + "…" : q)}」{_searchHits.Count}";
        if (_tabs[SearchTab].IsChecked == true)
            RenderList();
        else
            _tabs[SearchTab].IsChecked = true;
    }

    /// <summary>點清單裡的任務：跳出彈窗。點到列上的計時按鈕不算。</summary>
    async void TaskList_Click(object sender, MouseButtonEventArgs e)
    {
        for (var d = e.OriginalSource as DependencyObject; d is not null && d != TaskList; d = VisualTreeHelper.GetParent(d))
            if (d is ButtonBase)
                return;
        if ((e.OriginalSource as FrameworkElement)?.DataContext is TaskListRowVm row)
            await OpenTaskAsync(row.Key);
    }

    async void TaskList_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter && TaskList.SelectedItem is TaskListRowVm row)
        {
            e.Handled = true;
            await OpenTaskAsync(row.Key);
        }
    }

    // ───────────── 彈窗 ─────────────

    void ShowDialog()
    {
        if (Overlay.Visibility == Visibility.Visible)
            return;
        Overlay.Visibility = Visibility.Visible;
        if (SystemParameters.ClientAreaAnimation)
            Overlay.BeginAnimation(OpacityProperty, new DoubleAnimation(0, 1, TimeSpan.FromMilliseconds(140)));
    }

    /// <summary>關掉彈窗：正在讀取的任務作廢，正在編輯的欄位先存（LostFocus 會送出）。</summary>
    void CloseDialog()
    {
        if (Overlay.Visibility != Visibility.Visible)
            return;
        Keyboard.ClearFocus();
        _detailVersion++;
        Overlay.BeginAnimation(OpacityProperty, null);
        Overlay.Visibility = Visibility.Collapsed;
        DetailPanel.Visibility = CreatePanel.Visibility = Visibility.Collapsed;
        TaskList.Focus();
    }

    void Backdrop_Click(object sender, MouseButtonEventArgs e) => CloseDialog();

    void CloseDialog_Click(object sender, RoutedEventArgs e) => CloseDialog();

    async void RowTimer_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not TaskListRowVm row)
            return;
        ((Button)sender).IsEnabled = false;
        if (row.Timing)
            await _engine.PauseTaskAsync(row.Key);
        else
            await _engine.StartTaskAsync(row.Key);
        if (_detail?.Key == row.Key)
            RenderTimerButton();
    }

    // ───────────── 詳細 ─────────────

    async Task OpenTaskAsync(string key)
    {
        var version = ++_detailVersion;
        ShowDialog();
        SetDialogWidth(wide: false);
        CreatePanel.Visibility = Visibility.Collapsed;
        DetailEmpty.Text = "讀取中…";
        DetailEmpty.Visibility = Visibility.Visible;
        DetailPanel.Visibility = Visibility.Collapsed;
        try
        {
            var detail = await _engine.Api.GetTaskAsync(key, CancellationToken.None);
            var project = await _engine.GetProjectAsync(KeyFinder.ProjectOf(detail.Key));
            if (version != _detailVersion)
                return;
            ShowDetail(detail, project);
            await LoadChatAsync(detail.Key, version);
        }
        catch (Exception ex)
        {
            if (version != _detailVersion)
                return;
            Log.Error($"讀取任務 {key} 失敗", ex);
            DetailEmpty.Text = $"讀不到 {key}：{ex.Message}";
        }
    }

    void ShowDetail(TaskDetail d, ProjectDetail project)
    {
        _loadingDetail = true;
        _detail = d;
        DetailEmpty.Visibility = Visibility.Collapsed;
        DetailPanel.Visibility = Visibility.Visible;
        DetailPanel.ScrollToTop();
        Inputs.Show(DetailStatus, null);

        PathText.Text = d.Path.Count > 0 ? string.Join(" › ", d.Path.Select(p => $"{p.Key} {p.Title}")) : project.Name;
        KeyText.Text = d.Path.Count > 0 ? $"{d.Key} · {project.Name}" : d.Key;
        TitleBox.Text = d.Title;
        ApprovalText.Text = d.ApprovalState == "PENDING" ? "這個任務還在等管理者核准（可以先計時）。" :
            d.PendingChangeRequests > 0 ? $"有 {d.PendingChangeRequests} 個變更申請在等審核。" : "";
        ApprovalText.Visibility = ApprovalText.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

        var statuses = project.Statuses.Select(s => s.Name).ToList();
        if (!statuses.Contains(d.Status))
            statuses.Insert(0, d.Status);
        StatusBox.ItemsSource = statuses;
        StatusBox.SelectedItem = d.Status;
        PriorityBox.SelectedItem = PriorityBox.Items.OfType<ComboBoxItem>().FirstOrDefault(i => (string)i.Content == d.Priority);
        PlanDates.SetRange(d.PlannedStart, d.PlannedEnd);
        EstimateBox.Text = d.EstimatedHours?.ToString("0.##", CultureInfo.InvariantCulture) ?? "";
        RenderLogged(d);
        ActualText.Text = d.ActualStart is null && d.ActualEnd is null ? "—" :
            $"{Inputs.DateText(d.ActualStart)} → {(d.ActualEnd is { } ae ? Inputs.DateText(ae) : "進行中")}";
        AssigneesText.Text = d.Assignees.Count > 0 ? string.Join("、", d.Assignees.Select(a => a.DisplayName)) : "—";

        ChildrenList.ItemsSource = d.Children.Select(c => new { Key = c.Key, Title = c.Title, Status = $"· {c.Status}" }).ToList();
        ChildrenSection.Visibility = d.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        if (_childHoursFor != d.Key)
        {
            _childHoursFor = null;
            ChildHoursExpander.IsExpanded = false;
            ChildHoursPanel.Children.Clear();
            ChildHoursHeader.Text = "子任務的工時";
        }
        CategoryBox.IsChecked = d.IsCategory;
        NoteText.Text = d.Note ?? "";
        NoteSection.Visibility = string.IsNullOrWhiteSpace(d.Note) ? Visibility.Collapsed : Visibility.Visible;
        RenderTimerButton();
        _loadingDetail = false;
    }

    /// <summary>
    /// 已記錄／預估：例「2.1 / 4 小時」。用分鐘精確計算，只顯示到小數點後一位；超過預估時標紅。
    /// 沒設預估就只顯示已記錄。
    /// </summary>
    void RenderLogged(TaskDetail d)
    {
        double? logged = d.LoggedMinutes is { } m ? m / 60 : ParseHours(d.LoggedText);
        var loggedText = logged is { } h ? WorkMath.HoursNumber(h) : "0";
        LoggedText.Text = d.EstimatedHours is { } est
            ? $"{loggedText} / {WorkMath.HoursNumber(est)} 小時"
            : $"{loggedText} 小時（未設預估）";
        LoggedText.Foreground = Brushes2.Get(d.EstimatedHours is { } e && logged > e ? "ErrorBrush" : "TextBrush");
        LoggedText.ToolTip = logged is { } exact ? $"已記錄 {WorkMath.HoursMinutes(exact * 60)}" : null;
        if (d.IsCategory)
            LoggedText.Text += "（加總子任務）";
    }

    /// <summary>伺服器的顯示文字（例「3.58 小時」）取出小時數；舊版回應沒有分鐘數時用。</summary>
    static double? ParseHours(string text)
    {
        var number = new string(text.TakeWhile(c => char.IsDigit(c) || c == '.').ToArray());
        return double.TryParse(number, NumberStyles.Float, CultureInfo.InvariantCulture, out var h) ? h : null;
    }

    void RenderTimerButton()
    {
        if (_detail is null)
            return;
        var running = _engine.RunningKeys().Contains(_detail.Key);
        TimerButton.Content = running ? "❚❚ 暫停計時" : "▶ 開始計時";
        TimerButton.Style = (Style)FindResource(running ? "FlatButton" : "PrimaryButton");
        TimerButton.IsEnabled = running || !_detail.IsCategory;
        TimerButton.ToolTip = _detail.IsCategory && !running ? "分類任務只用來分組，不能計時" : null;
        CompleteButton.IsEnabled = _detail.StatusCategory is not "DONE" and not "CANCELLED";
        var pinned = _engine.IsPinned(_detail.Key);
        PinButton.Content = pinned ? "📌 取消釘選" : "📌 釘選";
        PinButton.ToolTip = pinned ? "從面板的托盤拿掉" : "釘到面板的托盤，一鍵切換正在做的任務";
        PinButton.IsEnabled = pinned || !_detail.IsCategory;
    }

    async Task LoadChatAsync(string key, int version)
    {
        ChatList.ItemsSource = null;
        ChatEmpty.Text = "讀取中…";
        try
        {
            var messages = await _engine.Api.ListChatAsync(key, null, null, CancellationToken.None);
            if (version != _detailVersion)
                return;
            ChatList.ItemsSource = ChatRows(messages);
            ChatEmpty.Text = messages.Count == 0 ? "還沒有訊息" : "";
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {key} 聊天失敗", ex);
            ChatEmpty.Text = "聊天讀取失敗";
        }
        ChatEmpty.Visibility = ChatEmpty.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    }

    internal static List<ChatRowVm> ChatRows(IEnumerable<ChatMessage> messages) =>
        messages.OrderBy(m => m.CreatedAt)
            .Select(m => new ChatRowVm(m.Author.DisplayName, Inputs.Ago(m.CreatedAt),
                m.Deleted ? "（訊息已刪除）" : m.PollQuestion is { } q ? $"📊 投票：{q}" : m.Body))
            .ToList();

    /// <summary>更新任務欄位；伺服器回的說明（例如送出變更申請）顯示在下方。</summary>
    async Task UpdateAsync(Dictionary<string, object?> fields, string what)
    {
        if (_detail is null || _loadingDetail)
            return;
        var key = _detail.Key;
        string? message = null;
        var ok = await Inputs.Run(null, DetailStatus, async () =>
            message = await _engine.Api.UpdateTaskAsync(key, fields, CancellationToken.None));
        if (!ok)
        {
            await OpenTaskAsync(key);
            return;
        }
        Inputs.Show(DetailStatus, message is { Length: > 0 } ? message : $"已更新{what}", error: false);
        await _engine.AfterExternalChangeAsync();
        await RefreshDetailQuietlyAsync(key);
    }

    async Task RefreshDetailQuietlyAsync(string key)
    {
        try
        {
            var detail = await _engine.Api.GetTaskAsync(key, CancellationToken.None);
            if (_detail?.Key != key)
                return;
            var status = DetailStatus.Text;
            var visibility = DetailStatus.Visibility;
            ShowDetail(detail, await _engine.GetProjectAsync(KeyFinder.ProjectOf(key)));
            (DetailStatus.Text, DetailStatus.Visibility) = (status, visibility);
        }
        catch (Exception ex)
        {
            Log.Error($"重新讀取 {key} 失敗", ex);
        }
    }

    void TitleBox_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter)
        {
            e.Handled = true;
            Keyboard.ClearFocus();
        }
        else if (e.Key == Key.Escape && _detail is not null)
        {
            TitleBox.Text = _detail.Title;
            Keyboard.ClearFocus();
        }
    }

    async void TitleBox_LostFocus(object sender, RoutedEventArgs e)
    {
        var title = TitleBox.Text.Trim();
        if (_detail is null || title.Length == 0 || title == _detail.Title)
        {
            if (_detail is not null && title.Length == 0)
                TitleBox.Text = _detail.Title;
            return;
        }
        await UpdateAsync(new() { ["title"] = title }, "名稱");
    }

    async void StatusBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_detail is null || _loadingDetail || StatusBox.SelectedItem is not string status || status == _detail.Status)
            return;
        await UpdateAsync(new() { ["status"] = status }, "狀態");
    }

    async void PriorityBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_detail is null || _loadingDetail || PriorityBox.SelectedItem is not ComboBoxItem { Content: string priority }
            || priority == _detail.Priority)
            return;
        await UpdateAsync(new() { ["priority"] = priority }, "優先級");
    }

    /// <summary>日曆按了「套用日期」或「清除」：只送有變的那一邊。</summary>
    async void PlanDates_Applied()
    {
        if (_detail is null)
            return;
        var fields = new Dictionary<string, object?>();
        if (PlanDates.Start != _detail.PlannedStart)
            fields["plannedStart"] = PlanDates.Start is { } s ? Inputs.DateText(s) : null;
        if (PlanDates.End != _detail.PlannedEnd)
            fields["plannedEnd"] = PlanDates.End is { } e ? Inputs.DateText(e) : null;
        if (fields.Count > 0)
            await UpdateAsync(fields, "預計日期");
    }

    async void EstimateBox_LostFocus(object sender, RoutedEventArgs e)
    {
        if (_detail is null)
            return;
        var text = EstimateBox.Text.Trim();
        double? hours = null;
        if (text.Length > 0)
        {
            if (!double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var h) || h < 0 || h > 1666)
            {
                Inputs.Show(DetailStatus, "預估工時請填小時數，例 4 或 1.5");
                EstimateBox.Text = _detail.EstimatedHours?.ToString("0.##", CultureInfo.InvariantCulture) ?? "";
                return;
            }
            hours = h;
        }
        if (hours == _detail.EstimatedHours)
            return;
        await UpdateAsync(new() { ["estimatedHours"] = hours }, "預估工時");
    }

    void CommitOnEnter(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        Keyboard.ClearFocus();
    }

    async void Timer_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is null)
            return;
        TimerButton.IsEnabled = false;
        if (_engine.RunningKeys().Contains(_detail.Key))
            await _engine.PauseTaskAsync(_detail.Key);
        else
            await _engine.StartTaskAsync(_detail.Key);
        TimerButton.IsEnabled = true;
        Inputs.Show(DetailStatus, _engine.Error);
        RenderTimerButton();
        RenderList(keepSelection: true);
    }

    async void Complete_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is null)
            return;
        var key = _detail.Key;
        bool? cascade = null;
        var openChildren = _detail.Children.Count(c => !IsDoneStatus(c.Status));
        if (openChildren > 0)
        {
            var answer = MessageBox.Show(Window.GetWindow(this)!,
                $"{key} 還有 {openChildren} 個子任務沒完成。\n\n是：照專案的「任務狀態連動」設定（有開的話子任務一起完成）\n否：只完成 {key}，不帶動子任務與上層",
                "停止並完成", MessageBoxButton.YesNoCancel, MessageBoxImage.Question);
            if (answer == MessageBoxResult.Cancel)
                return;
            cascade = answer == MessageBoxResult.Yes ? null : false;
        }
        CompleteButton.IsEnabled = false;
        await _engine.CompleteTaskAsync(key, cascade);
        Inputs.Show(DetailStatus, _engine.Error ?? "已停止並完成", error: _engine.Error is not null);
        await RefreshDetailQuietlyAsync(key);
    }

    /// <summary>子任務的狀態名稱是不是完成類（從專案的狀態清單查；查不到時用常見名稱判斷）。</summary>
    bool IsDoneStatus(string status)
    {
        if (_detail is not null && _engine.TryGetCachedProject(KeyFinder.ProjectOf(_detail.Key)) is { } project
            && project.Statuses.FirstOrDefault(s => s.Name == status) is { } option)
            return option.Category is "DONE" or "CANCELLED";
        return status is "完成" or "已完成" or "不處理" or "取消" or "已取消";
    }

    async void Pin_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is null)
            return;
        var key = _detail.Key;
        PinButton.IsEnabled = false;
        if (_engine.IsPinned(key))
            await _engine.UnpinTaskAsync(key);
        else
            await _engine.PinTaskAsync(key);
        Inputs.Show(DetailStatus, _engine.Error ?? (_engine.IsPinned(key) ? "已釘到托盤（面板的計時分頁最上面）" : "已取消釘選"),
            error: _engine.Error is not null);
        RenderTimerButton();
    }

    async void Category_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is null)
            return;
        await UpdateAsync(new() { ["isCategory"] = CategoryBox.IsChecked == true }, CategoryBox.IsChecked == true ? "成分類任務" : "成一般任務");
    }

    /// <summary>移動任務：同一個專案裡換上層或移到最上層（MCP 不能移到別的專案）。</summary>
    async void Move_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is null)
            return;
        var key = _detail.Key;
        var picker = new MoveTaskWindow(_engine, key, _detail.Path.LastOrDefault().Key is { Length: > 0 } p ? p : null)
        {
            Owner = Window.GetWindow(this),
        };
        if (picker.ShowDialog() != true)
            return;
        await _engine.MoveTaskAsync(key, picker.Parent);
        Inputs.Show(DetailStatus, _engine.Error ?? (picker.Parent is null ? "已移到最上層" : $"已移到 {picker.Parent} 底下"),
            error: _engine.Error is not null);
        await RefreshDetailQuietlyAsync(key);
    }

    string? _childHoursFor;

    /// <summary>展開「子任務的工時」才讀：每個直接子任務的計時段，依子任務列出，標題寫合計。</summary>
    async void ChildHours_Expanded(object sender, RoutedEventArgs e)
    {
        if (_detail is null || _childHoursFor == _detail.Key)
            return;
        var detail = _detail;
        _childHoursFor = detail.Key;
        ChildHoursPanel.Children.Clear();
        ChildHoursPanel.Children.Add(new TextBlock { Text = "讀取中…", Foreground = Brushes2.Get("MutedBrush") });
        var groups = new List<(string Key, string Title, List<TimeEntry> Entries)>();
        try
        {
            foreach (var child in detail.Children.Take(30))
                groups.Add((child.Key, child.Title, await _engine.Api.ListTaskEntriesAsync(child.Key, CancellationToken.None)));
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {detail.Key} 子任務的工時失敗", ex);
            ChildHoursPanel.Children.Clear();
            ChildHoursPanel.Children.Add(new TextBlock { Text = $"讀取失敗：{ex.Message}", Foreground = Brushes2.Get("ErrorBrush"), TextWrapping = TextWrapping.Wrap });
            _childHoursFor = null;
            return;
        }
        if (_detail?.Key != detail.Key)
            return;
        var now = DateTime.Now;
        double Minutes(TimeEntry t) => t.DurationMinutes ?? (t.StartedAt is { } s ? ((t.EndedAt ?? DateTimeOffset.Now) - s).TotalMinutes : 0);
        var total = groups.Sum(g => g.Entries.Sum(Minutes));
        ChildHoursHeader.Text = $"子任務的工時　合計 {WorkMath.HoursMinutes(total)}" + (detail.Children.Count > 30 ? "（前 30 個子任務）" : "");
        ChildHoursPanel.Children.Clear();
        foreach (var (key, title, entries) in groups.Where(g => g.Entries.Count > 0))
        {
            ChildHoursPanel.Children.Add(new TextBlock
            {
                Margin = new Thickness(0, 6, 0, 2),
                TextTrimming = TextTrimming.CharacterEllipsis,
                Inlines =
                {
                    new System.Windows.Documents.Run(key) { FontWeight = FontWeights.SemiBold },
                    new System.Windows.Documents.Run($" {title}"),
                    new System.Windows.Documents.Run($"　{WorkMath.HoursMinutes(entries.Sum(Minutes))}") { Foreground = Brushes2.Get("MutedBrush") },
                },
            });
            foreach (var t in entries.OrderByDescending(t => t.StartedAt))
            {
                var start = t.StartedAt?.ToLocalTime();
                var end = t.EndedAt?.ToLocalTime();
                ChildHoursPanel.Children.Add(new TextBlock
                {
                    Text = $"{start:M/d HH:mm}–{(t.Running ? "進行中" : end?.ToString("HH:mm"))}　{WorkMath.HoursMinutes(Minutes(t))}" +
                           (t.User is { Length: > 0 } u ? $"　{u}" : "") + (t.Note is { Length: > 0 } n ? $"　{n}" : ""),
                    Foreground = Brushes2.Get("MutedBrush"),
                    FontSize = 12,
                    Margin = new Thickness(12, 0, 0, 1),
                    TextTrimming = TextTrimming.CharacterEllipsis,
                });
            }
        }
        if (groups.All(g => g.Entries.Count == 0))
            ChildHoursPanel.Children.Add(new TextBlock { Text = "子任務都還沒有工時", Foreground = Brushes2.Get("MutedBrush") });
    }

    void Remind_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is null)
            return;
        CloseDialog();
        _host.ShowPage("reminders", _detail.Key);
    }

    void OpenWeb_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is not null)
            Web.Open(_engine.TaskUrl(_detail.Key));
    }

    async void Child_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is string key)
            await OpenTaskAsync(key);
    }

    async void ChatBox_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        await SendAsync();
    }

    async void Send_Click(object sender, RoutedEventArgs e) => await SendAsync();

    async Task SendAsync()
    {
        var body = ChatBox.Text.Trim();
        if (_detail is null || body.Length == 0)
            return;
        var key = _detail.Key;
        if (await Inputs.Run(SendButton, DetailStatus, () => _engine.Api.SendChatAsync(key, null, null, body, CancellationToken.None)))
        {
            ChatBox.Text = "";
            await LoadChatAsync(key, _detailVersion);
        }
    }

    // ───────────── 新增任務 ─────────────

    async void New_Click(object sender, RoutedEventArgs e) => await ShowCreateAsync(null);

    async void NewChild_Click(object sender, RoutedEventArgs e) => await ShowCreateAsync(_detail?.Key);

    void ShowCreate(string? parent) => _ = ShowCreateAsync(parent);

    /// <summary>開新增任務的彈窗；parent 有值時（任務的「＋ 子任務」）先選好那個專案與上層。</summary>
    async Task ShowCreateAsync(string? parent)
    {
        _detailVersion++;
        ShowDialog();
        SetDialogWidth(wide: true);
        _creatingChildOf = parent;
        _pickedParent = parent;
        DetailEmpty.Visibility = Visibility.Collapsed;
        DetailPanel.Visibility = Visibility.Collapsed;
        CreatePanel.Visibility = Visibility.Visible;
        CreateTitle.Text = parent is null ? "新增任務" : $"新增 {parent} 的子任務";
        NewTitle.Text = NewEstimate.Text = TreeFilter.Text = "";
        NewDates.SetRange(null, null);
        NewPriority.SelectedIndex = 0;
        Inputs.Show(CreateStatus, null);
        OpenCreatedButton.Visibility = Visibility.Collapsed;
        if (NewProject.ItemsSource is null)
        {
            try
            {
                NewProject.ItemsSource = await _engine.Api.ListProjectsAsync(CancellationToken.None);
            }
            catch (Exception ex)
            {
                Inputs.Show(CreateStatus, $"讀不到專案清單：{ex.Message}");
            }
        }
        var projectKey = parent is not null ? KeyFinder.ProjectOf(parent)
            : _detail is not null ? KeyFinder.ProjectOf(_detail.Key)
            : _engine.RunningKeys().Select(KeyFinder.ProjectOf).FirstOrDefault();
        if (NewProject.ItemsSource is IEnumerable<ProjectInfo> projects)
        {
            var target = projects.FirstOrDefault(p => p.Key == projectKey) ?? projects.FirstOrDefault();
            if (target is not null && Equals(NewProject.SelectedItem, target))
                await LoadTreeAsync(target.Key); // 同一個專案不會觸發 SelectionChanged，自己重讀
            else
            {
                _keepParentOnProjectChange = true;
                NewProject.SelectedItem = target;
            }
        }
        NewTitle.Focus();
    }

    /// <summary>詳細最寬 700；新增任務有專案樹，最寬 1040。視窗不夠寬時跟著縮（彈窗是置中撐滿、限制最大寬度）。</summary>
    void SetDialogWidth(bool wide) => Dialog.MaxWidth = wide ? 1040 : 700;

    async void NewProject_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (NewProject.SelectedItem is not ProjectInfo project)
            return;
        if (!_keepParentOnProjectChange)
            _pickedParent = null; // 換專案時上層歸零
        _keepParentOnProjectChange = false;
        await LoadTreeAsync(project.Key);
    }

    /// <summary>讀專案的全部任務與狀態清單，畫出專案樹。</summary>
    async Task LoadTreeAsync(string projectKey)
    {
        var version = ++_treeVersion;
        TreeMessage.Text = "讀取中…";
        TreeMessage.Visibility = Visibility.Visible;
        ProjectTree.ItemsSource = null;
        try
        {
            var tasks = await _engine.Api.ListProjectTasksAsync(projectKey, CancellationToken.None);
            var project = await _engine.GetProjectAsync(projectKey);
            if (version != _treeVersion)
                return;
            _projectTasks = tasks;
            var statuses = project.Statuses.Select(s => s.Name).ToList();
            NewStatus.ItemsSource = statuses;
            NewStatus.SelectedItem = project.Statuses.FirstOrDefault(s => s.IsDefault)?.Name ?? statuses.FirstOrDefault();
            if (_pickedParent is not null && !tasks.Any(t => string.Equals(t.Key, _pickedParent, StringComparison.OrdinalIgnoreCase)))
                _pickedParent = null;
            RenderTree(scrollToPicked: true);
        }
        catch (Exception ex)
        {
            if (version != _treeVersion)
                return;
            Log.Error($"讀取 {projectKey} 的任務樹失敗", ex);
            _projectTasks = [];
            TreeMessage.Text = $"讀不到專案的任務：{ex.Message}";
        }
        UpdateParentText();
    }

    void RenderTree(bool scrollToPicked = false)
    {
        var filter = TreeFilter.Text.Trim();
        var roots = TaskTree.Build(_projectTasks, TreeHideDoneBox.IsChecked != true, filter);
        var expandAll = filter.Length > 0 || _projectTasks.Count <= 150;
        var picked = _pickedParent is null ? new HashSet<string>() : AncestorsOf(_pickedParent);
        _treeNodes = roots.Select(r => TreeNodeVm.From(r, _pickedParent, expandAll, picked)).ToList();
        foreach (var node in _treeNodes)
            node.SetWidth(ProjectTree.ActualWidth);
        ProjectTree.ItemsSource = _treeNodes;
        RootRow.Background = _pickedParent is null ? Brushes2.Get("SelectedBrush") : Brushes.Transparent;
        TreeMessage.Text = _projectTasks.Count == 0 ? "這個專案還沒有任務，新增的會在最上層"
            : roots.Count == 0 ? "沒有符合的任務" : "";
        TreeMessage.Visibility = TreeMessage.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        if (scrollToPicked && _pickedParent is not null)
            Dispatcher.InvokeAsync(() => BringIntoView(ProjectTree, _pickedParent), System.Windows.Threading.DispatcherPriority.Loaded);
    }

    /// <summary>某任務的所有上層（含自己），展開用。</summary>
    HashSet<string> AncestorsOf(string key)
    {
        var byKey = _projectTasks.ToDictionary(t => t.Key, StringComparer.OrdinalIgnoreCase);
        var set = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        for (var k = key; k is not null && set.Add(k) && byKey.TryGetValue(k, out var t);)
            k = t.Parent;
        return set;
    }

    /// <summary>把樹裡某個任務捲到看得到的地方（容器要等畫面產生後才找得到）。</summary>
    static bool BringIntoView(ItemsControl parent, string key)
    {
        foreach (var item in parent.Items.OfType<TreeNodeVm>())
        {
            if (parent.ItemContainerGenerator.ContainerFromItem(item) is not TreeViewItem container)
                continue;
            if (string.Equals(item.Key, key, StringComparison.OrdinalIgnoreCase))
            {
                container.BringIntoView();
                return true;
            }
            if (item.Children.Count > 0 && container.IsExpanded)
            {
                container.UpdateLayout();
                if (BringIntoView(container, key))
                    return true;
            }
        }
        return false;
    }

    void UpdateParentText()
    {
        if (_pickedParent is null)
        {
            ParentText.Text = "最上層（不屬於任何任務）";
            return;
        }
        var byKey = _projectTasks.ToDictionary(t => t.Key, StringComparer.OrdinalIgnoreCase);
        var chain = AncestorsOf(_pickedParent).Reverse()
            .Select(k => byKey.TryGetValue(k, out var t) ? $"{t.Key} {t.Title}" : k);
        ParentText.Text = string.Join(" › ", chain);
    }

    void PickParent(string? key)
    {
        _pickedParent = key;
        foreach (var node in _treeNodes)
            node.SetPicked(key);
        RootRow.Background = key is null ? Brushes2.Get("SelectedBrush") : Brushes.Transparent;
        UpdateParentText();
        NewTitle.Focus();
    }

    void TreeRow_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is TreeNodeVm node)
            PickParent(node.Key);
    }

    void TreeAdd_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is TreeNodeVm node)
        {
            e.Handled = true;
            PickParent(node.Key);
        }
    }

    void RootAdd_Click(object sender, MouseButtonEventArgs e) => PickParent(null);

    void TreeFilter_TextChanged(object sender, TextChangedEventArgs e) => RenderTree();

    /// <summary>樹變寬變窄時，各列的名稱跟著截短，讓前面的 ＋ 與狀態都看得到。</summary>
    void ProjectTree_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        foreach (var node in _treeNodes)
            node.SetWidth(e.NewSize.Width);
    }

    void ShowDone_Click(object sender, RoutedEventArgs e)
    {
        _engine.Config.TreeHideDone = TreeHideDoneBox.IsChecked == true;
        _engine.Config.Save();
        RenderTree();
    }

    async void NewTitle_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        await CreateAsync();
    }

    /// <summary>取消：從任務的「＋ 子任務」進來的回到那個任務，否則關掉彈窗。</summary>
    async void CancelCreate_Click(object sender, RoutedEventArgs e)
    {
        if (_creatingChildOf is { } parent)
            await OpenTaskAsync(parent);
        else
            CloseDialog();
    }

    async void Create_Click(object sender, RoutedEventArgs e) => await CreateAsync();

    /// <summary>建立任務；成功後留在彈窗、重讀專案樹並標出新任務，可以接著新增下一筆。</summary>
    async Task CreateAsync()
    {
        if (NewProject.SelectedItem is not ProjectInfo project)
        {
            Inputs.Show(CreateStatus, "請選專案");
            return;
        }
        var title = NewTitle.Text.Trim();
        if (title.Length == 0)
        {
            Inputs.Show(CreateStatus, "請填名稱");
            NewTitle.Focus();
            return;
        }
        var fields = new Dictionary<string, object?> { ["project"] = project.Key, ["title"] = title };
        if (_pickedParent is not null)
            fields["parent"] = _pickedParent;
        if (NewStatus.SelectedItem is string status)
            fields["status"] = status;
        if (NewPriority.SelectedItem is ComboBoxItem { Content: string priority } && priority != "無")
            fields["priority"] = priority;
        if (NewDates.Start is { } start)
            fields["plannedStart"] = Inputs.DateText(start);
        if (NewDates.End is { } end)
            fields["plannedEnd"] = Inputs.DateText(end);
        if (NewEstimate.Text.Trim().Length > 0)
        {
            if (!double.TryParse(NewEstimate.Text.Trim(), NumberStyles.Float, CultureInfo.InvariantCulture, out var hours) || hours < 0)
            {
                Inputs.Show(CreateStatus, "預估工時請填小時數");
                return;
            }
            fields["estimatedHours"] = hours;
        }
        if (NewAssignMe.IsChecked == true)
            fields["assignees"] = new[] { "me" };

        string key = "";
        OpenCreatedButton.Visibility = Visibility.Collapsed;
        if (!await Inputs.Run(CreateButton, CreateStatus, async () => key = await _engine.Api.CreateTaskAsync(fields, CancellationToken.None)))
            return;
        if (NewStartTimer.IsChecked == true && key.Length > 0)
            await _engine.StartTaskAsync(key);
        _lastCreated = key.Length > 0 ? key : null;
        Inputs.Show(CreateStatus, key.Length > 0 ? $"已建立 {key}　{title}" : "已建立", error: false);
        if (_lastCreated is not null)
        {
            OpenCreatedButton.Content = $"開啟 {_lastCreated} ›";
            OpenCreatedButton.Visibility = Visibility.Visible;
        }
        NewTitle.Text = "";
        NewTitle.Focus();
        _ = _engine.AfterExternalChangeAsync();
        await LoadTreeAsync(project.Key);
        if (_lastCreated is not null)
            Dispatcher.InvokeAsync(() => BringIntoView(ProjectTree, _lastCreated), System.Windows.Threading.DispatcherPriority.Loaded);
    }

    async void OpenCreated_Click(object sender, RoutedEventArgs e)
    {
        if (_lastCreated is { } key)
            await OpenTaskAsync(key);
    }
}

/// <summary>專案樹的一列：選為上層時底色標出來，已完成的淡一點。</summary>
public sealed class TreeNodeVm : System.ComponentModel.INotifyPropertyChanged
{
    bool _isExpanded;
    bool _picked;
    double _textMaxWidth = 240;

    public required ProjectTask Task { get; init; }
    public string Key => Task.Key;
    public string Title => Task.Title;
    public string Status => Task.Status;
    public string StatusSuffix => Task.IsCategory ? $"  · 分類 · {Task.Status}" : $"  · {Task.Status}";
    public int Depth { get; private init; }
    public List<TreeNodeVm> Children { get; } = [];

    /// <summary>名稱最多多寬：樹的寬度扣掉每層縮排（19）、展開箭頭、＋ 按鈕與捲軸。</summary>
    public double TextMaxWidth => _textMaxWidth;

    public void SetWidth(double treeWidth)
    {
        var width = Math.Max(80, treeWidth - 19 * (Depth + 1) - 34 - 26);
        if (Math.Abs(width - _textMaxWidth) > 0.5)
        {
            _textMaxWidth = width;
            Raise(nameof(TextMaxWidth));
        }
        foreach (var c in Children)
            c.SetWidth(treeWidth);
    }
    public double TextOpacity => Task.StatusCategory is "DONE" or "CANCELLED" ? 0.55 : 1;
    public string Tip => $"{Task.Key} {Task.Title}" +
                         (Task.PlannedStart is not null || Task.PlannedEnd is not null
                             ? $"\n預計 {DateOrUnset(Task.PlannedStart)} → {DateOrUnset(Task.PlannedEnd)}" : "") +
                         "\n點一下或按 ＋：在這個任務底下新增";

    static string DateOrUnset(DateOnly? d) => d is { } day ? Inputs.DateText(day) : "未指定";

    public bool IsExpanded
    {
        get => _isExpanded;
        set
        {
            _isExpanded = value;
            Raise(nameof(IsExpanded));
        }
    }

    public Brush RowBrush => _picked ? Brushes2.Get("SelectedBrush") : Brushes.Transparent;

    public void SetPicked(string? key)
    {
        var picked = string.Equals(Key, key, StringComparison.OrdinalIgnoreCase);
        if (picked != _picked)
        {
            _picked = picked;
            Raise(nameof(RowBrush));
        }
        foreach (var c in Children)
            c.SetPicked(key);
    }

    /// <summary>expandAll：全部展開；否則只展開第一層與選中任務的上層。</summary>
    public static TreeNodeVm From(TaskTreeNode node, string? picked, bool expandAll, IReadOnlySet<string> pickedAncestors, int depth = 0)
    {
        var vm = new TreeNodeVm { Task = node.Task, Depth = depth };
        vm._picked = string.Equals(node.Task.Key, picked, StringComparison.OrdinalIgnoreCase);
        vm._isExpanded = expandAll || depth == 0 || pickedAncestors.Contains(node.Task.Key);
        vm.Children.AddRange(node.Children.Select(c => From(c, picked, expandAll, pickedAncestors, depth + 1)));
        return vm;
    }

    public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;

    void Raise(string name) => PropertyChanged?.Invoke(this, new System.ComponentModel.PropertyChangedEventArgs(name));
}

/// <summary>樹狀檢視的專案列。</summary>
public sealed class MyTreeProjectVm(string name, int count, HashSet<string> collapsed) : System.ComponentModel.INotifyPropertyChanged
{
    public string Name { get; } = name;
    public string CountText => $"  {count}";
    public required List<MyTreeTaskVm> Children { get; init; }
    public string ExpandGlyph => IsExpanded ? "▾" : "▸";

    public bool IsExpanded
    {
        get => !collapsed.Contains("p:" + Name);
        set
        {
            if (value)
                collapsed.Remove("p:" + Name);
            else
                collapsed.Add("p:" + Name);
            PropertyChanged?.Invoke(this, new(nameof(IsExpanded)));
            PropertyChanged?.Invoke(this, new(nameof(ExpandGlyph)));
        }
    }

    public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;
}

/// <summary>樹狀清單的任務列；Row 為 null 表示只是上層脈絡（不在這次的清單裡），淡色、沒有計時按鈕。</summary>
public sealed class MyTreeTaskVm : System.ComponentModel.INotifyPropertyChanged
{
    HashSet<string> _collapsed = [];

    public required string Key { get; init; }
    public required string Title { get; init; }
    public TaskListRowVm? Row { get; init; }
    /// <summary>層級：專案底下第一層是 1。</summary>
    public int Depth { get; init; }
    public List<MyTreeTaskVm> Children { get; } = [];

    public Thickness Indent => new((Depth - 1) * 20, 1, 0, 1);
    public Visibility RowVisibility => Row is null ? Visibility.Collapsed : Visibility.Visible;
    public Visibility ExpanderVisibility => Children.Count > 0 ? Visibility.Visible : Visibility.Hidden;
    public string ExpandGlyph => IsExpanded ? "▾" : "▸";
    public double TextOpacity => Row is null ? 0.55 : Row.Done ? 0.6 : 1;
    public string SubText => Row is null ? "上層任務（不在這個分頁）" : Children.Count > 0 ? $"{Children.Count} 個子任務" : "";
    public Visibility SubTextVisibility => SubText.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    public string Tip => Row is null ? $"{Key} {Title}\n上層任務（不在這個分頁裡），點一下查看" : $"{Key} {Title}";

    public bool IsExpanded
    {
        get => !_collapsed.Contains(Key);
        set
        {
            if (value)
                _collapsed.Remove(Key);
            else
                _collapsed.Add(Key);
            PropertyChanged?.Invoke(this, new(nameof(IsExpanded)));
            PropertyChanged?.Invoke(this, new(nameof(ExpandGlyph)));
        }
    }

    public static MyTreeTaskVm From(MyTaskNode node, IReadOnlyDictionary<string, TaskListRowVm> rows, HashSet<string> collapsed, int depth)
    {
        var vm = new MyTreeTaskVm
        {
            Key = node.Key,
            Title = node.Title,
            Row = node.Task is not null ? rows.GetValueOrDefault(node.Key) : null,
            Depth = depth,
            _collapsed = collapsed,
        };
        vm.Children.AddRange(node.Children.Select(c => From(c, rows, collapsed, depth + 1)));
        return vm;
    }

    public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;
}

/// <summary>看板的一張卡片；計時中的是深色卡片，Elapsed 每秒更新。</summary>
public sealed class KanbanCardVm(TaskSummary task, TaskListRowVm row) : System.ComponentModel.INotifyPropertyChanged
{
    string _elapsed = "";

    public TaskSummary Task { get; } = task;
    public TaskListRowVm Row { get; } = row;
    public string Key => Task.Key;
    public string Title => Task.Title;
    public string Project => Task.Project;
    public string Status => Task.Status;
    public string PathText => Task.Path;
    public Visibility PathVisibility => Task.Path.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    public string Tip => $"{Task.Key} {Task.Title}" + (Task.Path.Length > 0 ? $"\n{Task.Project} › {Task.Path}" : $"\n{Task.Project}");
    public bool Running => Row.Timing;
    public Visibility RunningVisibility => Running ? Visibility.Visible : Visibility.Collapsed;
    public Visibility TimerVisibility => Row.Done && !Running ? Visibility.Collapsed : Visibility.Visible; // 已完成的不用計時按鈕

    public Brush CardBrush => Brushes2.Get(Running ? "RunCardBrush" : "BgBrush");
    public Thickness CardBorder => new(Running ? 0 : 1);
    public Brush TextBrush => Brushes2.Get(Running ? "RunCardTextBrush" : "TextBrush");
    public Brush MutedBrush => Brushes2.Get(Running ? "RunCardMutedBrush" : "MutedBrush");
    public Brush TimerBrush => Running ? Brushes2.Get("ErrorBrush") : Brushes2.Get("MutedBrush");
    public Brush StatusBrush => Brushes2.Get(Task.StatusCategory switch
    {
        "IN_PROGRESS" => "StatusDoingBrush",
        "DONE" => "StatusDoneBrush",
        _ => "StatusTodoBrush",
    });

    /// <summary>到期日：已完成的只顯示日期，其他跟清單一樣（逾期 N 天、今天到期…）。</summary>
    public string DueText => Row.DueText;
    public Brush DueBrush => Running && Row.DueBrush == Brushes2.Get("MutedBrush") ? Brushes2.Get("RunCardMutedBrush") : Row.DueBrush;

    public string Elapsed
    {
        get => _elapsed;
        set
        {
            if (_elapsed == value)
                return;
            _elapsed = value;
            PropertyChanged?.Invoke(this, new(nameof(Elapsed)));
        }
    }

    public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;
}
