using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Documents;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 我的工作負荷（跟網頁「工作負荷」視圖同一份算法）：一週每天分配到（預計）或實際計時的工時對上容量（應出勤），
/// 每格顯示幾個任務，合計欄顯示這週做過幾個不同任務；預計模式另列還沒填預計工時、算不進來的任務。
/// </summary>
public sealed class WorkloadPage : UserControl, IWorkPage
{
    static readonly string[] WeekdayNames = ["一", "二", "三", "四", "五", "六", "日"];

    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;
    readonly TextBlock _range = new() { FontSize = 16, FontWeight = FontWeights.SemiBold, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(10, 0, 10, 0) };
    readonly UniformGrid _days = new() { Columns = 8, Margin = new Thickness(0, 0, 0, 12) };
    readonly StackPanel _tasks = new();
    readonly StackPanel _incomplete = new();
    readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    DateOnly _monday;
    string _mode = "actual";
    int _version;

    public WorkloadPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        var today = DateOnly.FromDateTime(DateTime.Today);
        _monday = today.AddDays(-(((int)today.DayOfWeek + 6) % 7));

        var modes = new StackPanel { Orientation = Orientation.Horizontal };
        foreach (var (key, label, tip) in new[] { ("actual", "實際計時", "實際計時（同時計時只算一次，含補登、會議）"), ("planned", "預計", "任務的預計工時照每天的容量比例分配") })
        {
            var b = new RadioButton { Style = (Style)FindResource("Segment"), GroupName = "WorkloadMode", Content = label, ToolTip = tip, IsChecked = key == _mode };
            b.Checked += async (_, _) =>
            {
                _mode = key;
                await LoadAsync();
            };
            modes.Children.Add(b);
        }
        Button Nav(string text, string tip, int days)
        {
            var b = new Button { Content = text, Style = (Style)FindResource("FlatButton"), Padding = new Thickness(10, 2, 10, 2), ToolTip = tip, FontSize = 15 };
            b.Click += async (_, _) =>
            {
                _monday = days == 0 ? DateOnly.FromDateTime(DateTime.Today).AddDays(-(((int)DateTime.Today.DayOfWeek + 6) % 7)) : _monday.AddDays(days);
                await LoadAsync();
            };
            return b;
        }
        var thisWeek = Nav("本週", "回到本週", 0);
        thisWeek.FontSize = 13;
        thisWeek.Margin = new Thickness(8, 0, 0, 0);
        var header = new DockPanel { Margin = new Thickness(0, 0, 0, 14) };
        var right = new Border { Style = (Style)FindResource("SegmentTrack"), Child = modes };
        DockPanel.SetDock(right, Dock.Right);
        header.Children.Add(right);
        header.Children.Add(new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Children =
            {
                new TextBlock { Text = "工作負荷", Style = (Style)FindResource("PageTitle"), Margin = new Thickness(0, 0, 16, 0), VerticalAlignment = VerticalAlignment.Center },
                Nav("‹", "上一週", -7), _range, Nav("›", "下一週", 7), thisWeek,
            },
        });

        var body = new StackPanel
        {
            Children =
            {
                _days,
                new TextBlock { Text = "這週的任務", Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(0, 4, 0, 6) },
                new Border { Style = (Style)FindResource("Card"), Padding = new Thickness(14, 8, 14, 8), Child = _tasks },
                _incomplete,
            },
        };
        var dock = new DockPanel { Margin = new Thickness(24, 18, 24, 16) };
        foreach (var top in new UIElement[] { header, _status })
        {
            DockPanel.SetDock(top, Dock.Top);
            dock.Children.Add(top);
        }
        dock.Children.Add(new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = body });
        Content = dock;
    }

    public Task ActivateAsync(object? arg) => LoadAsync();

    async Task LoadAsync()
    {
        var version = ++_version;
        var (from, to) = (_monday, _monday.AddDays(6));
        _range.Text = $"{from:M/d}～{to:M/d}";
        Inputs.Show(_status, null);
        try
        {
            var load = await _engine.Api.MyWorkloadAsync(from, to, _mode, CancellationToken.None);
            // 實際模式：每天做了幾個任務用自己的計時段算（伺服器只給每天的時數）
            Dictionary<DateOnly, int>? perDay = null;
            var distinct = load.Tasks.Count;
            if (_mode == "actual")
            {
                var entries = await _engine.Api.ListTimeEntriesAsync(from, to, CancellationToken.None);
                perDay = entries.Where(e => e.StartedAt is not null)
                    .GroupBy(e => DateOnly.FromDateTime(e.StartedAt!.Value.LocalDateTime))
                    .ToDictionary(g => g.Key, g => g.Select(e => e.Task).Distinct(StringComparer.OrdinalIgnoreCase).Count());
                distinct = Math.Max(distinct, entries.Select(e => e.Task).Distinct(StringComparer.OrdinalIgnoreCase).Count());
            }
            if (version != _version)
                return;
            Render(load, perDay, distinct);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error("讀取工作負荷失敗", ex);
            Inputs.Show(_status, $"讀取失敗：{ex.Message}");
        }
    }

    void Render(Workload load, Dictionary<DateOnly, int>? perDay, int distinctTasks)
    {
        _days.Children.Clear();
        var byDate = load.Days.ToDictionary(d => d.Date);
        for (var i = 0; i < 7; i++)
        {
            var date = load.From.AddDays(i);
            var day = byDate.GetValueOrDefault(date);
            var count = perDay?.GetValueOrDefault(date) ?? day?.TaskCount ?? 0;
            _days.Children.Add(Cell($"週{WeekdayNames[i]} {date:M/d}", day?.Minutes ?? 0, day?.CapacityMinutes ?? 0,
                count > 0 ? $"{count} 個任務" : "", date == DateOnly.FromDateTime(DateTime.Today)));
        }
        _days.Children.Add(Cell("合計", load.TotalMinutes, load.CapacityMinutes,
            distinctTasks > 0 ? $"做過 {distinctTasks} 個不同任務" : "", false, total: true));

        _tasks.Children.Clear();
        if (load.Tasks.Count == 0)
            _tasks.Children.Add(new TextBlock { Text = load.Mode == "actual" ? "這週還沒有計時" : "這週沒有分配到預計工時的任務", Foreground = Brushes2.Get("MutedBrush") });
        foreach (var t in load.Tasks.OrderByDescending(t => t.Minutes))
            _tasks.Children.Add(TaskRow(t.Key, t.Title, WorkMath.HoursMinutes(t.Minutes)));

        _incomplete.Children.Clear();
        if (load.Mode == "planned" && load.Incomplete.Count > 0)
        {
            _incomplete.Children.Add(new TextBlock { Text = $"算不進來的任務 {load.Incomplete.Count}", Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(0, 14, 0, 6) });
            var card = new StackPanel();
            foreach (var (key, title, reason) in load.Incomplete)
                card.Children.Add(TaskRow(key, title, reason));
            _incomplete.Children.Add(new Border { Style = (Style)FindResource("Card"), Padding = new Thickness(14, 8, 14, 8), Child = card });
        }
    }

    FrameworkElement Cell(string title, int minutes, int capacity, string tasks, bool today, bool total = false)
    {
        var ratio = capacity > 0 ? minutes / (double)capacity : minutes > 0 ? 1 : 0;
        var brush = ratio > 1.05 ? "ErrorBrush" : ratio >= 0.95 ? "RunBrush" : minutes > 0 ? "AccentBrush" : "LineBrush";
        var bar = new Grid { Height = 5, Margin = new Thickness(0, 6, 0, 4) };
        bar.Children.Add(new Border { Background = Brushes2.Get("LineBrush"), CornerRadius = new CornerRadius(2.5) });
        bar.Children.Add(new Border
        {
            Background = Brushes2.Get(brush),
            CornerRadius = new CornerRadius(2.5),
            HorizontalAlignment = HorizontalAlignment.Left,
            Width = 0,
            Tag = Math.Min(1, ratio),
        });
        bar.SizeChanged += (_, e) => ((Border)bar.Children[1]).Width = e.NewSize.Width * (double)((Border)bar.Children[1]).Tag;
        var panel = new StackPanel
        {
            Children =
            {
                new TextBlock { Text = title, FontSize = 11.5, Foreground = Brushes2.Get(today ? "AccentBrush" : "MutedBrush"), FontWeight = today || total ? FontWeights.SemiBold : FontWeights.Normal },
                new TextBlock { Text = minutes > 0 ? WorkMath.HoursText(minutes) : "—", FontSize = 17, FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 4, 0, 0) },
                new TextBlock { Text = capacity > 0 ? $"/ {WorkMath.HoursText(capacity)}" : "不上班", FontSize = 11, Foreground = Brushes2.Get("MutedBrush") },
                bar,
                new TextBlock { Text = tasks, FontSize = 11, Foreground = Brushes2.Get("MutedBrush"), TextWrapping = TextWrapping.Wrap },
            },
        };
        return new Border
        {
            Style = (Style)FindResource("Card"),
            Padding = new Thickness(10, 8, 10, 8),
            Margin = new Thickness(0, 0, 6, 0),
            Child = panel,
            BorderBrush = today ? Brushes2.Get("AccentBrush") : null,
        };
    }

    FrameworkElement TaskRow(string key, string title, string right)
    {
        var grid = new Grid { Margin = new Thickness(0, 3, 0, 3) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var text = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis };
        var link = new Hyperlink(new Run(key)) { FontWeight = FontWeights.SemiBold };
        link.Click += (_, _) => _host.ShowPage("tasks", key);
        text.Inlines.Add(link);
        text.Inlines.Add(new Run(" " + title));
        grid.Children.Add(text);
        var r = new TextBlock { Text = right, Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(12, 0, 0, 0) };
        Grid.SetColumn(r, 1);
        grid.Children.Add(r);
        return grid;
    }
}
