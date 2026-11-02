using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;
using HayakuBar.UI.Work;

namespace HayakuBar.UI;

/// <summary>
/// 在測試任務底下加入測試結果（QA 記一個問題）：名稱、內容（重現步驟）、分類、嚴重度、要誰修、關聯任務、期限，
/// 一開始是「要修」（待修復）或「之後再測」（未開始）。關聯任務先帶入測試任務的上層，也能快速選同一層的其他任務。
/// 截圖、附件 MCP 不能上傳，建好後到網頁貼。
/// </summary>
public sealed class AddTestResultWindow : SmallDialog
{
    readonly TimerEngine _engine;
    readonly string _testTask;
    readonly TextBox _title = new() { Margin = new Thickness(0, 0, 0, 10) };
    readonly TextBox _content = MultiLine(90);
    readonly ComboBox _category = new() { ItemsSource = TaskKinds.Categories, SelectedIndex = 0, Width = 120, Margin = new Thickness(0, 0, 12, 0) };
    readonly ComboBox _severity = new() { ItemsSource = TaskKinds.Severities, SelectedIndex = 2, Width = 100 };
    readonly ComboBox _assignee = new() { IsEditable = true, Margin = new Thickness(0, 0, 0, 10) };
    readonly TextBox _related = new() { Margin = new Thickness(0, 0, 0, 4) };
    readonly ComboBox _siblings = new() { Margin = new Thickness(0, 0, 0, 10), Visibility = Visibility.Collapsed };
    readonly TextBox _due;
    readonly RadioButton _toFix = new() { Content = "要修（待修復）", IsChecked = true, Margin = new Thickness(0, 0, 16, 0), GroupName = "start" };
    readonly RadioButton _toTest = new() { Content = "先建好，之後再測（未開始）", GroupName = "start" };
    List<PersonRef> _members = [];

    /// <summary>加好了：新測試結果的編號（回應裡找不到是空字串）。</summary>
    public event Action<string>? Added;

    /// <summary>parent：測試任務的上層（預設的關聯任務）；siblings：同一層的其他任務（快速加為關聯）。</summary>
    public AddTestResultWindow(TimerEngine engine, string testTask, string testTitle, string? parent,
        IReadOnlyList<(string Key, string Title)>? siblings = null)
        : base("加入測試結果", 500)
    {
        _engine = engine;
        _testTask = testTask;
        _due = new TextBox { Style = (Style)FindResource("HintBox"), Tag = "期限，例 11/5、明天、+3（可以不填）", Margin = new Thickness(0, 0, 0, 10) };
        Body.Children.Add(new TextBlock
        {
            Text = $"🧪 {testTask} {testTitle}".TrimEnd(), FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
        });
        Body.Children.Add(Label("名稱"));
        Body.Children.Add(_title);
        Body.Children.Add(Label("內容（重現步驟、預期與實際結果；可用 Markdown）"));
        Body.Children.Add(_content);
        Body.Children.Add(new StackPanel
        {
            Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 10),
            Children =
            {
                new StackPanel { Children = { Label("分類"), _category } },
                new StackPanel { Children = { Label("嚴重度"), _severity } },
            },
        });
        Body.Children.Add(Label("要誰修（不選是未指派）"));
        Body.Children.Add(_assignee);
        Body.Children.Add(Label("關聯任務（編號，用逗號分開；可以跨專案）"));
        _related.Text = parent ?? "";
        Body.Children.Add(_related);
        if (siblings is { Count: > 0 })
        {
            _siblings.ItemsSource = new[] { "＋ 加入同一層的任務…" }.Concat(siblings.Where(s => s.Key != testTask).Select(s => $"{s.Key} {s.Title}")).ToList();
            _siblings.SelectedIndex = 0;
            _siblings.Visibility = Visibility.Visible;
            _siblings.SelectionChanged += (_, _) =>
            {
                if (_siblings.SelectedIndex <= 0 || KeyFinder.Parse(_siblings.SelectedItem as string) is not { } key)
                    return;
                var keys = RelatedKeys();
                if (!keys.Contains(key, StringComparer.OrdinalIgnoreCase))
                    _related.Text = string.Join(", ", keys.Append(key));
                _siblings.SelectedIndex = 0;
            };
        }
        Body.Children.Add(_siblings);
        Body.Children.Add(Label("期限（要修時給工程師看的預計結束）"));
        Body.Children.Add(_due);
        Body.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 10), Children = { _toFix, _toTest } });
        Body.Children.Add(new TextBlock
        {
            Text = "截圖、附件 MCP 不能上傳，建好後到網頁貼（內容可以文字和截圖穿插）。",
            Foreground = Brushes2.Get("HintBrush"), FontSize = 12, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 12),
        });

        AddButton("加入", primary: true, () => AddAsync(again: false));
        AddButton("加入，再記一筆", primary: false, () => AddAsync(again: true));
        AddButton("取消", primary: false, cancel: true);
        Loaded += async (_, _) =>
        {
            _title.Focus();
            await LoadMembersAsync();
        };
    }

    /// <summary>要誰修的選單：專案成員（專案資料有快取，通常不用多打請求）。</summary>
    async Task LoadMembersAsync()
    {
        try
        {
            _members = (await _engine.GetProjectAsync(KeyFinder.ProjectOf(_testTask))).Members.ToList();
            _assignee.ItemsSource = _members.Select(m => m.DisplayName).ToList();
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {_testTask} 的專案成員失敗", ex);
        }
    }

    List<string> RelatedKeys() =>
        _related.Text.Split([',', '，', '、', ' '], StringSplitOptions.RemoveEmptyEntries)
            .Select(KeyFinder.Parse).OfType<string>().Distinct(StringComparer.OrdinalIgnoreCase).ToList();

    async Task AddAsync(bool again)
    {
        var title = _title.Text.Trim();
        if (title.Length == 0)
        {
            ShowError("請填名稱");
            _title.Focus();
            return;
        }
        DateOnly? due = null;
        if (_due.Text.Trim().Length > 0 && (due = Inputs.ParseDate(_due.Text)) is null)
        {
            ShowError("期限看不懂，例 2026-11-05、11/5、明天、+3");
            return;
        }
        var who = _assignee.Text.Trim();
        var assignee = _members.FirstOrDefault(m => m.DisplayName == who)?.Username ?? (who.Length > 0 ? who : null);
        foreach (var b in Buttons.Children.OfType<Button>())
            b.IsEnabled = false;
        try
        {
            var key = await _engine.Api.AddTestResultAsync(_testTask, title, _content.Text.Trim(), _category.SelectedItem as string,
                _severity.SelectedItem as string, assignee is null ? null : [assignee], RelatedKeys(), _toFix.IsChecked == true, due,
                CancellationToken.None);
            Log.Info($"在 {_testTask} 加入測試結果 {key}：{title}");
            Added?.Invoke(key ?? "");
            _ = _engine.AfterExternalChangeAsync();
            if (!again)
            {
                Close();
                return;
            }
            _title.Text = _content.Text = "";
            StatusText.Text = $"已加入 {key}，繼續記下一筆";
            StatusText.Foreground = Brushes2.Get("RunBrush");
            StatusText.Visibility = Visibility.Visible;
            _title.Focus();
        }
        catch (Exception ex)
        {
            Log.Error($"在 {_testTask} 加入測試結果失敗", ex);
            ShowError(ex.Message);
        }
        foreach (var b in Buttons.Children.OfType<Button>())
            b.IsEnabled = true;
    }
}
