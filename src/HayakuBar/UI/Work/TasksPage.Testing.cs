using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 任務詳細的測試流程與鍵盤：
/// 測試結果顯示分類、嚴重度、第幾輪、回報的人，依狀態給〔開始修〕〔送復驗〕〔通過〕〔退回〕〔不處理〕；
/// 測試任務顯示底下測試結果的進度與各狀態數量，子任務區變成「測試結果」（可以搜尋、依狀態篩選），〔加入測試結果〕（R）。
/// ↑↓ 在同一層的任務之間移動（已完成的也跳得到）。
/// </summary>
public partial class TasksPage
{
    /// <summary>上層任務 → 它的子任務（打開任務時順手記下，同一層移動不用多打請求）。</summary>
    readonly Dictionary<string, IReadOnlyList<(string Key, string Title, string Status)>> _childrenOf = new(StringComparer.OrdinalIgnoreCase);

    string _resultFilter = "";
    string? _resultFilterFor;
    string _resultBucket = "";

    static readonly (string Bucket, string Label)[] Buckets =
    [
        ("", "全部"), ("fixing", "修復中"), ("retest", "待復驗"), ("testing", "測試中"), ("pass", "通過"), ("closed", "不處理"),
    ];

    void RenderTesting(TaskDetail d)
    {
        _childrenOf[d.Key] = d.Children;
        if (_resultFilterFor != d.Key)
        {
            _resultFilterFor = d.Key;
            _resultFilter = _resultBucket = "";
            ResultFilterBox.Text = "";
            if (ResultBucketBox.ItemsSource is not null)
                ResultBucketBox.SelectedIndex = 0;
        }
        _loadingDetail = true;
        TypeBox.ItemsSource ??= new[] { TaskKind.Normal, TaskKind.Test, TaskKind.TestResult }.Select(TaskKinds.Label).ToList();
        TypeBox.SelectedItem = TaskKinds.Label(d.Kind);
        _loadingDetail = false;
        TestPanel.Children.Clear();
        TestPanel.Visibility = d.Kind == TaskKind.Normal ? Visibility.Collapsed : Visibility.Visible;
        ChildrenHeader.Text = d.Kind == TaskKind.Test ? $"測試結果（{d.Children.Count}）" : "子任務";
        if (d.Kind == TaskKind.TestResult)
        {
            RenderResultPanel(d);
            if (_engine.TryGetCachedProject(KeyFinder.ProjectOf(d.Key)) is null)
                _ = LoadSubStatusesAsync(d);
        }
        else if (d.Kind == TaskKind.Test)
            RenderTestTaskPanel(d);
        RenderChildren(d);
    }

    // ───────────── 測試結果 ─────────────

    void RenderResultPanel(TaskDetail d)
    {
        var info = d.Test;
        var parts = new List<string> { "🐞 測試結果" };
        if (info.Category is { } category)
            parts.Add(category);
        if (info.Severity is { } severity)
            parts.Add($"嚴重度 {severity}");
        if (info.Round > 0)
            parts.Add(info.Hot ? $"第 {info.Round} 輪 🔥" : $"第 {info.Round} 輪");
        var reporter = info.Reporter ?? d.CreatedBy;
        if (reporter is { Length: > 0 })
            parts.Add($"回報：{reporter}");
        var line = new TextBlock { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 6) };
        line.Inlines.Add(new System.Windows.Documents.Run(string.Join("・", parts))
        {
            Foreground = Brushes2.Get(info.Hot ? "ErrorBrush" : TaskKinds.SeverityBrush(info.Severity)),
            FontWeight = info.Hot ? FontWeights.SemiBold : FontWeights.Normal,
        });
        if (d.Path.Count > 0)
        {
            line.Inlines.Add(new System.Windows.Documents.Run("　測試任務 ") { Foreground = Brushes2.Get("MutedBrush") });
            line.Inlines.Add(TaskLink(d.Path[^1].Key));
        }
        TestPanel.Children.Add(line);

