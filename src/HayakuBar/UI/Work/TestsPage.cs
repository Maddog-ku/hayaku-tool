using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 測試：網頁的三種測試視圖。
/// 需修復（工程師看自己要修的：開始修、送復驗，直接改期限與預估工時）、
/// 需測試（QA 看要驗的：通過、退回；要測的測試任務可以加入測試結果）、
/// 測試總覽（管理者看整體：數字、嚴重度分布、各測試任務進度、逾時清單）。
/// 打開這頁才讀；收到測試結果的新通知時（通知本來就在輪詢）才自動重讀，不另外輪詢。
/// </summary>
public sealed class TestsPage : UserControl, IWorkPage
{
    static readonly (string View, string Label)[] Views = [("fix", "需修復"), ("test", "需測試"), ("overview", "測試總覽")];
    static readonly (string Key, string Label)[] Groupings = [("", "不分組"), ("test", "依測試任務"), ("severity", "依嚴重度"), ("assignee", "依執行者"), ("project", "依專案")];
    static readonly (string Key, string Label)[] Periods = [("30d", "近 30 天"), ("week", "本週"), ("month", "本月"), ("all", "全部")];

    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;
    readonly Dictionary<string, RadioButton> _tabs = [];
    readonly ComboBox _project = new() { Width = 200, Margin = new Thickness(0, 0, 8, 0) };
    readonly ComboBox _group = new() { Width = 120, Margin = new Thickness(0, 0, 8, 0), ItemsSource = Groupings.Select(g => g.Label).ToList(), SelectedIndex = 1 };
    readonly ComboBox _period = new() { Width = 100, Margin = new Thickness(0, 0, 8, 0), ItemsSource = Periods.Select(p => p.Label).ToList(), SelectedIndex = 0 };
    readonly ComboBox _severity = new() { Width = 115, Margin = new Thickness(0, 0, 8, 0) };
    readonly TextBox _filter = new() { Width = 180, Margin = new Thickness(0, 0, 8, 0) };
    readonly CheckBox _mine = new() { Content = "只看跟我有關的", IsChecked = true, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 10, 0) };
    readonly CheckBox _hot = new() { Content = "第 3 輪起", VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 0, 10, 0), ToolTip = "只看退回 2 次以上的" };
    readonly TextBlock _summary = new() { Margin = new Thickness(0, 0, 0, 8) };
    readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    readonly StackPanel _list = new();
    string _view = "fix";
    List<TestResultRow> _rows = [];

    /// <summary>收起來的段落與分組（視圖|段落|分組）；只記到關掉程式為止。</summary>
    readonly HashSet<string> _collapsed = [];
    TestOverview? _overview;
    DateTime _loaded = DateTime.MinValue;
    string _loadedKey = "";
    DateTimeOffset _seenNotification = DateTimeOffset.Now;
    int _version;
    bool _building;

    public TestsPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        _filter.Style = (Style)FindResource("HintBox");
        _filter.Tag = "🔍 名稱或編號";
        _summary.SetResourceReference(TextBlock.ForegroundProperty, "MutedBrush");
        _severity.ItemsSource = new[] { "全部嚴重度" }.Concat(TaskKinds.Severities).ToList();
        _severity.SelectedIndex = 0;

        var tabs = new StackPanel { Orientation = Orientation.Horizontal };
        foreach (var (view, label) in Views)
        {
            var tab = new RadioButton { Style = (Style)FindResource("Segment"), GroupName = "TestView", Content = label, Tag = view };
            tab.Checked += async (_, _) =>
            {
                if (_building)
                    return;
                _view = view;
                await LoadAsync(force: false);
            };
            _tabs[view] = tab;
            tabs.Children.Add(tab);
        }
        _building = true;
        _tabs[_view].IsChecked = true;
        _building = false;

        _project.SelectionChanged += async (_, _) =>
        {
            if (!_building)
                await LoadAsync(force: false);
        };
        _period.SelectionChanged += async (_, _) =>
        {
            if (!_building)
                await LoadAsync(force: false);
        };
        _mine.Click += async (_, _) => await LoadAsync(force: false);
        _group.SelectionChanged += (_, _) => Render();
        _severity.SelectionChanged += (_, _) => Render();
        _filter.TextChanged += (_, _) => Render();
        _hot.Click += (_, _) => Render();

        var reload = Button("⟳ 重新整理", "FlatButton");
        reload.Click += async (_, _) => await LoadAsync(force: true);
        var title = new DockPanel { Margin = new Thickness(0, 0, 0, 10) };
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Children = { reload } };
        DockPanel.SetDock(actions, Dock.Right);
        title.Children.Add(actions);
        title.Children.Add(new StackPanel
        {
            Orientation = Orientation.Horizontal,
            Children =
            {
                new TextBlock { Text = "測試", Style = (Style)FindResource("PageTitle"), Margin = new Thickness(0, 0, 14, 0), VerticalAlignment = VerticalAlignment.Center },
                new Border { Style = (Style)FindResource("SegmentTrack"), Child = tabs, VerticalAlignment = VerticalAlignment.Center },
            },
        });
        var filters = new WrapPanel { Margin = new Thickness(0, 0, 0, 8), Children = { _project, _period, _filter, _severity, _group, _mine, _hot } };

        var dock = new DockPanel { Margin = new Thickness(24, 18, 24, 16) };
        foreach (var top in new UIElement[] { title, filters, _summary, _status })
        {
            DockPanel.SetDock(top, Dock.Top);
            dock.Children.Add(top);
        }
        dock.Children.Add(new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = _list });
        Content = dock;

        Loaded += (_, _) => _engine.Changed += OnEngineChanged;
        Unloaded += (_, _) => _engine.Changed -= OnEngineChanged;
    }

    Button Button(string text, string style) =>
        new() { Content = text, Style = (Style)FindResource(style), Padding = new Thickness(12, 5, 12, 5), Margin = new Thickness(6, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center };

    Button Small(string text, string style, string? tip = null) =>
        new() { Content = text, Style = (Style)FindResource(style), Padding = new Thickness(9, 2, 9, 2), Margin = new Thickness(6, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center, ToolTip = tip };

    string? SelectedProject => (_project.SelectedItem as ComboBoxItem)?.Tag as string;

    /// <summary>arg：fix、test、overview 切到那個視圖。</summary>
    public async Task ActivateAsync(object? arg)
    {
        RenderProjects();
        if (arg is string view && _tabs.TryGetValue(view, out var tab) && view != _view)
        {
            _building = true;
            tab.IsChecked = true;
            _view = view;
            _building = false;
        }
        await LoadAsync(force: false);
    }

    void RenderProjects()
    {
        var selected = SelectedProject;
        _building = true;
        var items = new List<ComboBoxItem> { new() { Content = "全部專案", Tag = null } };
        items.AddRange(_engine.Projects.Where(p => !p.IsArchived).Select(p => new ComboBoxItem { Content = ProjectIcons.Badge(p, p.Name, 16), Tag = p.Key }));
        _project.ItemsSource = items;
        _project.SelectedItem = items.FirstOrDefault(i => i.Tag as string == selected) ?? items[0];
        _building = false;
    }

    /// <summary>有測試結果的新通知（被退回、待復驗…）時重讀，看到的是別人剛改的；不另外輪詢。</summary>
    void OnEngineChanged()
    {
        if (!IsVisible)
            return;
        var latest = _engine.Notifications.Where(n => n.IsTestResult).Select(n => n.CreatedAt).DefaultIfEmpty(DateTimeOffset.MinValue).Max();
        if (latest <= _seenNotification)
            return;
        _seenNotification = latest;
        _ = LoadAsync(force: true);
    }

    async Task LoadAsync(bool force)
    {
        var mine = _mine.IsChecked == true;
        var period = Periods[Math.Max(0, _period.SelectedIndex)].Key;
        var key = $"{_view}|{SelectedProject}|{mine}|{period}";
        _period.Visibility = _view == "overview" ? Visibility.Visible : Visibility.Collapsed;
        _mine.Visibility = _group.Visibility = _severity.Visibility = _filter.Visibility = _hot.Visibility =
            _view == "overview" ? Visibility.Collapsed : Visibility.Visible;
        if (!force && key == _loadedKey && DateTime.Now - _loaded < TimeSpan.FromMinutes(1))
        {
            Render();
            return;
        }
        var version = ++_version;
        Inputs.Show(_status, null);
        _summary.Text = "讀取中…";
        try
        {
            if (_view == "overview")
            {
                var overview = await _engine.Api.TestOverviewAsync(SelectedProject, period, CancellationToken.None);
                if (version != _version)
                    return;
                _overview = overview;
            }
            else
            {
                var rows = await _engine.Api.ListTestResultsAsync(_view, SelectedProject, mine, CancellationToken.None);
                if (version != _version)
                    return;
                _rows = rows;
            }
            _loaded = DateTime.Now;
            _loadedKey = key;
            Render();
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取測試視圖 {_view} 失敗", ex);
            _summary.Text = "";
            Inputs.Show(_status, $"讀取失敗：{ex.Message}");
        }
    }

    // ───────────── 畫面 ─────────────

    void Render()
    {
        _list.Children.Clear();
        if (_view == "overview")
        {
            RenderOverview();
            return;
        }
        var q = _filter.Text.Trim();
        var severity = _severity.SelectedIndex > 0 ? TaskKinds.Severities[_severity.SelectedIndex - 1] : null;
        var rows = _rows
            .Where(r => q.Length == 0 || r.Key.Contains(q, StringComparison.OrdinalIgnoreCase) || r.Title.Contains(q, StringComparison.OrdinalIgnoreCase))
            .Where(r => severity is null || r.Severity == severity || r.Kind == TaskKind.Test)
            .Where(r => _hot.IsChecked != true || r.Round >= 3)
            .ToList();
        if (_view == "fix")
        {
            AddGroups(rows, "", "fix");
            _summary.Text = rows.Count == 0 ? (_mine.IsChecked == true ? "沒有你要修的測試結果 🎉" : "沒有要修的測試結果")
                : $"要修 {rows.Count} 個" + (rows.Count(r => r.Round >= 3) is var hot and > 0 ? $"（第 3 輪起 {hot} 個 🔥）" : "");
            return;
        }
        var retest = rows.Where(r => r.Group == "retest").ToList();
        var toTest = rows.Where(r => r.Group == "toTest").ToList();
        var tests = rows.Where(r => r.Group == "tests").ToList();
        AddSection("retest", $"待復驗（{retest.Count}）", retest, "修好了，等你驗：通過或退回");
        AddSection("toTest", $"要測的測試結果（{toTest.Count}）", toTest, "還沒測或測試中的");
        AddSection("tests", $"要測的測試任務（{tests.Count}）", tests, "未開始、測試中的測試任務：測到問題就加入測試結果");
        _summary.Text = rows.Count == 0 ? "沒有要測的 🎉" : $"待復驗 {retest.Count}・要測 {toTest.Count}・測試任務 {tests.Count}";
    }

    void AddSection(string id, string title, List<TestResultRow> rows, string hint)
    {
        if (rows.Count == 0)
            return;
        var key = $"{_view}|{id}";
        var open = !_collapsed.Contains(key);
        var header = new TextBlock { Text = $"{(open ? "▾" : "▸")} {title}", Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(0, 10, 0, 2) };
        _list.Children.Add(Toggle(header, key));
        if (!open)
            return;
        _list.Children.Add(new TextBlock { Text = hint, Foreground = Brushes2.Get("HintBrush"), FontSize = 12, Margin = new Thickness(0, 0, 0, 4) });
        AddGroups(rows, "    ", key);
    }

    /// <summary>段落、分組的標題：點一下收合／展開。</summary>
    FrameworkElement Toggle(TextBlock header, string key)
    {
        header.Cursor = System.Windows.Input.Cursors.Hand;
        header.ToolTip = "點一下收合／展開";
        header.MouseLeftButtonUp += (_, _) =>
        {
            if (!_collapsed.Remove(key))
                _collapsed.Add(key);
            Render();
        };
        return header;
    }

    /// <summary>照選的分組方式（依測試任務、嚴重度、執行者、專案）列出；parent 是收合用的段落代號。</summary>
    void AddGroups(List<TestResultRow> rows, string indent, string parent)
    {
        var grouping = Groupings[Math.Max(0, _group.SelectedIndex)].Key;
        if (grouping.Length == 0 || rows.Count < 2)
        {
            foreach (var r in Sorted(rows))
                _list.Children.Add(Row(r));
            return;
        }
        string GroupOf(TestResultRow r) => grouping switch
        {
            "test" => r.TestTask is { } t ? $"🧪 {t} {r.TestTaskTitle}".TrimEnd() : "（沒有測試任務）",
            "severity" => r.Severity ?? "（沒有嚴重度）",
            "assignee" => r.Assignees.Count > 0 ? string.Join("、", r.Assignees) : "未指派",
            _ => r.Project.Length > 0 ? r.Project : "（沒有專案）",
        };
        var groups = rows.GroupBy(GroupOf).ToList();
        if (grouping == "severity")
            groups = groups.OrderBy(g => SeverityRank(g.First().Severity)).ToList();
        foreach (var g in groups)
        {
            var key = $"{parent}|{grouping}|{g.Key}";
            var open = !_collapsed.Contains(key);
            _list.Children.Add(Toggle(new TextBlock
            {
                Text = $"{indent}{(open ? "▾" : "▸")} {g.Key}　{g.Count()}", FontWeight = FontWeights.SemiBold, Foreground = Brushes2.Get("MutedBrush"),
                Margin = new Thickness(0, 8, 0, 2), FontSize = 12,
            }, key));
            if (!open)
                continue;
            foreach (var r in Sorted(g))
                _list.Children.Add(Row(r));
        }
    }

    static int SeverityRank(string? s) => s is null ? 9 : TaskKinds.Severities.ToList().IndexOf(s) is var i and >= 0 ? i : 8;

    /// <summary>嚴重的、輪數多的、期限近的排前面。</summary>
    static IEnumerable<TestResultRow> Sorted(IEnumerable<TestResultRow> rows) =>
        rows.OrderBy(r => SeverityRank(r.Severity)).ThenByDescending(r => r.Round).ThenBy(r => r.PlannedEnd ?? DateOnly.MaxValue);

    FrameworkElement Row(TestResultRow r)
    {
        var grid = new Grid { Margin = new Thickness(0, 2, 0, 2) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        var name = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center };
        var key = KeyCopy.Run(r.Key);
        key.FontWeight = FontWeights.SemiBold;
        key.SetResourceReference(TextElement.ForegroundProperty, "AccentBrush");
        name.MouseLeftButtonUp += (_, e) => KeyCopy.TryCopy(e.OriginalSource, _engine, r.Key);
        name.Inlines.Add(key);
        name.Inlines.Add(new Run(" "));
        var title = new Hyperlink(new Run(TaskText.Title(r.Title, new TestInfo(r.Kind, r.Category, r.Severity, r.Round, r.Reporter))))
        {
            TextDecorations = null, Cursor = System.Windows.Input.Cursors.Hand,
        };
        title.SetResourceReference(TextElement.ForegroundProperty, "TextBrush");
        title.Click += (_, _) => _host.ShowPage("tasks", r.Key);
        name.Inlines.Add(title);
        var meta = new[]
        {
            r.Status, r.Severity, r.Category, r.Assignees.Count > 0 ? string.Join("、", r.Assignees) : r.Kind == TaskKind.TestResult ? "未指派" : null,
            r.Reporter is { } rep ? $"回報 {rep}" : null, r.Stale ? "逾時未處理" : null,
        }.Where(x => !string.IsNullOrEmpty(x));
        name.Inlines.Add(new Run($"　{string.Join(" · ", meta)}")
        {
            FontSize = 12, Foreground = Brushes2.Get(r.Round >= 3 || r.Stale ? "ErrorBrush" : TaskKinds.SeverityBrush(r.Severity) == "ErrorBrush" ? "WarnBrush" : "MutedBrush"),
        });
        name.ToolTip = $"{r.Key} {r.Title}" + (r.TestTask is { } tt ? $"\n測試任務 {tt} {r.TestTaskTitle}" : "") + "\n點編號複製、點名稱看詳細";
        grid.Children.Add(name);

        var actions = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Center };
        if (r.Kind == TaskKind.Test)
        {
            var add = Small("＋ 加入測試結果", "FlatButton", "R：記一個問題");
            add.Click += (_, _) => OpenAdd(r);
            actions.Children.Add(add);
        }
        else if (_view == "fix")
        {
            actions.Children.Add(DueBox(r));
            actions.Children.Add(EstimateBox(r));
            if (r.Status == TimerEngine.StatusFixing)
            {
                var submit = Small("✓ 送復驗", "PrimaryButton", "修好了，請回報的人驗");
                submit.Click += async (_, _) => await RunAsync(submit, () => _engine.SubmitRetestAsync(r.Key));
                actions.Children.Add(submit);
            }
            else
            {
                var fix = Small("🔧 開始修", "PrimaryButton", "狀態改成修復中，可以順手填預估工時");
                fix.Click += (_, _) => OpenDialog("fix", r);
                actions.Children.Add(fix);
            }
        }
        else
        {
            var pass = Small("✓ 通過", "PrimaryButton", "復驗通過，結案");
            pass.Click += async (_, _) => await RunAsync(pass, () => _engine.RetestAsync(r.Key, pass: true, null));
            var reopen = Small("↩ 退回", "FlatButton", "還有問題，退回給工程師（輪數 +1）");
            reopen.Click += (_, _) => OpenDialog("reopen", r);
            actions.Children.Add(pass);
            actions.Children.Add(reopen);
        }
        Grid.SetColumn(actions, 1);
        grid.Children.Add(actions);
        return new Border
        {
            Child = grid, Padding = new Thickness(8, 4, 8, 4), CornerRadius = new CornerRadius(6),
            Background = Brushes2.Get(r.Round >= 3 ? "SubtleBrush" : "BgBrush"),
        };
    }

    /// <summary>需修復視圖直接改期限：打完按 Enter 或離開就存。</summary>
    TextBox DueBox(TestResultRow r)
    {
        var box = new TextBox
        {
            Width = 86, Style = (Style)FindResource("HintBox"), Tag = "期限", Text = r.PlannedEnd is { } d ? $"{d:yyyy-MM-dd}" : "",
            Margin = new Thickness(6, 0, 0, 0), ToolTip = "期限（預計結束），例 11/5、明天；清空就是沒有期限",
        };
        var original = box.Text;
        async Task SaveAsync()
        {
            var text = box.Text.Trim();
            if (text == original)
                return;
            DateOnly? due = null;
            if (text.Length > 0 && (due = Inputs.ParseDate(text)) is null)
            {
                Inputs.Show(_status, "期限看不懂，例 2026-11-05、11/5、明天");
                box.Text = original;
                return;
            }
            original = due is { } nd ? $"{nd:yyyy-MM-dd}" : "";
            box.Text = original;
            await SaveFieldAsync(r.Key, "plannedEnd", due is { } v ? Inputs.DateText(v) : null, "期限");
        }
        box.LostKeyboardFocus += async (_, _) => await SaveAsync();
        box.KeyDown += (_, e) =>
        {
            if (e.Key == System.Windows.Input.Key.Enter)
                System.Windows.Input.Keyboard.ClearFocus();
        };
        return box;
    }

    /// <summary>需修復視圖直接填預估工時（小時）。</summary>
    TextBox EstimateBox(TestResultRow r)
    {
        var box = new TextBox
        {
            Width = 64, Style = (Style)FindResource("HintBox"), Tag = "預估 h", Margin = new Thickness(6, 0, 0, 0),
            ToolTip = "預估工時（小時），打完按 Enter",
        };
        box.KeyDown += async (_, e) =>
        {
            if (e.Key != System.Windows.Input.Key.Enter)
                return;
            var text = box.Text.Trim();
            if (text.Length == 0)
                return;
            if (!double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var h) || h < 0 || h > 1666)
            {
                Inputs.Show(_status, "預估工時請填小時數，例 2 或 0.5");
                return;
            }
            await SaveFieldAsync(r.Key, "estimatedHours", h, $"預估工時 {h} 小時");
            System.Windows.Input.Keyboard.ClearFocus();
        };
        return box;
    }

    async Task SaveFieldAsync(string key, string field, object? value, string what)
    {
        string? message = null;
        if (await Inputs.Run(null, _status, async () =>
                message = await _engine.Api.UpdateTaskAsync(key, new Dictionary<string, object?> { [field] = value }, CancellationToken.None)))
            Inputs.Show(_status, message is { Length: > 0 } ? message : $"{key} 已更新{what}", error: false);
    }

    async Task RunAsync(Button button, Func<Task<string>> action)
    {
        string? message = null;
        if (await Inputs.Run(button, _status, async () => message = await action()))
        {
            Inputs.Show(_status, message, error: false);
            await LoadAsync(force: true);
        }
    }

    void OpenDialog(string action, TestResultRow r)
    {
        var window = new TestActionWindow(_engine, new TestActionRequest(action, r.Key, r.Title)) { Owner = Window.GetWindow(this) };
        window.Done += async message =>
        {
            Inputs.Show(_status, message, error: false);
            await LoadAsync(force: true);
        };
        window.Show();
    }

    void OpenAdd(TestResultRow r)
    {
        var window = new AddTestResultWindow(_engine, r.Key, r.Title, null) { Owner = Window.GetWindow(this) };
        window.Added += async key =>
        {
            Inputs.Show(_status, key.Length > 0 ? $"已加入測試結果 {key}" : "已加入測試結果", error: false);
            await LoadAsync(force: true);
        };
        window.Show();
    }

    // ───────────── 測試總覽 ─────────────

    void RenderOverview()
    {
        if (_overview is not { } o)
            return;
        _summary.Text = $"{Periods[Math.Max(0, _period.SelectedIndex)].Label}・{(SelectedProject ?? "全部專案")}";
        var cards = new WrapPanel { Margin = new Thickness(0, 0, 0, 10) };
        void Card(string label, string value, string? sub = null, bool alert = false)
        {
            var panel = new StackPanel
            {
                Children =
                {
                    new TextBlock { Text = label, Foreground = Brushes2.Get("MutedBrush"), FontSize = 12 },
                    new TextBlock { Text = value, FontSize = 22, FontWeight = FontWeights.SemiBold, Foreground = Brushes2.Get(alert ? "ErrorBrush" : "TextBrush") },
                },
            };
            if (sub is not null)
                panel.Children.Add(new TextBlock { Text = sub, Foreground = Brushes2.Get("MutedBrush"), FontSize = 11 });
            cards.Children.Add(new Border { Style = (Style)FindResource("Card"), Padding = new Thickness(14, 8, 14, 8), Margin = new Thickness(0, 0, 8, 8), MinWidth = 120, Child = panel });
        }
        Card("未結案", o.Open.ToString(), $"致命 {o.Critical}・嚴重 {o.Major}", o.Critical > 0);
        Card("待復驗", o.Retest.ToString(), o.OldestRetestDays is { } days ? $"最久等了 {days} 天" : null);
        Card("新增", o.Created.ToString());
        Card("結案", (o.Done + o.WontFix).ToString(), $"完成 {o.Done}・不處理 {o.WontFix}");
        Card("平均輪數", o.AvgRound is { } avg ? avg.ToString("0.#", CultureInfo.InvariantCulture) : "—", $"第 3 輪起 {o.Round3Plus}", o.Round3Plus > 0);
        Card("逾時未處理", o.Stale.ToString(), null, o.Stale > 0);
        _list.Children.Add(cards);

        if (o.BySeverity.Count > 0)
            _list.Children.Add(new TextBlock
            {
                Text = "嚴重度　" + string.Join("　", o.BySeverity.Select(s => $"{s.Severity} {s.Count}")),
                Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 0, 0, 8),
            });

        _list.Children.Add(new TextBlock { Text = $"各測試任務（{o.Tests.Count}）", Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(0, 6, 0, 4) });
        if (o.Tests.Count == 0)
            _list.Children.Add(new TextBlock { Text = "這段期間沒有測試任務", Foreground = Brushes2.Get("MutedBrush") });
        foreach (var t in o.Tests)
        {
            var grid = new Grid { Margin = new Thickness(0, 2, 0, 2) };
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(160) });
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(230) });
            var name = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center };
            var link = new Hyperlink(new Run($"🧪 {t.Key}")) { FontWeight = FontWeights.SemiBold, TextDecorations = null };
            link.Click += (_, _) => _host.ShowPage("tasks", t.Key);
            name.Inlines.Add(link);
            name.Inlines.Add(new Run($" {t.Title}"));
            grid.Children.Add(name);
            var finished = t.Passed;
            var bar = new ProgressBar
            {
                Height = 6, Maximum = Math.Max(1, t.Total), Value = finished, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 8, 0),
                Foreground = Brushes2.Get("RunBrush"), Background = Brushes2.Get("SubtleBrush"), BorderThickness = new Thickness(0),
            };
            Grid.SetColumn(bar, 1);
            grid.Children.Add(bar);
            var counts = new TextBlock
            {
                Text = $"全部 {t.Total}・通過 {t.Passed}・修復中 {t.Fixing}・測試中 {t.Testing + t.Retest}", FontSize = 12,
                Foreground = Brushes2.Get("MutedBrush"), VerticalAlignment = VerticalAlignment.Center,
            };
            Grid.SetColumn(counts, 2);
            grid.Children.Add(counts);
            _list.Children.Add(grid);
        }

        if (o.StaleItems.Count > 0)
        {
            _list.Children.Add(new TextBlock { Text = $"逾時未處理（{o.StaleItems.Count}）", Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(0, 12, 0, 4) });
            foreach (var r in Sorted(o.StaleItems))
                _list.Children.Add(Row(r with { Stale = true }));
        }
    }
}
