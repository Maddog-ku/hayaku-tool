using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Documents;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 專案動態：誰開始做了什麼、新增了哪些任務、完成了哪些（跟網頁「專案動態」視圖同一套），
/// 上方是這段期間的進展摘要；可以選專案、期間、事件類型，只看自己的。
/// </summary>
public sealed class ActivityPage : UserControl, IWorkPage
{
    static readonly (string Key, string Label)[] Types = [("progress", "進展"), ("assign", "指派"), ("talk", "討論"), ("other", "其他")];
    static readonly (string Key, string Label)[] Periods = [("week", "本週"), ("lastWeek", "上週"), ("month", "最近 30 天")];

    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;
    readonly ComboBox _project = new() { Width = 220, Margin = new Thickness(0, 0, 8, 0) };
    readonly Dictionary<string, CheckBox> _types = [];
    readonly CheckBox _onlyMe = new() { Content = "只看我", VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(4, 0, 0, 0) };
    readonly UniformGrid _cards = new() { Columns = 4, Margin = new Thickness(0, 0, 0, 8) };
    readonly TextBlock _top = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 12) };
    readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    readonly StackPanel _list = new();
    string _period = "week";
    int _version;
    bool _building;

    public ActivityPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        _top.SetResourceReference(TextBlock.ForegroundProperty, "MutedBrush");
        _project.SelectionChanged += async (_, _) =>
        {
            if (!_building)
                await LoadAsync();
        };
        var periods = new StackPanel { Orientation = Orientation.Horizontal };
        foreach (var (key, label) in Periods)
        {
            var b = new RadioButton { Style = (Style)FindResource("Segment"), GroupName = "ActivityPeriod", Content = label, IsChecked = key == _period };
            b.Checked += async (_, _) =>
            {
                _period = key;
                await LoadAsync();
            };
            periods.Children.Add(b);
        }
        var types = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(12, 0, 0, 0) };
        foreach (var (key, label) in Types)
        {
            var c = new CheckBox { Content = label, IsChecked = key == "progress", VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 10, 0) };
            c.Click += async (_, _) =>
            {
                if (_types.Values.All(x => x.IsChecked != true))
                    c.IsChecked = true; // 至少要一種
                await LoadAsync();
            };
            _types[key] = c;
            types.Children.Add(c);
        }
        _onlyMe.Click += async (_, _) => await LoadAsync();

        var reload = new Button { Content = "⟳ 重新整理", Style = (Style)FindResource("FlatButton"), Padding = new Thickness(12, 5, 12, 5),
            HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center };
        reload.Click += async (_, _) => await LoadAsync();
        var title = new Grid { Margin = new Thickness(0, 0, 0, 10) };
        title.Children.Add(new TextBlock { Text = "專案動態", Style = (Style)FindResource("PageTitle"), Margin = new Thickness(0), VerticalAlignment = VerticalAlignment.Center });
        title.Children.Add(reload);
        var filters = new WrapPanel { Margin = new Thickness(0, 0, 0, 12) };
        filters.Children.Add(_project);
        filters.Children.Add(new Border { Style = (Style)FindResource("SegmentTrack"), Child = periods });
        filters.Children.Add(types);
        filters.Children.Add(_onlyMe);

        var dock = new DockPanel { Margin = new Thickness(24, 18, 24, 16) };
        foreach (var top in new UIElement[] { title, filters, _status, _cards, _top })
        {
            DockPanel.SetDock(top, Dock.Top);
            dock.Children.Add(top);
        }
        dock.Children.Add(new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = _list });
        Content = dock;
    }

    public Task ActivateAsync(object? arg)
    {
        RenderProjects();
        return LoadAsync();
    }

    void RenderProjects()
    {
        var selected = (_project.SelectedItem as ComboBoxItem)?.Tag as string;
        _building = true;
        var items = new List<ComboBoxItem> { new() { Content = "全部專案", Tag = null } };
        items.AddRange(_engine.Projects.Where(p => !p.IsArchived)
            .Select(p => new ComboBoxItem { Content = ProjectIcons.Badge(p, p.Name, 16), Tag = p.Key }));
        _project.ItemsSource = items;
        _project.SelectedItem = items.FirstOrDefault(i => i.Tag as string == selected) ?? items[0];
        _building = false;
    }

    (DateOnly From, DateOnly To) Range()
    {
        var today = DateOnly.FromDateTime(DateTime.Today);
        var monday = today.AddDays(-(((int)today.DayOfWeek + 6) % 7));
        return _period switch
        {
            "lastWeek" => (monday.AddDays(-7), monday.AddDays(-1)),
            "month" => (today.AddDays(-29), today),
            _ => (monday, today),
        };
    }

    async Task LoadAsync()
    {
        var version = ++_version;
        Inputs.Show(_status, null);
        _top.Text = "讀取中…";
        var (from, to) = Range();
        var project = (_project.SelectedItem as ComboBoxItem)?.Tag as string;
        var types = _types.Where(t => t.Value.IsChecked == true).Select(t => t.Key).ToList();
        try
        {
            var feed = await _engine.Api.ProjectActivityAsync(project, from, to, types, CancellationToken.None,
                _onlyMe.IsChecked == true ? ["me"] : null);
            if (version != _version)
                return;
            Render(feed);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error("讀取專案動態失敗", ex);
            _top.Text = "";
            Inputs.Show(_status, $"讀取失敗：{ex.Message}");
        }
    }

    void Render(ActivityFeed feed)
    {
        _cards.Children.Clear();
        foreach (var (label, value, brush) in new[]
                 {
                     ("新增", feed.Created, "AccentBrush"), ("開始", feed.Started, "RunBrush"),
                     ("完成", feed.Completed, "TextBrush"), ("逾期", feed.Overdue, "ErrorBrush"),
                 })
        {
            var card = new Border { Style = (Style)FindResource("Card"), Padding = new Thickness(14, 10, 14, 10), Margin = new Thickness(0, 0, 8, 0) };
            var number = new TextBlock { Text = value.ToString(), FontSize = 22, FontWeight = FontWeights.SemiBold };
            number.SetResourceReference(TextBlock.ForegroundProperty, value > 0 ? brush : "MutedBrush");
            card.Child = new StackPanel { Children = { new TextBlock { Text = label, Foreground = Brushes2.Get("MutedBrush"), FontSize = 12 }, number } };
            _cards.Children.Add(card);
        }
        var period = Periods.First(p => p.Key == _period).Label;
        _top.Text = $"{period}（{feed.From:M/d}～{feed.To:M/d}）的進展" +
                    (feed.TopMembers.Count > 0 ? "　最活躍：" + string.Join("、", feed.TopMembers.Take(5).Select(m => $"{m.Name} {m.Count}")) : "") +
                    (feed.Items.Count >= 50 ? "　（只列最新 50 則）" : "");

        _list.Children.Clear();
        if (feed.Items.Count == 0)
        {
            _list.Children.Add(new TextBlock { Text = "這段期間沒有動態", Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 10, 0, 0) });
            return;
        }
        foreach (var day in feed.Items.GroupBy(i => i.At.Split(' ')[0]))
        {
            _list.Children.Add(new TextBlock { Text = day.Key, Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(0, 10, 0, 4) });
            var card = new StackPanel();
            foreach (var item in day)
                card.Children.Add(Row(item));
            _list.Children.Add(new Border { Style = (Style)FindResource("Card"), Padding = new Thickness(12, 6, 12, 6), Child = card });
        }
    }

    FrameworkElement Row(ActivityItem item)
    {
        var grid = new Grid { Margin = new Thickness(0, 3, 0, 3) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(48) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(28) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var parts = item.At.Split(' ');
        grid.Children.Add(new TextBlock { Text = parts.Length > 1 ? parts[1] : item.At, FontFamily = (System.Windows.Media.FontFamily)FindResource("MonoFont"),
            Foreground = Brushes2.Get("MutedBrush"), VerticalAlignment = VerticalAlignment.Center });
        var icon = ProjectIcons.Icon(_engine.ProjectOf(item.Project), item.Project, 18);
        icon.ToolTip = _engine.ProjectOf(item.Project)?.Name ?? item.Project;
        Grid.SetColumn(icon, 1);
        grid.Children.Add(icon);
        var text = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center, ToolTip = item.Text };
        // 文字裡的任務編號做成連結，點了在我的任務打開
        var rest = item.Text;
        while (KeyFinder.Parse(rest) is { } key && rest.IndexOf(key, StringComparison.OrdinalIgnoreCase) is var i and >= 0)
        {
            if (_engine.Projects.Count > 0 && _engine.ProjectOf(KeyFinder.ProjectOf(key)) is null)
            {
                // 不是專案代碼（例「ISMS-P-13」）：當一般文字
                text.Inlines.Add(new Run(rest[..(i + key.Length)]));
                rest = rest[(i + key.Length)..];
                continue;
            }
            text.Inlines.Add(new Run(rest[..i]));
            var link = new Hyperlink(new Run(rest.Substring(i, key.Length))) { FontWeight = FontWeights.SemiBold };
            link.Click += (_, _) => _host.ShowPage("tasks", key);
            text.Inlines.Add(link);
            rest = rest[(i + key.Length)..];
        }
        text.Inlines.Add(new Run(rest));
        Grid.SetColumn(text, 2);
        grid.Children.Add(text);
        return grid;
    }
}