        var buttons = new WrapPanel();
        Button Add(string text, string tip, Func<Task> click, bool primary = false)
        {
            var b = new Button
            {
                Content = text, ToolTip = tip, Style = (Style)FindResource(primary ? "PrimaryButton" : "FlatButton"),
                Padding = new Thickness(12, 5, 12, 5), Margin = new Thickness(0, 0, 6, 6),
            };
            b.Click += async (_, _) => await click();
            buttons.Children.Add(b);
            return b;
        }
        var bucket = TaskKinds.Bucket(d.Status, d.StatusCategory, info.Stage);
        var stage = TaskKinds.StageOf(d.Status, info.Stage);
        var canRetest = _engine.CanRetest(d.Key, reporter);
        // 子狀態（例 待更新測試機、待上正式機）：更新好了推到下一步，不是開始修或通過
        var batch = _engine.BatchStep(d.Key, d.Status);
        if (batch is not null)
            Add($"⇢ {batch.BatchText}", $"{batch.Name}：更新好了就推到下一步，可以填更新說明（例版本號）", () => OpenAdvanceDialog(d, batch), primary: true);
        else if (d.Status == "待修復" || stage == "待修復")
            Add("🔧 開始修", "狀態改成修復中，可以順手填預估工時", () => OpenTestDialog("fix", d), primary: true);
        else if (stage == TimerEngine.StatusFixing)
        {
            if (_engine.FixDoneStep(d.Key, d.Status) is { } next)
                Add("✓ 修好了，待更新", $"改成{next.Name}" + (next.BatchText is { } b ? $"，{b}後再送復驗" : ""),
                    () => RunTestAsync(() => _engine.SubmitRetestAsync(d.Key, d.Status), d.Key), primary: true);
            else
                Add("✓ 送復驗", "修好了，請回報的人驗", () => RunTestAsync(() => _engine.SubmitRetestAsync(d.Key, d.Status), d.Key), primary: true);
        }
        else if (bucket == "fixing" && d.Status != TimerEngine.StatusFixing)
            Add("🔧 開始修", "狀態改成修復中，可以順手填預估工時", () => OpenTestDialog("fix", d), primary: true);
        if (canRetest && bucket is "retest" or "testing")
        {
            if (batch is null)
                Add("✓ 通過", _engine.PassStep(d.Key, d.Status) is { } next ? $"復驗通過，改成{next.Name}（還不算完成）" : "復驗通過，結案（已完成）",
                    () => RunTestAsync(() => _engine.RetestAsync(d.Key, pass: true, null), d.Key), primary: true);
            Add("↩ 退回", "還有問題，退回給工程師再修（輪數 +1）", () => OpenTestDialog("reopen", d));
        }
        if (canRetest && bucket is not ("pass" or "closed"))
            Add("不處理", "不修了，結案", async () =>
            {
                if (MessageBox.Show(Window.GetWindow(this)!, $"{d.Key} 標為不處理（結案，不修了）？", "不處理", MessageBoxButton.OKCancel,
                        MessageBoxImage.Question) == MessageBoxResult.OK)
                    await RunTestAsync(() => _engine.WontFixAsync(d.Key), d.Key);
            });
        if (buttons.Children.Count > 0)
            TestPanel.Children.Add(buttons);
        else if (!canRetest && bucket == "retest")
            TestPanel.Children.Add(Muted("等回報的人（或專案管理員）復驗。"));
    }

    Task OpenAdvanceDialog(TaskDetail d, TestSubStatus step)
    {
        var window = new AdvanceTestsWindow(_engine, KeyFinder.ProjectOf(d.Key), step, [d.Key]) { Owner = Window.GetWindow(this) };
        window.Done += async message =>
        {
            Inputs.Show(DetailStatus, message, error: false);
            await RefreshDetailQuietlyAsync(d.Key);
        };
        window.Show();
        return Task.CompletedTask;
    }

    /// <summary>測試結果的按鈕要看專案的子狀態：專案還沒讀過就讀一次（每個專案只讀一次），讀到後重畫這一塊。</summary>
    async Task LoadSubStatusesAsync(TaskDetail d)
    {
        if (!await _engine.LoadProjectsOfAsync([d.Key]) || _detail?.Key != d.Key || _engine.CachedSubStatuses(d.Key).Count == 0)
            return;
        TestPanel.Children.Clear();
        RenderResultPanel(d);
    }

    Task OpenTestDialog(string action, TaskDetail d)
    {
        var window = new TestActionWindow(_engine, new TestActionRequest(action, d.Key, d.Title)) { Owner = Window.GetWindow(this) };
        window.Done += async message =>
        {
            Inputs.Show(DetailStatus, message, error: false);
            await RefreshDetailQuietlyAsync(d.Key);
        };
        window.Show();
        return Task.CompletedTask;
    }

    async Task RunTestAsync(Func<Task<string>> action, string key)
    {
        string? message = null;
        if (await Inputs.Run(null, DetailStatus, async () => message = await action()))
        {
            Inputs.Show(DetailStatus, message, error: false);
            await RefreshDetailQuietlyAsync(key);
        }
    }

    // ───────────── 測試任務 ─────────────

    void RenderTestTaskPanel(TaskDetail d)
    {
        var counts = d.Children.GroupBy(c => TaskKinds.Bucket(c.Status)).ToDictionary(g => g.Key, g => g.Count());
        int N(string bucket) => counts.GetValueOrDefault(bucket);
        var total = d.Children.Count;
        var finished = N("pass") + N("closed");
        TestPanel.Children.Add(new TextBlock
        {
            Text = total == 0 ? "🧪 測試任務・還沒有測試結果" : $"🧪 測試任務・{finished}/{total} 已結案",
            Margin = new Thickness(0, 0, 0, 4),
        });
        if (total > 0)
        {
            TestPanel.Children.Add(new ProgressBar
            {
                Height = 6, Maximum = total, Value = finished, Margin = new Thickness(0, 0, 0, 4),
                Foreground = Brushes2.Get("RunBrush"), Background = Brushes2.Get("SubtleBrush"), BorderThickness = new Thickness(0),
            });
            TestPanel.Children.Add(Muted(string.Join("　", Buckets.Skip(1)
                .Where(b => N(b.Bucket) > 0).Select(b => $"{b.Label} {N(b.Bucket)}"))));
        }
        var add = new Button
        {
            Content = "＋ 加入測試結果", ToolTip = "R：記一個問題（分類、嚴重度、要誰修、關聯任務）",
            Style = (Style)FindResource("PrimaryButton"), Padding = new Thickness(12, 5, 12, 5), Margin = new Thickness(0, 4, 0, 6),
            HorizontalAlignment = HorizontalAlignment.Left,
        };
        add.Click += (_, _) => OpenAddTestResult(d);
        TestPanel.Children.Add(add);
    }

    /// <summary>加入測試結果：關聯任務先帶入測試任務的上層，同一層的其他任務可以快速選。</summary>
    void OpenAddTestResult(TaskDetail d)
    {
        var parent = d.Path.Count > 0 ? d.Path[^1].Key : null;
        var siblings = parent is not null && _childrenOf.TryGetValue(parent, out var list)
            ? list.Select(c => (c.Key, c.Title)).ToList()
            : null;
        var window = new AddTestResultWindow(_engine, d.Key, d.Title, parent, siblings) { Owner = Window.GetWindow(this) };
        window.Added += async key =>
        {
            Inputs.Show(DetailStatus, key.Length > 0 ? $"已加入測試結果 {key}" : "已加入測試結果", error: false);
            await RefreshDetailQuietlyAsync(d.Key);
        };
        window.Show();
    }

    /// <summary>子任務清單；測試任務的是測試結果，可以用名稱或編號搜尋、依狀態篩選，照狀態分組排。</summary>
    void RenderChildren(TaskDetail d)
    {
        IEnumerable<(string Key, string Title, string Status)> children = d.Children;
        var isTest = d.Kind == TaskKind.Test;
        if (isTest)
        {
            var q = _resultFilter.Trim();
            children = children
                .Where(c => q.Length == 0 || c.Key.Contains(q, StringComparison.OrdinalIgnoreCase) || c.Title.Contains(q, StringComparison.OrdinalIgnoreCase))
                .Where(c => _resultBucket.Length == 0 || TaskKinds.Bucket(c.Status) == _resultBucket)
                .OrderBy(c => Array.FindIndex(Buckets, b => b.Bucket == TaskKinds.Bucket(c.Status)));
        }
        ChildrenList.ItemsSource = children.Select(c => new { c.Key, Title = isTest ? $"🐞 {c.Title}" : c.Title, Status = $"· {c.Status}" }).ToList();
        ChildrenSection.Visibility = d.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        ResultFilterBar.Visibility = isTest && d.Children.Count > 3 ? Visibility.Visible : Visibility.Collapsed;
        if (isTest && ResultBucketBox.ItemsSource is null)
        {
            ResultBucketBox.ItemsSource = Buckets.Select(b => b.Label).ToList();
            ResultBucketBox.SelectedIndex = 0;
        }
    }

    void ResultFilter_TextChanged(object sender, TextChangedEventArgs e)
    {
        _resultFilter = ResultFilterBox.Text;
        if (_detail is not null)
            RenderChildren(_detail);
    }

    void ResultBucket_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        _resultBucket = ResultBucketBox.SelectedIndex > 0 ? Buckets[ResultBucketBox.SelectedIndex].Bucket : "";
        if (_detail is not null)
            RenderChildren(_detail);
    }

    // ───────────── 類型 ─────────────

    async void TypeBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_detail is null || _loadingDetail || TypeBox.SelectedItem is not string label || label == TaskKinds.Label(_detail.Kind))
            return;
        await UpdateAsync(new() { ["type"] = label }, $"類型（{label}）");
    }

    // ───────────── 鍵盤 ─────────────

    /// <summary>詳細開著、焦點不在輸入框時：↑↓ 同一層的上一個／下一個任務，R 在測試任務加入測試結果。</summary>
    void HandleDetailKeys(KeyEventArgs e)
    {
        if (Overlay.Visibility != Visibility.Visible || DetailPanel.Visibility != Visibility.Visible || _detail is null
            || Keyboard.Modifiers != ModifierKeys.None)
            return;
        if (Keyboard.FocusedElement is TextBoxBase or ComboBox or ComboBoxItem or DatePicker || Keyboard.FocusedElement is DependencyObject focused
            && FindParent<ComboBox>(focused) is not null)
            return;
        switch (e.Key)
        {
            case Key.Up or Key.Down:
                e.Handled = true;
                _ = MoveSiblingAsync(e.Key == Key.Down ? 1 : -1);
                break;
            case Key.R when _detail.Kind == TaskKind.Test:
                e.Handled = true;
                OpenAddTestResult(_detail);
                break;
        }
    }

    static T? FindParent<T>(DependencyObject d) where T : DependencyObject
    {
        for (var p = d; p is not null; p = System.Windows.Media.VisualTreeHelper.GetParent(p))
            if (p is T found)
                return found;
        return null;
    }

    bool _movingSibling;

    /// <summary>
    /// 同一層的上一個／下一個任務：有上層的照上層的子任務順序（含已完成的；上層沒讀過就讀一次）；
    /// 最上層的照我的任務裡同專案、也在最上層的。
    /// </summary>
    async Task MoveSiblingAsync(int step)
    {
        if (_detail is null || _movingSibling)
            return;
        _movingSibling = true;
        try
        {
            var key = _detail.Key;
            List<string> keys;
            var parent = _detail.Path.Count > 0 ? _detail.Path[^1].Key : null;
            if (parent is not null)
            {
                if (!_childrenOf.TryGetValue(parent, out var children))
                {
                    Inputs.Show(DetailStatus, "讀取同一層的任務…", error: false);
                    var parentDetail = await _engine.Api.GetTaskAsync(parent, CancellationToken.None);
                    _childrenOf[parent] = children = parentDetail.Children;
                }
                keys = children.Select(c => c.Key).ToList();
            }
            else
            {
                var project = KeyFinder.ProjectOf(key);
                keys = _engine.TaskList(TimerEngine.AllTab).Concat(_doneTasks)
                    .Where(t => (t.Ancestors?.Count ?? 0) == 0 && string.Equals(KeyFinder.ProjectOf(t.Key), project, StringComparison.OrdinalIgnoreCase))
                    .Select(t => t.Key).Distinct(StringComparer.OrdinalIgnoreCase)
                    .OrderBy(k => int.TryParse(k[(k.LastIndexOf('-') + 1)..], out var n) ? n : int.MaxValue).ToList();
            }
            var index = keys.FindIndex(k => string.Equals(k, key, StringComparison.OrdinalIgnoreCase));
            var next = index + step;
            if (index < 0 || next < 0 || next >= keys.Count)
            {
                Inputs.Show(DetailStatus, step > 0 ? "已經是這一層的最後一個" : "已經是這一層的第一個", error: false);
                return;
            }
            if (_detail?.Key == key)
                await OpenTaskAsync(keys[next]);
        }
        catch (Exception ex)
        {
            Log.Error("切換同一層的任務失敗", ex);
            Inputs.Show(DetailStatus, $"讀不到同一層的任務：{ex.Message}");
        }
        finally
        {
            _movingSibling = false;
        }
    }
}
