using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Claude;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;
using HayakuBar.UI.Work;

namespace HayakuBar.UI;

/// <summary>
/// 偵測到沒有任務的新工作時，從提示按「建立任務」開的小視窗：專案自動選好，使用者選專案樹上的位置、狀態、預計起訖、
/// 預計工時（必填，開視窗時先請 Claude 估）。
/// 建好後（預設）指定給那個 Claude session，之後就自動為它計時。
/// </summary>
public partial class QuickCreateWindow : Window
{
    readonly TimerEngine _engine;
    readonly NewWorkSuggestion _suggestion;
    List<ProjectTask> _tasks = [];
    List<TreeNodeVm> _nodes = [];
    string? _parent;
    bool _keepParent;
    int _version;
    CancellationTokenSource? _estimateCts;
    bool _estimateEdited; // 使用者自己改過預計工時：自動估回來的不覆蓋
    bool _settingEstimate;

    public QuickCreateWindow(TimerEngine engine, NewWorkSuggestion suggestion)
    {
        InitializeComponent();
        _engine = engine;
        _suggestion = suggestion;
        HeaderText.Text = $"建立任務 · {suggestion.UnitName}";
        SubText.Text = suggestion.Title is { } t ? $"Claude 正在改程式：{t}" : "Claude 正在改程式，但對應不到任務";
        SubText.ToolTip = suggestion.UnitPath;
        NewTitle.Text = suggestion.Title?.TrimEnd('…') ?? "";
        var today = DateOnly.FromDateTime(DateTime.Today);
        PlanDates.SetRange(today, today);
        UpdateParentText();
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);
        Loaded += async (_, _) =>
        {
            NewTitle.Focus();
            NewTitle.SelectAll();
            await LoadProjectsAsync();
            await EstimateAsync(manual: false);
        };
        Closed += (_, _) => _estimateCts?.Cancel();
    }

    /// <summary>建好的任務編號；沒建立就關掉是 null。</summary>
    public string? CreatedKey { get; private set; }

    public string CreatedTitle { get; private set; } = "";

    /// <summary>有沒有指定給 session（App 用來決定提示文字）。</summary>
    public bool Assigned { get; private set; }

    /// <summary>擺在工作列工時的正上方（跟提示一樣的位置），不超出螢幕。</summary>
    public void PlaceNear(Rect anchor, bool alignLeft)
    {
        var work = SystemParameters.WorkArea;
        var left = alignLeft ? anchor.Left : anchor.Right - Width;
        Left = Math.Clamp(left, work.Left + 8, Math.Max(work.Left + 8, work.Right - Width - 8));
        Top = Math.Max(work.Top + 8, work.Bottom - Height - 8);
    }

    async Task LoadProjectsAsync()
    {
        try
        {
            var projects = await _engine.Api.ListProjectsAsync(CancellationToken.None);
            ProjectBox.ItemsSource = projects;
            var key = ProjectGuess.Guess(projects.Select(p => (p.Key, p.Name)).ToList(), _suggestion.Project, _suggestion.UnitName,
                _engine.RunningKeys().Select(KeyFinder.ProjectOf));
            var target = projects.FirstOrDefault(p => p.Key == key) ?? projects.FirstOrDefault();
            if (target is null)
            {
                TreeMessage.Text = "沒有可以建立任務的專案";
                return;
            }
            // 上次在這個資料夾選的上層，只在同一個專案時帶入
            _parent = string.Equals(target.Key, _suggestion.Project, StringComparison.OrdinalIgnoreCase) ? _suggestion.Parent : null;
            _keepParent = true;
            ProjectBox.SelectedItem = target;
        }
        catch (Exception ex)
        {
            Log.Error("讀取專案清單失敗", ex);
            Inputs.Show(StatusText, $"讀不到專案清單：{ex.Message}");
        }
    }

    async void ProjectBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (ProjectBox.SelectedItem is not ProjectInfo project)
            return;
        if (!_keepParent)
            _parent = null; // 換專案時上層歸零
        _keepParent = false;
        await LoadTreeAsync(project.Key);
    }

    /// <summary>讀專案的全部任務與狀態清單，畫出專案樹。</summary>
    async Task LoadTreeAsync(string projectKey)
    {
        var version = ++_version;
        TreeMessage.Text = "讀取中…";
        TreeMessage.Visibility = Visibility.Visible;
        ProjectTree.ItemsSource = null;
        try
        {
            var tasks = await _engine.Api.ListProjectTasksAsync(projectKey, CancellationToken.None);
            var project = await _engine.GetProjectAsync(projectKey);
            if (version != _version)
                return;
            _tasks = tasks;
            StatusBox.ItemsSource = project.Statuses.Select(s => s.Name).ToList();
            // Claude 已經在改程式了，預設「進行中」類的狀態
            StatusBox.SelectedItem = (project.Statuses.FirstOrDefault(s => s.Category == "IN_PROGRESS")
                                      ?? project.Statuses.FirstOrDefault(s => s.IsDefault)
                                      ?? project.Statuses.FirstOrDefault())?.Name;
            if (_parent is not null && !tasks.Any(t => string.Equals(t.Key, _parent, StringComparison.OrdinalIgnoreCase)))
                _parent = null;
            RenderTree(scrollToPicked: true);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取 {projectKey} 的任務樹失敗", ex);
            _tasks = [];
            TreeMessage.Text = $"讀不到專案的任務：{ex.Message}";
        }
        UpdateParentText();
    }

    void RenderTree(bool scrollToPicked = false)
    {
        var filter = TreeFilter.Text.Trim();
        var roots = TaskTree.Build(_tasks, ShowDoneBox.IsChecked == true, filter);
        var expandAll = filter.Length > 0 || _tasks.Count <= 150;
        var picked = _parent is null ? new HashSet<string>() : AncestorsOf(_parent);
        _nodes = roots.Select(r => TreeNodeVm.From(r, _parent, expandAll, picked)).ToList();
        foreach (var node in _nodes)
            node.SetWidth(ProjectTree.ActualWidth);
        ProjectTree.ItemsSource = _nodes;
        RootRow.Background = _parent is null ? Brushes2.Get("SelectedBrush") : Brushes.Transparent;
        TreeMessage.Text = _tasks.Count == 0 ? "這個專案還沒有任務，新增的會在最上層"
            : roots.Count == 0 ? "沒有符合的任務" : "";
        TreeMessage.Visibility = TreeMessage.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        if (scrollToPicked && _parent is { } key)
            Dispatcher.InvokeAsync(() => BringIntoView(ProjectTree, key), System.Windows.Threading.DispatcherPriority.Loaded);
    }

    /// <summary>某任務的所有上層（含自己），展開用。</summary>
    HashSet<string> AncestorsOf(string key)
    {
        var byKey = _tasks.ToDictionary(t => t.Key, StringComparer.OrdinalIgnoreCase);
        var set = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        for (var k = key; k is not null && set.Add(k) && byKey.TryGetValue(k, out var t);)
            k = t.Parent;
        return set;
    }

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
        if (_parent is null)
        {
            ParentText.Text = "最上層（不屬於任何任務）";
            return;
        }
        var byKey = _tasks.ToDictionary(t => t.Key, StringComparer.OrdinalIgnoreCase);
        ParentText.Text = string.Join(" › ", AncestorsOf(_parent).Reverse()
            .Select(k => byKey.TryGetValue(k, out var t) ? $"{t.Key} {t.Title}" : k));
    }

    void PickParent(string? key)
    {
        _parent = key;
        foreach (var node in _nodes)
            node.SetPicked(key);
        RootRow.Background = key is null ? Brushes2.Get("SelectedBrush") : Brushes.Transparent;
        UpdateParentText();
    }

    void TreeRow_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is TreeNodeVm node)
        {
            e.Handled = true; // 不要再傳給外層的列
            PickParent(node.Key);
        }
    }

    void RootRow_Click(object sender, MouseButtonEventArgs e) => PickParent(null);

    void TreeFilter_TextChanged(object sender, TextChangedEventArgs e) => RenderTree();

    void ShowDone_Click(object sender, RoutedEventArgs e) => RenderTree();

    void ProjectTree_SizeChanged(object sender, SizeChangedEventArgs e)
    {
        foreach (var node in _nodes)
            node.SetWidth(e.NewSize.Width);
    }

    /// <summary>回應裡沒有編號時：到專案裡找剛建好的（同名稱、同上層、編號最大的那個）。</summary>
    async Task<string?> FindCreatedAsync(string projectKey, string title)
    {
        try
        {
            var tasks = await _engine.Api.ListProjectTasksAsync(projectKey, CancellationToken.None);
            return tasks
                .Where(t => t.Title == title && string.Equals(t.Parent, _parent, StringComparison.OrdinalIgnoreCase))
                .MaxBy(t => int.TryParse(t.Key[(t.Key.LastIndexOf('-') + 1)..], out var n) ? n : 0)?.Key;
        }
        catch (Exception ex)
        {
            Log.Error("找剛建立的任務失敗", ex);
            return null;
        }
    }

    async void NewTitle_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        await CreateAsync();
    }

    async void Create_Click(object sender, RoutedEventArgs e) => await CreateAsync();

    async void Estimate_Click(object sender, RoutedEventArgs e) => await EstimateAsync(manual: true);

    void EstimateBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        if (!_settingEstimate)
            _estimateEdited = true;
    }

    /// <summary>
    /// 請 Claude 依任務名稱與 session 開頭的指令估預計工時。開視窗時自動問一次（使用者已經自己填了就不覆蓋），
    /// 按「✦ 問 Claude」重問會直接帶入。
    /// </summary>
    async Task EstimateAsync(bool manual)
    {
        var title = NewTitle.Text.Trim();
        if (title.Length == 0)
        {
            EstimateHint.Text = manual ? "先填名稱，Claude 才能估" : "";
            return;
        }
        _estimateCts?.Cancel();
        var cts = _estimateCts = new CancellationTokenSource();
        EstimateButton.IsEnabled = false;
        EstimateHint.Text = "Claude 估算中…";
        var project = ProjectBox.SelectedItem is ProjectInfo p ? $"{p.Name}（資料夾 {_suggestion.UnitName}）" : _suggestion.UnitName;
        var file = _suggestion.SessionFile;
        var hours = await Task.Run(() => ClaudeEstimator.EstimateAsync(title, project,
            file is null ? null : ClaudeMonitor.ReadOpeningPrompts(file), cts.Token));
        if (cts != _estimateCts)
            return; // 又重問了，或視窗關了
        _estimateCts = null;
        EstimateButton.IsEnabled = true;
        if (hours is not { } h)
        {
            EstimateHint.Text = "問不到 Claude，請自己填";
            return;
        }
        var text = h.ToString("0.##", CultureInfo.InvariantCulture);
        if (_estimateEdited && !manual && EstimateBox.Text.Trim().Length > 0)
        {
            EstimateHint.Text = $"Claude 估 {text} 小時（保留你填的）";
            return;
        }
        _settingEstimate = true;
        EstimateBox.Text = text;
        _settingEstimate = false;
        _estimateEdited = false;
        EstimateHint.Text = "Claude 估的，可以改";
    }

    /// <summary>預計工時的小時數（可加「小時」「h」）；空白或不是正數回 null。</summary>
    static double? ParseEstimate(string text)
    {
        text = text.Trim().TrimEnd('h', 'H').Replace("小時", "").Trim();
        return double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var hours) && hours > 0 ? hours : null;
    }

    async Task CreateAsync()
    {
        if (ProjectBox.SelectedItem is not ProjectInfo project)
        {
            Inputs.Show(StatusText, "請選專案");
            return;
        }
        var title = NewTitle.Text.Trim();
        if (title.Length == 0)
        {
            Inputs.Show(StatusText, "請填名稱");
            NewTitle.Focus();
            return;
        }
        if (ParseEstimate(EstimateBox.Text) is not { } estimate)
        {
            Inputs.Show(StatusText, EstimateBox.Text.Trim().Length == 0 ? "請填預計工時（可以按「✦ 問 Claude」）" : "預計工時請填小時數");
            EstimateBox.Focus();
            return;
        }
        var fields = new Dictionary<string, object?>
        {
            ["project"] = project.Key, ["title"] = title, ["assignees"] = new[] { "me" }, ["estimatedHours"] = estimate,
        };
        if (_parent is not null)
            fields["parent"] = _parent;
        if (StatusBox.SelectedItem is string status)
            fields["status"] = status;
        if (PlanDates.Start is { } start)
            fields["plannedStart"] = Inputs.DateText(start);
        if (PlanDates.End is { } end)
            fields["plannedEnd"] = Inputs.DateText(end);

        var key = "";
        if (!await Inputs.Run(CreateButton, StatusText, async () => key = await _engine.Api.CreateTaskAsync(fields, CancellationToken.None)))
            return;
        _engine.RememberFolder(_suggestion.UnitPath, project.Key, _parent);
        if (key.Length == 0)
            key = await FindCreatedAsync(project.Key, title) ?? "";
        CreatedKey = key.Length > 0 ? key : null;
        CreatedTitle = title;
        if (CreatedKey is not null && AssignBox.IsChecked == true)
        {
            Assigned = true;
            await _engine.AssignSessionTaskAsync(_suggestion.SessionId, CreatedKey, startNow: true);
        }
        else if (CreatedKey is null && AssignBox.IsChecked == true)
            Log.Error($"建立「{title}」後找不到新任務的編號，沒辦法指定給 Claude session");
        _ = _engine.AfterExternalChangeAsync();
        Close();
    }
}
