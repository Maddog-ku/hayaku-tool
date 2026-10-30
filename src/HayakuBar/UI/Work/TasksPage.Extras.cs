using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 任務詳細的相依關係、附件、文件、異動紀錄與刪除。四塊都平常收起、展開才讀（同一個任務只讀一次），
/// 不會因為打開任務就多打請求。
/// </summary>
public partial class TasksPage
{
    string? _depsFor;
    string? _attachFor;
    string? _docsFor;
    string? _historyFor;
    string? _historyCursor;

    /// <summary>換了任務：四塊收起、清空，下次展開再讀；同一個任務重畫（例如改了欄位）就保留。</summary>
    void ResetExtras(TaskDetail d)
    {
        DocsHeader.Text = d.ContentBlockCount > 1 ? $"文件（{d.ContentBlockCount}）" : "文件";
        DocsExpander.Visibility = d.ContentBlockCount > 0 ? Visibility.Visible : Visibility.Collapsed;
        if (_depsFor == d.Key || _attachFor == d.Key || _docsFor == d.Key || _historyFor == d.Key)
            return;
        _depsFor = _attachFor = _docsFor = _historyFor = _historyCursor = null;
        foreach (var expander in new[] { DepsExpander, AttachExpander, DocsExpander, HistoryExpander })
            expander.IsExpanded = false;
        foreach (var panel in new[] { DepsPanel, AttachPanel, DocsPanel, HistoryPanel })
            panel.Children.Clear();
    }

    static TextBlock Muted(string text) =>
        new() { Text = text, Foreground = Brushes2.Get("MutedBrush"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 2, 0, 4) };

    static TextBlock Failed(string what, Exception ex) =>
        new() { Text = $"{what}失敗：{ex.Message}", Foreground = Brushes2.Get("ErrorBrush"), TextWrapping = TextWrapping.Wrap };

    /// <summary>任務編號做成連結：點了在這裡打開那個任務。</summary>
    Hyperlink TaskLink(string key)
    {
        var link = new Hyperlink(new Run(key)) { FontWeight = FontWeights.SemiBold };
        link.Click += async (_, _) => await OpenTaskAsync(key);
        return link;
    }

    // ───────────── 相依關係 ─────────────

    async void Deps_Expanded(object sender, RoutedEventArgs e)
    {
        if (_detail is null || _depsFor == _detail.Key)
            return;
        await LoadDepsAsync(_detail.Key);
    }

