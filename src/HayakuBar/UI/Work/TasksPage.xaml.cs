using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

public sealed class TaskListRowVm
{
    public required string Key { get; init; }
    public required string Title { get; init; }
    public string Context { get; init; } = "";
    public string Status { get; init; } = "";
    public DateOnly? PlannedEnd { get; init; }
    public bool Timing { get; init; }
    public bool Done { get; init; }

    public Visibility StatusVisibility => Status.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    public string TimerGlyph => Timing ? "❚❚" : "▶";
    public string TimerTip => Timing ? "暫停計時" : "開始計時";
    public Brush TimerBrush => Brushes2.Get(Timing ? "RunBrush" : "MutedBrush");

    public string DueText
    {
        get
        {
            if (PlannedEnd is not { } end)
                return "";
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
    string _tab = "in_progress";
    string? _statusFilter; // null：全部狀態
    bool _buildingFilters;
    List<SearchHit> _searchHits = [];
    TaskDetail? _detail;
    bool _loadingDetail;
    int _detailVersion;

    public TasksPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        InitializeComponent();
        foreach (var (tab, label) in TimerEngine.TaskTabs.Append((SearchTab, "搜尋結果")))
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
        _buildingFilters = false;

        Loaded += (_, _) => _engine.Changed += OnEngineChanged;
        Unloaded += (_, _) => _engine.Changed -= OnEngineChanged;
    }

    void OnEngineChanged()
    {
        if (IsVisible)
            RenderList(keepSelection: true);
    }

    /// <summary>arg：任務編號（直接打開那個任務）、"new"（新增）、或分頁名稱（overdue…）。</summary>
    public async Task ActivateAsync(object? arg)
    {
        RenderList(keepSelection: true);
        switch (arg)
        {
            case "new":
                ShowCreate(null);
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
        foreach (var (tab, label) in TimerEngine.TaskTabs)
        {
            var count = _engine.TaskList(tab).Count;
            _tabs[tab].Content = count > 0 ? $"{label} {count}" : label;
        }
        List<TaskListRowVm> rows;
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
                Context = h.Project,
                Timing = running.Contains(h.Key),
            }).ToList();
            ListMessage.Text = rows.Count == 0 ? "找不到符合的任務" : "";
        }
        else
        {
            var tasks = _engine.TaskList(_tab);
            RenderStatusFilters(TaskOrder.Statuses(tasks), tasks.Count);
            var shown = TaskOrder.Sort(tasks.Where(t => _statusFilter is null || t.Status == _statusFilter), sort, running);
            rows = shown.Select(t => new TaskListRowVm
            {
                Key = t.Key,
                Title = t.Title,
                Context = t.Path.Length > 0 ? $"{t.Project} › {t.Path}" : t.Project,
                Status = t.Status,
                PlannedEnd = t.PlannedEnd,
                Timing = running.Contains(t.Key) || t.Timing,
                Done = t.StatusCategory is "DONE" or "CANCELLED",
            }).ToList();
            ListMessage.Text = rows.Count > 0 ? "" : _engine.TaskLists.Count == 0 ? "讀取中…" : _statusFilter is not null
                ? $"沒有「{_statusFilter}」的任務" : _tab switch
            {
                "overdue" => "沒有逾期的任務 🎉",
                "should_start" => "沒有該開始還沒開始的任務",
                _ => "這裡沒有任務",
            };
        }
        var selected = (TaskList.SelectedItem as TaskListRowVm)?.Key ?? _detail?.Key;
        TaskList.ItemsSource = rows;
        if (keepSelection && selected is not null)
        {
            _loadingDetail = true;
            TaskList.SelectedItem = rows.FirstOrDefault(r => r.Key == selected);
            _loadingDetail = false;
        }
    }

    /// <summary>
    /// 狀態篩選的小標籤：「全部 27」加上這個分頁裡出現的每個狀態與數量；點了只顯示那個狀態。
    /// 選的狀態在新清單裡不見了（換分頁、任務改狀態）就回到全部。搜尋結果沒有狀態，不顯示。
    /// </summary>
    void RenderStatusFilters(List<(string Status, int Count)> statuses, int total = 0)
    {
        if (_statusFilter is not null && !statuses.Any(s => s.Status == _statusFilter))
            _statusFilter = null;
        StatusFilterPanel.Visibility = statuses.Count > 1 || _statusFilter is not null ? Visibility.Visible : Visibility.Collapsed;
        _buildingFilters = true;
        StatusFilterPanel.Children.Clear();
        var chips = new List<(string? Status, int Count)> { (null, total) };
        chips.AddRange(statuses.Select(s => ((string?)s.Status, s.Count)));
        foreach (var (status, count) in chips)
        {
            var chip = new RadioButton
            {
                Style = (Style)FindResource("ChipToggle"),
                GroupName = "TaskStatusFilter",
                Content = $"{status ?? "全部"} {count}",
                Margin = new Thickness(0, 0, 5, 4),
                IsChecked = status == _statusFilter,
            };
            chip.Checked += (_, _) =>
            {
                if (_buildingFilters)
                    return;
                _statusFilter = status;
                RenderList(keepSelection: true);
            };
            StatusFilterPanel.Children.Add(chip);
        }
        _buildingFilters = false;
    }

    void SortBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_buildingFilters || SortBox.SelectedIndex < 0)
            return;
        _engine.Config.TaskSort = TaskOrder.Sorts[SortBox.SelectedIndex].Key;
        _engine.Config.Save();
        RenderList(keepSelection: true);
    }

    async void Reload_Click(object sender, RoutedEventArgs e) => await _engine.AfterExternalChangeAsync();

    async void SearchBox_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Escape)
        {
            SearchBox.Text = "";
            _tabs["in_progress"].IsChecked = true;
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

    async void TaskList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_loadingDetail || TaskList.SelectedItem is not TaskListRowVm row)
            return;
        await OpenTaskAsync(row.Key);
    }

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
        PlannedStartBox.Text = Inputs.DateText(d.PlannedStart);
        PlannedEndBox.Text = Inputs.DateText(d.PlannedEnd);
        EstimateBox.Text = d.EstimatedHours?.ToString("0.##", CultureInfo.InvariantCulture) ?? "";
        LoggedText.Text = d.LoggedText.Length > 0 ? d.LoggedText : "—";
        ActualText.Text = d.ActualStart is null && d.ActualEnd is null ? "—" :
            $"{Inputs.DateText(d.ActualStart)} → {(d.ActualEnd is { } ae ? Inputs.DateText(ae) : "進行中")}";
        AssigneesText.Text = d.Assignees.Count > 0 ? string.Join("、", d.Assignees.Select(a => a.DisplayName)) : "—";

        ChildrenList.ItemsSource = d.Children.Select(c => new { Key = c.Key, Title = c.Title, Status = $"· {c.Status}" }).ToList();
        ChildrenSection.Visibility = d.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        NoteText.Text = d.Note ?? "";
        NoteSection.Visibility = string.IsNullOrWhiteSpace(d.Note) ? Visibility.Collapsed : Visibility.Visible;
        RenderTimerButton();
        _loadingDetail = false;
    }

    void RenderTimerButton()
    {
        if (_detail is null)
            return;
        var running = _engine.RunningKeys().Contains(_detail.Key);
        TimerButton.Content = running ? "❚❚ 暫停計時" : "▶ 開始計時";
        TimerButton.Style = (Style)FindResource(running ? "FlatButton" : "PrimaryButton");
        CompleteButton.IsEnabled = _detail.StatusCategory is not "DONE" and not "CANCELLED";
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

    async void DateBox_LostFocus(object sender, RoutedEventArgs e)
    {
        if (_detail is null || sender is not TextBox box)
            return;
        var isStart = box == PlannedStartBox;
        var current = isStart ? _detail.PlannedStart : _detail.PlannedEnd;
        var text = box.Text.Trim();
        DateOnly? value = null;
        if (text.Length > 0)
        {
            value = Inputs.ParseDate(text);
            if (value is null)
            {
                Inputs.Show(DetailStatus, "日期看不懂，請用 11/5、2026-11-05、明天、+3 這類寫法");
                box.Text = Inputs.DateText(current);
                return;
            }
        }
        box.Text = Inputs.DateText(value);
        if (value == current)
            return;
        await UpdateAsync(new() { [isStart ? "plannedStart" : "plannedEnd"] = value is { } v ? Inputs.DateText(v) : null },
            isStart ? "預計開始" : "預計結束");
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
        CompleteButton.IsEnabled = false;
        await _engine.CompleteTaskAsync(key);
        Inputs.Show(DetailStatus, _engine.Error ?? "已停止並完成", error: _engine.Error is not null);
        await RefreshDetailQuietlyAsync(key);
    }

    void Remind_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is not null)
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

    async Task ShowCreateAsync(string? parent)
    {
        _detailVersion++;
        DetailEmpty.Visibility = Visibility.Collapsed;
        DetailPanel.Visibility = Visibility.Collapsed;
        CreatePanel.Visibility = Visibility.Visible;
        CreateTitle.Text = parent is null ? "新增任務" : $"新增 {parent} 的子任務";
        NewParent.Text = parent ?? "";
        NewTitle.Text = NewStart.Text = NewEnd.Text = NewEstimate.Text = "";
        Inputs.Show(CreateStatus, null);
        NewTitle.Focus();
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
            NewProject.SelectedItem = projects.FirstOrDefault(p => p.Key == projectKey) ?? projects.FirstOrDefault();
    }

    void CancelCreate_Click(object sender, RoutedEventArgs e)
    {
        CreatePanel.Visibility = Visibility.Collapsed;
        if (_detail is not null)
            DetailPanel.Visibility = Visibility.Visible;
        else
        {
            DetailEmpty.Text = "從左邊選一個任務";
            DetailEmpty.Visibility = Visibility.Visible;
        }
    }

    async void Create_Click(object sender, RoutedEventArgs e)
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
        if (NewParent.Text.Trim().Length > 0)
        {
            if (KeyFinder.Parse(NewParent.Text) is not { } parent)
            {
                Inputs.Show(CreateStatus, "上層任務請填任務編號，例 F2P-5");
                return;
            }
            fields["parent"] = parent;
        }
        foreach (var (box, name) in new[] { (NewStart, "plannedStart"), (NewEnd, "plannedEnd") })
        {
            if (box.Text.Trim().Length == 0)
                continue;
            if (Inputs.ParseDate(box.Text) is not { } d)
            {
                Inputs.Show(CreateStatus, "日期看不懂，請用 11/5、明天、+3 這類寫法");
                return;
            }
            fields[name] = Inputs.DateText(d);
        }
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
        if (!await Inputs.Run(CreateButton, CreateStatus, async () => key = await _engine.Api.CreateTaskAsync(fields, CancellationToken.None)))
            return;
        if (NewStartTimer.IsChecked == true && key.Length > 0)
            await _engine.StartTaskAsync(key);
        await _engine.AfterExternalChangeAsync();
        if (key.Length > 0)
            await OpenTaskAsync(key);
        else
            Inputs.Show(CreateStatus, "已建立", error: false);
    }
}
