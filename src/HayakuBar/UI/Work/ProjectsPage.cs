using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 專案：任一個專案的全部任務（不只指派給我的），樹狀顯示；可套用專案存好的視圖、篩選、只看我的、只看沒有子任務的，
/// 勾選多個一次改狀態或優先級。點任務在「我的任務」的詳細打開。
/// 任務樹與「建立任務」小視窗共用快取（10 分鐘內不重抓，按重新整理才重讀）；視圖的篩選能在本機套用就不另外打請求。
/// </summary>
public sealed class ProjectsPage : UserControl, IWorkPage
{
    const string AllView = "";
    static readonly string[] Priorities = ["緊急", "高", "中", "低", "無"];

    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;
    readonly ComboBox _project = new() { Width = 230, Margin = new Thickness(0, 0, 8, 0) };
    readonly ComboBox _view = new() { Width = 160, Margin = new Thickness(0, 0, 8, 0), DisplayMemberPath = "Name" };
    readonly TextBox _filter = new() { Width = 200, Margin = new Thickness(0, 0, 10, 0) };
    readonly CheckBox _hideDone = new() { Content = "不顯示完成及不處理", VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 10, 0) };
    readonly CheckBox _mine = new() { Content = "只看我的", VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 10, 0) };
    readonly CheckBox _leafOnly = new() { Content = "只顯示沒有子任務的", VerticalAlignment = VerticalAlignment.Center };
    readonly TextBlock _summary = new() { Margin = new Thickness(0, 0, 0, 8) };
    readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    readonly Border _batch;
    readonly TextBlock _batchText = new() { VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 12, 0), FontWeight = FontWeights.SemiBold };
    readonly ComboBox _batchStatus = new() { Width = 140, Margin = new Thickness(0, 0, 6, 0) };
    readonly ComboBox _batchPriority = new() { Width = 80, Margin = new Thickness(0, 0, 6, 0), ItemsSource = Priorities };
    readonly StackPanel _list = new();
    readonly Dictionary<string, List<ProjectView>> _views = new(StringComparer.OrdinalIgnoreCase);
    readonly HashSet<string> _selected = new(StringComparer.OrdinalIgnoreCase);
    readonly HashSet<string> _collapsed = new(StringComparer.OrdinalIgnoreCase);
    List<ProjectTask> _tasks = [];
    List<ProjectTask>? _viewTasks;
    ProjectDetail? _detail;
    int _version;
    bool _building;

    public ProjectsPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        _filter.Style = (Style)FindResource("HintBox");
        _filter.Tag = "🔍 篩選編號或名稱";
        _summary.SetResourceReference(TextBlock.ForegroundProperty, "MutedBrush");
        _hideDone.IsChecked = engine.Config.TreeHideDone;

        _project.SelectionChanged += async (_, _) =>
        {
            if (!_building)
                await LoadAsync(fresh: false);
        };
        _view.SelectionChanged += async (_, _) =>
        {
            if (!_building)
                await ApplyViewAsync();
        };
        _filter.TextChanged += (_, _) => Render();
        _hideDone.Click += (_, _) =>
        {
            _engine.Config.TreeHideDone = _hideDone.IsChecked == true;
            _engine.Config.Save();
            Render();
        };
        _mine.Click += (_, _) => Render();
        _leafOnly.Click += (_, _) => Render();

        var create = Button("＋ 新增任務", "FlatButton");
        create.Click += (_, _) => _host.ShowPage("tasks", "new");
        var reload = Button("⟳ 重新整理", "FlatButton");
        reload.Click += async (_, _) => await LoadAsync(fresh: true);
        var web = Button("在網頁開啟 ↗", "FlatButton");
        web.Click += (_, _) =>
        {
            if (SelectedProject is { } key)
                Web.Open(_engine.WebUrl($"/projects/{Uri.EscapeDataString(key)}"));
        };
        var title = new DockPanel { Margin = new Thickness(0, 0, 0, 10) };
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Children = { create, reload, web } };
        DockPanel.SetDock(actions, Dock.Right);
        title.Children.Add(actions);
        title.Children.Add(new TextBlock { Text = "專案", Style = (Style)FindResource("PageTitle"), Margin = new Thickness(0), VerticalAlignment = VerticalAlignment.Center });

        var filters = new WrapPanel { Margin = new Thickness(0, 0, 0, 8), Children = { _project, _view, _filter, _hideDone, _mine, _leafOnly } };

        var applyStatus = Button("改狀態", "PrimaryButton");
        applyStatus.Click += async (_, _) =>
        {
            if (_batchStatus.SelectedItem is string status)
                await BatchAsync(applyStatus, "status", status, $"狀態改成「{status}」");
        };
        var applyPriority = Button("改優先級", "FlatButton");
        applyPriority.Click += async (_, _) =>
        {
            if (_batchPriority.SelectedItem is string priority)
                await BatchAsync(applyPriority, "priority", priority, $"優先級改成「{priority}」");
        };
        var clear = Button("取消選取", "FlatButton");
        clear.Click += (_, _) =>
        {
            _selected.Clear();
            Render();
        };
        _batch = new Border
        {
            Style = (Style)FindResource("Card"), Padding = new Thickness(12, 6, 12, 6), Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed,
            Child = new WrapPanel { Children = { _batchText, _batchStatus, applyStatus, _batchPriority, applyPriority, clear } },
        };

        var dock = new DockPanel { Margin = new Thickness(24, 18, 24, 16) };
        foreach (var top in new UIElement[] { title, filters, _summary, _status, _batch })
        {
            DockPanel.SetDock(top, Dock.Top);
            dock.Children.Add(top);
        }
        dock.Children.Add(new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = _list });
        Content = dock;
    }

    Button Button(string text, string style) =>
        new() { Content = text, Style = (Style)FindResource(style), Padding = new Thickness(12, 5, 12, 5), Margin = new Thickness(6, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };

    string? SelectedProject => (_project.SelectedItem as ComboBoxItem)?.Tag as string;

    /// <summary>arg：專案代碼（例 F2P）時切到那個專案。</summary>
    public async Task ActivateAsync(object? arg)
    {
        var first = _project.ItemsSource is null;
        RenderProjects(arg as string);
        if (first || arg is string)
            await LoadAsync(fresh: false);
    }

    void RenderProjects(string? wanted)
    {
        var selected = wanted ?? SelectedProject;
        _building = true;
        var items = _engine.Projects.Where(p => !p.IsArchived)
            .Select(p => new ComboBoxItem { Content = ProjectIcons.Badge(p, p.Name, 16), Tag = p.Key }).ToList();
        _project.ItemsSource = items;
        _project.SelectedItem = items.FirstOrDefault(i => string.Equals(i.Tag as string, selected, StringComparison.OrdinalIgnoreCase))
                                ?? items.FirstOrDefault(i => _engine.RunningKeys().Any(k => KeyFinder.ProjectOf(k) == (string?)i.Tag))
                                ?? items.FirstOrDefault();
        _building = false;
        if (items.Count == 0)
            _summary.Text = "還沒有讀到專案，稍後再試或按重新整理";
    }

    async Task LoadAsync(bool fresh)
    {
        if (SelectedProject is not { } project)
            return;
        var version = ++_version;
        Inputs.Show(_status, null);
        _summary.Text = "讀取中…";
        if (fresh)
            _engine.ForgetProjectTree(project);
        try
        {
            var tasks = await _engine.ProjectTreeAsync(project);
            var detail = await _engine.GetProjectAsync(project);
            if (!_views.TryGetValue(project, out var views) || fresh)
            {
                try
                {
                    views = await _engine.Api.ListViewsAsync(project, CancellationToken.None);
                }
                catch (Exception ex)
                {
                    Log.Error($"讀取 {project} 的視圖失敗", ex);
                    views = [];
                }
                _views[project] = views;
            }
            if (version != _version)
                return;
            _tasks = tasks;
            _detail = detail;
            _selected.Clear();
            _building = true;
            var options = new List<ProjectView> { new(AllView, "全部任務", "", false, new Dictionary<string, IReadOnlyList<string>>(), false) };
            options.AddRange(views);
            var previous = (_view.SelectedItem as ProjectView)?.Id;
            _view.ItemsSource = options;
            _view.SelectedItem = options.FirstOrDefault(v => v.Id == previous) ?? options[0];
            _batchStatus.ItemsSource = detail.Statuses.Select(s => s.Name).ToList();
            _building = false;
            await ApplyViewAsync();
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取 {project} 的任務失敗", ex);
            _summary.Text = "";
            Inputs.Show(_status, $"讀取失敗：{ex.Message}");
        }
    }

    /// <summary>套用視圖：條件都認得就在本機篩，不然請伺服器用 view 篩（只有這種情況多一個請求）。</summary>
    async Task ApplyViewAsync()
    {
        if (_view.SelectedItem is not ProjectView view || view.Id == AllView)
        {
            _viewTasks = null;
            Render();
            return;
        }
        _viewTasks = ViewFilter.Apply(_tasks, view, _engine.User?.Username);
        if (_viewTasks is null && SelectedProject is { } project)
        {
            var version = _version;
            _summary.Text = "套用視圖中…";
            try
            {
                var filtered = await Task.Run(() => _engine.Api.ListProjectTasksAsync(project, CancellationToken.None, view.Id));
                if (version != _version)
                    return;
                _viewTasks = filtered;
            }
            catch (Exception ex)
            {
                Log.Error($"套用視圖 {view.Name} 失敗", ex);
                Inputs.Show(_status, $"套用視圖失敗：{ex.Message}");
            }
        }
        Render();
    }

    // ───────────── 畫清單 ─────────────

    void Render()
    {
        _list.Children.Clear();
        var me = _engine.User?.Username;
        var source = _viewTasks ?? _tasks;
        IEnumerable<ProjectTask> shown = source;
        if (_mine.IsChecked == true && me is not null)
            shown = shown.Where(t => t.Assignees?.Contains(me, StringComparer.OrdinalIgnoreCase) == true);
        var includeDone = _hideDone.IsChecked != true;
        var filter = _filter.Text.Trim();
        var count = 0;
        if (_leafOnly.IsChecked == true)
        {
            // 沒有子任務的（用整個專案判斷，不受篩選影響），平鋪列出
            var parents = _tasks.Select(t => t.Parent).OfType<string>().ToHashSet(StringComparer.OrdinalIgnoreCase);
            foreach (var t in shown.Where(t => !parents.Contains(t.Key) && !t.IsCategory
                                               && (includeDone || t.StatusCategory is not ("DONE" or "CANCELLED"))
                                               && (filter.Length == 0 || t.Key.Contains(filter, StringComparison.OrdinalIgnoreCase) || t.Title.Contains(filter, StringComparison.OrdinalIgnoreCase))))
            {
                _list.Children.Add(Row(t, 0, hasChildren: false));
                count++;
            }
        }
        else
        {
            var roots = TaskTree.Build(shown, includeDone, filter);
            void Add(IEnumerable<TaskTreeNode> nodes, int depth)
            {
                foreach (var n in nodes)
                {
                    _list.Children.Add(Row(n.Task, depth, n.Children.Count > 0));
                    count++;
                    if (n.Children.Count > 0 && IsOpen(n.Task.Key, depth))
                        Add(n.Children, depth + 1);
                }
            }
            Add(roots, 0);
        }
        _summary.Text = source.Count == 0 ? "這個專案還沒有任務"
            : count == 0 ? "沒有符合的任務"
            : $"顯示 {count} 個（共 {source.Count} 個）";
        _batch.Visibility = _selected.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        _batchText.Text = $"已選 {_selected.Count} 個";
    }

    /// <summary>任務多（超過 150 個）時預設只展開最上層，使用者點開的記在這裡。</summary>
    readonly HashSet<string> _expanded = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>這個節點展開了沒：使用者收起、點開的優先；有篩選或任務不多時全部展開；不然只展開最上層。</summary>
    bool IsOpen(string key, int depth) =>
        !_collapsed.Contains(key)
        && (_expanded.Contains(key) || _filter.Text.Trim().Length > 0 || (_viewTasks ?? _tasks).Count <= 150 || depth == 0);

    FrameworkElement Row(ProjectTask t, int depth, bool hasChildren)
    {
        var grid = new Grid { Margin = new Thickness(0, 1, 0, 1) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(26) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(depth * 18 + 20) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(120) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(130) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(70) });

        var check = new CheckBox { IsChecked = _selected.Contains(t.Key), VerticalAlignment = VerticalAlignment.Center };
        check.Click += (_, _) =>
        {
            if (check.IsChecked == true)
                _selected.Add(t.Key);
            else
                _selected.Remove(t.Key);
            _batch.Visibility = _selected.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
            _batchText.Text = $"已選 {_selected.Count} 個";
        };
        grid.Children.Add(check);

        if (hasChildren)
        {
            var open = IsOpen(t.Key, depth);
            var toggle = new Button
            {
                Content = open ? "▾" : "▸", Style = (Style)FindResource("FlatButton"), Padding = new Thickness(4, 0, 4, 0),
                HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 2, 0),
            };
            toggle.Click += (_, _) =>
            {
                if (open)
                {
                    _collapsed.Add(t.Key);
                    _expanded.Remove(t.Key);
                }
                else
                {
                    _collapsed.Remove(t.Key);
                    _expanded.Add(t.Key);
                }
                Render();
            };
            Grid.SetColumn(toggle, 1);
            grid.Children.Add(toggle);
        }

        var name = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center, ToolTip = $"{t.Key} {t.Title}\n點一下看詳細" };
        var link = new Hyperlink(new Run(t.Key)) { FontWeight = FontWeights.SemiBold, TextDecorations = null, Cursor = System.Windows.Input.Cursors.Hand };
        link.SetResourceReference(TextElement.ForegroundProperty, "AccentBrush");
        link.Click += (_, _) => _host.ShowPage("tasks", t.Key);
        name.Inlines.Add(link);
        name.Inlines.Add(new Run($" {t.Title}") { FontWeight = t.IsCategory ? FontWeights.SemiBold : FontWeights.Normal });
        if (t.IsCategory)
            name.Inlines.Add(new Run("  · 分類") { Foreground = Brushes2.Get("MutedBrush"), FontSize = 11 });
        var done = t.StatusCategory is "DONE" or "CANCELLED";
        name.Opacity = done ? 0.55 : 1;
        Grid.SetColumn(name, 2);
        grid.Children.Add(name);

        var status = new TextBlock { Text = t.Status, FontSize = 12, TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 0, 0) };
        status.SetResourceReference(TextBlock.ForegroundProperty, t.StatusCategory == "IN_PROGRESS" ? "AccentBrush" : "MutedBrush");
        Grid.SetColumn(status, 3);
        grid.Children.Add(status);

        var people = (t.Assignees ?? []).Select(a => _detail?.Members.FirstOrDefault(m => string.Equals(m.Username, a, StringComparison.OrdinalIgnoreCase))?.DisplayName ?? a).ToList();
        var assignees = new TextBlock
        {
            Text = people.Count > 0 ? string.Join("、", people) : "—", FontSize = 12, TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 0, 0), ToolTip = people.Count > 0 ? string.Join("、", people) : "未指派",
        };
        assignees.SetResourceReference(TextBlock.ForegroundProperty, "MutedBrush");
        Grid.SetColumn(assignees, 4);
        grid.Children.Add(assignees);

        var overdue = !done && !t.IsCategory && t.PlannedEnd is { } end && end < DateOnly.FromDateTime(DateTime.Today);
        var due = new TextBlock
        {
            Text = t.PlannedEnd is { } e ? $"{e:M/d}" : "", FontSize = 12, HorizontalAlignment = HorizontalAlignment.Right,
            VerticalAlignment = VerticalAlignment.Center, ToolTip = t.PlannedEnd is null ? null : overdue ? "預計結束已過" : "預計結束",
        };
        due.SetResourceReference(TextBlock.ForegroundProperty, overdue ? "ErrorBrush" : "MutedBrush");
        Grid.SetColumn(due, 5);
        grid.Children.Add(due);
        return grid;
    }

    // ───────────── 批次修改 ─────────────

    async Task BatchAsync(Button button, string field, string value, string what)
    {
        if (SelectedProject is not { } project || _selected.Count == 0)
            return;
        var items = _selected.Take(100).Select(k => new Dictionary<string, object?> { ["task"] = k, [field] = value }).ToList();
        List<BatchResult> results = [];
        if (!await Inputs.Run(button, _status, async () => results = await _engine.Api.UpdateTasksAsync(items, CancellationToken.None)))
            return;
        var failed = results.Where(r => r.Error is not null).ToList();
        var requested = results.Count(r => r.Error is null && r.Message is { Length: > 0 });
        var text = $"{results.Count - failed.Count} 個{what}" + (requested > 0 ? $"（其中 {requested} 個送出變更申請）" : "")
                   + (failed.Count > 0 ? $"；{failed.Count} 個失敗：" + string.Join("、", failed.Take(5).Select(f => $"{f.Task} {f.Error}")) : "");
        Log.Info($"批次修改 {project}：{text}");
        _selected.Clear();
        await _engine.AfterExternalChangeAsync();
        await LoadAsync(fresh: true);
        Inputs.Show(_status, text, error: failed.Count > 0);
    }
}
