using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// 套用子任務範本（Hayaku 1.5.0 範本中心）：選範本，〔預覽〕先看會建哪些、日期（dryRun，不寫入），〔套用〕才建立。
/// 範本清單在打開時才讀；任務右鍵選單（一個任務）與專案頁勾選多個任務開這個。
/// </summary>
public sealed class ApplyTemplateWindow : SmallDialog
{
    const int Max = 500;
    readonly TimerEngine _engine;
    readonly IReadOnlyList<string> _tasks;
    readonly ComboBox _template = new() { Margin = new Thickness(0, 0, 0, 12), IsEnabled = false };
    readonly CheckBox _skipExisting = new()
    {
        Content = "略過已經有子任務的任務", IsChecked = true, Margin = new Thickness(0, 0, 0, 8),
        ToolTip = "不勾：已經有子任務的也套用，第一層同名的略過",
    };
    readonly TextBlock _result = new() { TextWrapping = TextWrapping.Wrap };
    readonly ScrollViewer _resultBox;
    readonly Button _preview;
    readonly Button _apply;

    /// <summary>套用成功了（呼叫端據此重新整理）。</summary>
    public bool Applied { get; private set; }

    public ApplyTemplateWindow(TimerEngine engine, string project, IReadOnlyList<string> tasks) : base("套用子任務範本", 500)
    {
        _engine = engine;
        _tasks = tasks.Take(Max).ToList();
        _resultBox = new ScrollViewer
        {
            Content = _result, MaxHeight = 220, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Margin = new Thickness(0, 0, 0, 12), Visibility = Visibility.Collapsed,
        };
        Body.Children.Add(new TextBlock
        {
            Text = _tasks.Count == 1 ? $"套到 {_tasks[0]} 底下" : $"套到 {project} 的 {_tasks.Count} 個任務底下",
            FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 6),
        });
        if (_tasks.Count is > 1 and <= 12)
            Body.Children.Add(Muted(string.Join("、", _tasks), 6));
        if (tasks.Count > Max)
            Body.Children.Add(Muted($"一次最多 {Max} 個，只套前 {Max} 個", 6));
        Body.Children.Add(Muted("執行者一次只能套一個任務，建出來是待核准。範本在網頁的範本中心建立。", 12));
        Body.Children.Add(Label("範本"));
        Body.Children.Add(_template);
        Body.Children.Add(_skipExisting);
        Body.Children.Add(_resultBox);
        _template.SelectionChanged += (_, _) => _resultBox.Visibility = Visibility.Collapsed;
        _skipExisting.Click += (_, _) => _resultBox.Visibility = Visibility.Collapsed;
        _preview = AddButton("預覽", primary: false, PreviewAsync);
        _apply = AddButton("套用", primary: true, () => RunAsync(ApplyAsync));
        AddButton("取消", primary: false, cancel: true);
        _apply.IsEnabled = _preview.IsEnabled = false;
        Loaded += async (_, _) => await LoadAsync(project);
    }

    TextBlock Muted(string text, double bottom) =>
        new() { Text = text, Foreground = Brushes2.Get("MutedBrush"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, bottom) };

    SubtaskTemplate? Selected => (_template.SelectedItem as ComboBoxItem)?.Tag as SubtaskTemplate;

    async Task LoadAsync(string project)
    {
        _template.ItemsSource = new[] { new ComboBoxItem { Content = "讀取中…" } };
        _template.SelectedIndex = 0;
        List<SubtaskTemplate> templates;
        try
        {
            templates = await _engine.Api.ListSubtaskTemplatesAsync(project, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error("讀取子任務範本失敗", ex);
            _template.ItemsSource = null;
            ShowError($"讀取範本失敗：{ex.Message}");
            return;
        }
        if (templates.Count == 0)
        {
            _template.ItemsSource = new[] { new ComboBoxItem { Content = "沒有能套到這個專案的範本" } };
            _template.SelectedIndex = 0;
            return;
        }
        _template.ItemsSource = templates.Select(t => new ComboBoxItem { Content = t.Label, Tag = t, ToolTip = t.Description }).ToList();
        _template.SelectedIndex = 0;
        _template.IsEnabled = _apply.IsEnabled = _preview.IsEnabled = true;
    }

    async Task PreviewAsync()
    {
        if (Selected is not { } template)
            return;
        _preview.IsEnabled = _apply.IsEnabled = false;
        try
        {
            var result = await _engine.Api.ApplySubtaskTemplateAsync(_tasks, template.Id, _skipExisting.IsChecked == true, dryRun: true, CancellationToken.None);
            ShowResult("預覽（還沒建立）", result);
        }
        catch (Exception ex)
        {
            Log.Error("預覽套用範本失敗", ex);
            ShowError(ex.Message);
        }
        _preview.IsEnabled = _apply.IsEnabled = true;
    }

    async Task ApplyAsync()
    {
        if (Selected is not { } template)
            throw new InvalidOperationException("請先選範本");
        var result = await _engine.Api.ApplySubtaskTemplateAsync(_tasks, template.Id, _skipExisting.IsChecked == true, dryRun: false, CancellationToken.None);
        var text = Summary(result);
        Log.Info($"套用範本「{template.Name}」到 {string.Join("、", _tasks.Take(20))}{(_tasks.Count > 20 ? "…" : "")}：{text}");
        _engine.ShowNotice(text.Length > 120 ? text[..120] + "…" : text);
        Applied = true;
        await _engine.AfterExternalChangeAsync();
    }

    void ShowResult(string title, TemplateApplyResult result)
    {
        _result.Text = $"{title}\n{Summary(result)}";
        _result.Foreground = Brushes2.Get("TextBrush");
        _resultBox.Visibility = Visibility.Visible;
    }

    /// <summary>每個任務一行；認不出格式時顯示伺服器的說明或原文（截短）。</summary>
    static string Summary(TemplateApplyResult result)
    {
        var parts = new List<string>();
        if (result.Message is { Length: > 0 } message)
            parts.Add(message);
        parts.AddRange(result.Lines);
        if (parts.Count == 0)
            parts.Add(result.Raw.Length == 0 ? "完成" : result.Raw.Length > 800 ? result.Raw[..800] + "…" : result.Raw);
        return string.Join("\n", parts);
    }
}
