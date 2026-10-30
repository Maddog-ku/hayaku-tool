using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Documents;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 待我審核：等我審核的變更申請依專案分組，可以直接核准、核准並關注、拒絕；
/// 「我送出的」列出自己還在等審核的申請，可以撤回。
/// </summary>
public sealed class ReviewsPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;
    readonly StackPanel _list = new();
    readonly TextBlock _summary = new() { Margin = new Thickness(0, 0, 0, 12) };
    readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    readonly RadioButton _toReview;
    readonly RadioButton _mine;
    List<ChangeRequest> _pending = [];

    public ReviewsPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        _summary.SetResourceReference(TextBlock.ForegroundProperty, "MutedBrush");
        _toReview = new RadioButton { Style = (Style)FindResource("Segment"), GroupName = "ReviewTab", Content = "待我審核", IsChecked = true };
        _mine = new RadioButton { Style = (Style)FindResource("Segment"), GroupName = "ReviewTab", Content = "我送出的" };
        _toReview.Checked += (_, _) => Render();
        _mine.Checked += (_, _) => Render();

        var reload = new Button { Content = "⟳ 重新整理", Style = (Style)FindResource("FlatButton"), Padding = new Thickness(12, 5, 12, 5),
            HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center };
        reload.Click += async (_, _) => await LoadAsync();
        var title = new Grid { Margin = new Thickness(0, 0, 0, 10) };
        title.Children.Add(new TextBlock { Text = "待我審核", Style = (Style)FindResource("PageTitle"), Margin = new Thickness(0), VerticalAlignment = VerticalAlignment.Center });
        title.Children.Add(reload);
        var tabs = new Border { Style = (Style)FindResource("SegmentTrack"), HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(0, 0, 0, 10),
            Child = new StackPanel { Orientation = Orientation.Horizontal, Children = { _toReview, _mine } } };

        var dock = new DockPanel { Margin = new Thickness(24, 18, 24, 16) };
        foreach (var top in new UIElement[] { title, tabs, _summary, _status })
        {
            DockPanel.SetDock(top, Dock.Top);
            dock.Children.Add(top);
        }
        dock.Children.Add(new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = _list });
        Content = dock;
    }

    public Task ActivateAsync(object? arg)
    {
        if (arg is "mine")
            _mine.IsChecked = true;
        return LoadAsync();
    }

    async Task LoadAsync()
    {
        Inputs.Show(_status, null);
        _summary.Text = "讀取中…";
        try
        {
            _pending = await _engine.Api.ListChangeRequestsAsync("PENDING", CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error("讀取變更申請失敗", ex);
            _summary.Text = "";
            Inputs.Show(_status, $"讀取失敗：{ex.Message}");
            return;
        }
        Render();
    }

    string? Me => _engine.User?.Username;

    void Render()
    {
        _list.Children.Clear();
        var mine = _mine.IsChecked == true;
        var items = _pending.Where(c => (c.Requester?.Username == Me) == mine).OrderByDescending(c => c.CreatedAt).ToList();
        var toReview = _pending.Count(c => c.Requester?.Username != Me);
        var mineCount = _pending.Count - toReview;
        _toReview.Content = toReview > 0 ? $"待我審核 {toReview}" : "待我審核";
        _mine.Content = mineCount > 0 ? $"我送出的 {mineCount}" : "我送出的";
        _summary.Text = mine
            ? items.Count == 0 ? "沒有在等審核的申請。" : $"{items.Count} 筆申請還在等審核，撤回後就不會再送給管理者。"
            : items.Count == 0
                ? _engine.CanReview ? "沒有要審核的申請 🎉" : "你在各專案都不是管理員或編輯者，沒有要審核的申請。"
                : $"{items.Count} 筆申請等你審核（{items.Select(c => c.Project).Distinct().Count()} 個專案）";

        foreach (var group in items.GroupBy(c => c.Project))
        {
            var project = _engine.ProjectOf(group.Key);
            var header = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 6, 0, 8) };
            header.Children.Add(ProjectIcons.Icon(project, group.Key, 20));
            header.Children.Add(new TextBlock { Text = project?.Name ?? group.Key, FontWeight = FontWeights.SemiBold, FontSize = 14,
                VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 0, 0) });
            header.Children.Add(new TextBlock { Text = $"  {group.Count()}", VerticalAlignment = VerticalAlignment.Center, Foreground = Brushes2.Get("MutedBrush") });
            _list.Children.Add(header);
            foreach (var c in group)
                _list.Children.Add(Card(c, mine));
        }
    }

    FrameworkElement Card(ChangeRequest c, bool mine)
    {
        var body = new StackPanel();
        var first = new TextBlock { TextTrimming = TextTrimming.CharacterEllipsis };
        first.Inlines.Add(new Run(c.TypeText) { Foreground = Brushes2.Get(c.Type == "DELETE" ? "ErrorBrush" : "AccentBrush"), FontWeight = FontWeights.SemiBold });
        first.Inlines.Add(new Run("　"));
        if (c.TaskKey is { } key)
        {
            var link = new Hyperlink(new Run(key)) { FontWeight = FontWeights.SemiBold };
            link.Click += (_, _) => _host.ShowPage("tasks", key);
            first.Inlines.Add(link);
            first.Inlines.Add(new Run(" "));
        }
        first.Inlines.Add(new Run(c.TaskTitle));
        body.Children.Add(first);

        var meta = $"{(mine ? "送出" : c.Requester?.DisplayName ?? "")} · {Inputs.Ago(c.CreatedAt)}";
        if (c.LoggedMinutes > 0)
            meta += $" · 已記錄 {WorkMath.HoursMinutes(c.LoggedMinutes)}";
        if (c.DescendantCount > 0)
            meta += $" · 含 {c.DescendantCount} 個子任務";
        body.Children.Add(new TextBlock { Text = meta, FontSize = 11.5, Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 2, 0, 0) });
        foreach (var change in c.Changes)
            body.Children.Add(new TextBlock { Text = "• " + change, Margin = new Thickness(0, 3, 0, 0), TextWrapping = TextWrapping.Wrap });
        if (c.Reason is { Length: > 0 } reason)
            body.Children.Add(new TextBlock { Text = $"原因：{reason}", Margin = new Thickness(0, 4, 0, 0), TextWrapping = TextWrapping.Wrap,
                Foreground = Brushes2.Get("MutedBrush") });

        var status = new TextBlock { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 6, 0, 0), Visibility = Visibility.Collapsed };
        var actions = new WrapPanel { Margin = new Thickness(0, 10, 0, 0) };
        if (mine)
        {
            var withdraw = Button("撤回", "FlatButton");
            withdraw.Click += async (_, _) =>
            {
                if (await Inputs.Run(withdraw, status, () => _engine.Api.WithdrawChangeRequestAsync(c.Id, CancellationToken.None)))
                    await AfterActionAsync();
            };
            actions.Children.Add(withdraw);
        }
        else
        {
            var note = new TextBox { Width = 220, Margin = new Thickness(0, 0, 8, 6), Style = (Style)FindResource("HintBox"), Tag = "給申請人的說明（選填）" };
            var approve = Button("✓ 核准", "PrimaryButton");
            approve.Click += async (_, _) => await ReviewAsync(c, true, false, note.Text, approve, status);
            actions.Children.Add(approve);
            if (c.Type != "DELETE")
            {
                var watch = Button("✓ 核准並關注", "FlatButton");
                watch.ToolTip = "核准後把你加成這個任務的關注者";
                watch.Click += async (_, _) => await ReviewAsync(c, true, true, note.Text, watch, status);
                actions.Children.Add(watch);
            }
            var reject = Button("✕ 拒絕", "FlatButton");
            reject.Click += async (_, _) => await ReviewAsync(c, false, false, note.Text, reject, status);
            actions.Children.Add(reject);
            actions.Children.Add(note);
        }
        body.Children.Add(actions);
        body.Children.Add(status);
        return new Border { Style = (Style)FindResource("Card"), Padding = new Thickness(14, 12, 14, 12), Margin = new Thickness(0, 0, 0, 8), Child = body };
    }

    Button Button(string text, string style) =>
        new() { Content = text, Style = (Style)FindResource(style), Padding = new Thickness(12, 4, 12, 4), Margin = new Thickness(0, 0, 6, 6) };

    async Task ReviewAsync(ChangeRequest c, bool approve, bool watch, string note, Button button, TextBlock status)
    {
        if (!approve && c.Type == "CREATE")
        {
            var logged = c.LoggedMinutes > 0 ? $"（已經記錄 {WorkMath.HoursMinutes(c.LoggedMinutes)} 工時）" : "";
            var ok = MessageBox.Show(Window.GetWindow(this)!, $"拒絕會刪除 {c.TaskKey} {c.TaskTitle}{logged}，確定？", "拒絕新增申請",
                MessageBoxButton.OKCancel, MessageBoxImage.Question);
            if (ok != MessageBoxResult.OK)
                return;
        }
        if (await Inputs.Run(button, status, () => _engine.Api.ReviewChangeRequestAsync(c.Id, approve, watch, note, CancellationToken.None,
                !approve && c.Type == "CREATE" ? "delete" : null)))
            await AfterActionAsync();
    }

    async Task AfterActionAsync()
    {
        await _engine.RefreshReviewsAsync();
        await LoadAsync();
        _ = _engine.AfterExternalChangeAsync();
    }
}