    async Task LoadDepsAsync(string key)
    {
        _depsFor = key;
        DepsPanel.Children.Clear();
        DepsPanel.Children.Add(Muted("讀取中…"));
        DependencyInfo deps;
        try
        {
            deps = await _engine.Api.ListDependenciesAsync(key, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {key} 的相依關係失敗", ex);
            DepsPanel.Children.Clear();
            DepsPanel.Children.Add(Failed("讀取相依關係", ex));
            _depsFor = null;
            return;
        }
        if (_detail?.Key != key)
            return;
        DepsPanel.Children.Clear();
        AddDepGroup("這個任務在等", deps.Predecessors, key);
        AddDepGroup("在等這個任務", deps.Successors, key);
        if (deps.Predecessors.Count == 0 && deps.Successors.Count == 0)
            DepsPanel.Children.Add(Muted("沒有相依關係"));
        DepsPanel.Children.Add(NewDependencyRow(key));
    }

    void AddDepGroup(string title, IReadOnlyList<DependencyItem> items, string key)
    {
        if (items.Count == 0)
            return;
        DepsPanel.Children.Add(new TextBlock { Text = title, Foreground = Brushes2.Get("MutedBrush"), FontSize = 12, Margin = new Thickness(0, 4, 0, 2) });
        foreach (var d in items)
        {
            var text = new TextBlock { TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center };
            text.Inlines.Add(TaskLink(d.OtherKey));
            text.Inlines.Add(new Run($" {d.OtherTitle}"));
            if (d.OtherStatus is { Length: > 0 } status)
                text.Inlines.Add(new Run($"  · {status}") { Foreground = Brushes2.Get("MutedBrush"), FontSize = 12 });
            if (d.Rule.Length > 0)
                text.Inlines.Add(new Run($"  · {d.Rule}") { Foreground = Brushes2.Get("MutedBrush"), FontSize = 12 });
            if (d.Blocking)
                text.Inlines.Add(new Run("  · 還擋著") { Foreground = Brushes2.Get("ErrorBrush"), FontSize = 12, FontWeight = FontWeights.SemiBold });
            var remove = new Button { Content = "移除", Style = (Style)FindResource("FlatButton"), Padding = new Thickness(8, 2, 8, 2), Margin = new Thickness(8, 0, 0, 0) };
            remove.IsEnabled = d.Id.Length > 0;
            remove.Click += async (_, _) =>
            {
                string? message = null;
                if (await Inputs.Run(remove, DetailStatus, async () => message = await _engine.Api.RemoveDependencyAsync(d.Id, CancellationToken.None)))
                {
                    Inputs.Show(DetailStatus, message ?? $"已移除與 {d.OtherKey} 的相依", error: false);
                    await LoadDepsAsync(key);
                }
            };
            var row = new DockPanel { Margin = new Thickness(0, 2, 0, 2) };
            DockPanel.SetDock(remove, Dock.Right);
            row.Children.Add(remove);
            row.Children.Add(text);
            DepsPanel.Children.Add(row);
        }
    }

    /// <summary>新增相依：任務編號、方向（這個任務等它／它等這個任務）、規則。</summary>
    FrameworkElement NewDependencyRow(string key)
    {
        var other = new TextBox { Width = 110, Style = (Style)FindResource("HintBox"), Tag = "任務編號", Margin = new Thickness(0, 0, 6, 0) };
        var direction = new ComboBox { Width = 130, Margin = new Thickness(0, 0, 6, 0), ItemsSource = new[] { "這個任務等它", "它等這個任務" }, SelectedIndex = 0 };
        var template = new ComboBox
        {
            Width = 170, Margin = new Thickness(0, 0, 6, 0), SelectedIndex = 0,
            ItemsSource = HayakuApi.DependencyTemplates.Select(t => t.Label).ToList(),
        };
        var add = new Button { Content = "＋ 新增相依", Style = (Style)FindResource("FlatButton"), Padding = new Thickness(10, 3, 10, 3) };
        add.Click += async (_, _) =>
        {
            if (KeyFinder.Parse(other.Text) is not { } target)
            {
                Inputs.Show(DetailStatus, "請填另一個任務的編號（例 F2P-12）");
                return;
            }
            var code = HayakuApi.DependencyTemplates[Math.Max(0, template.SelectedIndex)].Code;
            string? message = null;
            if (await Inputs.Run(add, DetailStatus, async () =>
                    message = await _engine.Api.AddDependencyAsync(key, target, direction.SelectedIndex == 0, code, CancellationToken.None)))
            {
                Inputs.Show(DetailStatus, message ?? $"已設定與 {target} 的相依", error: false);
                await LoadDepsAsync(key);
            }
        };
        return new WrapPanel { Margin = new Thickness(0, 8, 0, 4), Children = { other, direction, template, add } };
    }

    // ───────────── 附件 ─────────────

    async void Attach_Expanded(object sender, RoutedEventArgs e)
    {
        if (_detail is null || _attachFor == _detail.Key)
            return;
        var key = _attachFor = _detail.Key;
        AttachPanel.Children.Clear();
        AttachPanel.Children.Add(Muted("讀取中…"));
        List<AttachmentItem> items;
        string? webUrl;
        try
        {
            (items, webUrl) = await _engine.Api.ListAttachmentsAsync(key, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {key} 的附件失敗", ex);
            AttachPanel.Children.Clear();
            AttachPanel.Children.Add(Failed("讀取附件", ex));
            _attachFor = null;
            return;
        }
        if (_detail?.Key != key)
            return;
        AttachPanel.Children.Clear();
        if (items.Count == 0)
            AttachPanel.Children.Add(Muted("沒有附件"));
        foreach (var a in items)
        {
            var text = new TextBlock { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 2, 0, 2) };
            var link = new Hyperlink(new Run($"📎 {a.Name}"));
            var url = a.Url ?? webUrl;
            link.Click += (_, _) =>
            {
                if (url is not null)
                    Web.Open(url);
            };
            text.Inlines.Add(link);
            var meta = new[] { a.Size is { } s ? FileSize(s) : null, a.Uploader, a.At is { } at ? Inputs.Ago(at) : null }.Where(x => x is not null);
            text.Inlines.Add(new Run($"  · {string.Join(" · ", meta)}") { Foreground = Brushes2.Get("MutedBrush"), FontSize = 12 });
            AttachPanel.Children.Add(text);
        }
        if (webUrl is not null)
        {
            var open = new Button { Content = "上傳、下載到網頁 ↗", Style = (Style)FindResource("LinkButton"), HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(0, 4, 0, 0) };
            open.Click += (_, _) => Web.Open(webUrl);
            AttachPanel.Children.Add(open);
        }
    }

    static string FileSize(long bytes) => bytes switch
    {
        < 1024 => $"{bytes} B",
        < 1024 * 1024 => $"{bytes / 1024.0:0.#} KB",
        _ => $"{bytes / 1024.0 / 1024:0.#} MB",
    };

    // ───────────── 文件 ─────────────

    async void Docs_Expanded(object sender, RoutedEventArgs e)
    {
        if (_detail is null || _docsFor == _detail.Key)
            return;
        var key = _docsFor = _detail.Key;
        DocsPanel.Children.Clear();
        DocsPanel.Children.Add(Muted("讀取中…"));
        List<ContentDoc> docs;
        try
        {
            docs = await _engine.Api.GetTaskContentAsync(key, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {key} 的文件失敗", ex);
            DocsPanel.Children.Clear();
            DocsPanel.Children.Add(Failed("讀取文件", ex));
            _docsFor = null;
            return;
        }
        if (_detail?.Key != key)
            return;
        DocsPanel.Children.Clear();
        if (docs.Count == 0)
            DocsPanel.Children.Add(Muted("沒有文件"));
        foreach (var doc in docs)
        {
            DocsPanel.Children.Add(new TextBlock { Text = doc.Title, FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 4, 0, 4) });
            DocsPanel.Children.Add(new Border
            {
                Background = Brushes2.Get("SubtleBrush"), CornerRadius = new CornerRadius(6), Padding = new Thickness(10, 8, 10, 8), Margin = new Thickness(0, 0, 0, 8),
                Child = new TextBox
                {
                    Style = (Style)FindResource("PlainText"), Text = doc.Text.Length > 0 ? doc.Text : "（空白）",
                    MaxHeight = 320, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                },
            });
        }
        DocsPanel.Children.Add(Muted("唯讀；要編輯請在網頁開啟。"));
    }

    // ───────────── 異動紀錄 ─────────────

    async void History_Expanded(object sender, RoutedEventArgs e)
    {
        if (_detail is null || _historyFor == _detail.Key)
            return;
        _historyFor = _detail.Key;
        _historyCursor = null;
        HistoryPanel.Children.Clear();
        await LoadHistoryAsync(_detail.Key);
    }

    /// <summary>讀一頁（50 筆）接在後面；還有更多時最後放「載入更多」。</summary>
    async Task LoadHistoryAsync(string key)
    {
        var loading = Muted("讀取中…");
        HistoryPanel.Children.Add(loading);
        List<TaskActivityItem> items;
        string? cursor;
        try
        {
            (items, cursor) = await _engine.Api.TaskActivityAsync(key, _historyCursor, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {key} 的異動紀錄失敗", ex);
            HistoryPanel.Children.Remove(loading);
            HistoryPanel.Children.Add(Failed("讀取異動紀錄", ex));
            _historyFor = null;
            return;
        }
        if (_detail?.Key != key)
            return;
        HistoryPanel.Children.Remove(loading);
        if (items.Count == 0 && _historyCursor is null)
            HistoryPanel.Children.Add(Muted("沒有異動紀錄"));
        foreach (var item in items)
        {
            var text = new TextBlock { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 2, 0, 2) };
            var when = item.At == DateTimeOffset.MinValue ? "" : item.At.ToLocalTime().ToString("MM/dd HH:mm", CultureInfo.InvariantCulture);
            text.Inlines.Add(new Run($"{when}  {item.Actor}  ") { Foreground = Brushes2.Get("MutedBrush"), FontSize = 12 });
            text.Inlines.Add(new Run(item.Text));
            HistoryPanel.Children.Add(text);
        }
        _historyCursor = cursor;
        if (cursor is not null)
        {
            var more = new Button { Content = "載入更多", Style = (Style)FindResource("LinkButton"), HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(0, 4, 0, 0) };
            more.Click += async (_, _) =>
            {
                HistoryPanel.Children.Remove(more);
                await LoadHistoryAsync(key);
            };
            HistoryPanel.Children.Add(more);
        }
    }

    // ───────────── 刪除 ─────────────

    async void Delete_Click(object sender, RoutedEventArgs e)
    {
        if (_detail is null)
            return;
        var d = _detail;
        var children = d.Children.Count > 0 ? $"，連同 {d.Children.Count} 個子任務" : "";
        var ok = MessageBox.Show(Window.GetWindow(this)!,
            $"刪除 {d.Key} {d.Title}{children}？\n可以從網頁的垃圾桶還原；如果你是執行者，會送出刪除申請給管理者審核。",
            "刪除任務", MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok != MessageBoxResult.OK)
            return;
        string? message = null;
        if (!await Inputs.Run(DeleteButton, DetailStatus, async () => message = await _engine.Api.DeleteTaskAsync(d.Key, null, CancellationToken.None)))
            return;
        Log.Info($"刪除任務 {d.Key}：{message ?? "完成"}");
        _engine.ShowNotice(message ?? $"已刪除 {d.Key}（可以從網頁的垃圾桶還原）");
        CloseDialog();
        await _engine.AfterExternalChangeAsync();
        RenderList(keepSelection: false);
    }
}
