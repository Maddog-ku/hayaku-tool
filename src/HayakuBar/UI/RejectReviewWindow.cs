using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// 從審核提示按「駁回」開的小視窗：可以填給申請人的說明；駁回新增任務的申請會刪掉那個任務，先講清楚再送出。
/// </summary>
public sealed class RejectReviewWindow : Window
{
    readonly TimerEngine _engine;
    readonly ChangeRequest _request;
    readonly TextBox _note = new()
    {
        AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 64, MaxHeight = 160,
        VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Margin = new Thickness(0, 0, 0, 12),
    };
    readonly Button _reject;

    readonly bool _demo;

    /// <summary>demo：截圖、檢查外觀用，按「駁回」不會真的送出。</summary>
    public RejectReviewWindow(TimerEngine engine, ChangeRequest request, bool demo = false)
    {
        _engine = engine;
        _request = request;
        _demo = demo;
        Title = "駁回申請";
        Width = 440;
        SizeToContent = SizeToContent.Height;
        ResizeMode = ResizeMode.NoResize;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        Topmost = true;
        ShowInTaskbar = false;
        FontFamily = (System.Windows.Media.FontFamily)FindResource("UiFont");
        FontSize = 13;
        Background = Brushes2.Get("BgBrush");
        Foreground = Brushes2.Get("TextBrush");
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);

        var summary = new StackPanel { Margin = new Thickness(0, 0, 0, 12) };
        summary.Children.Add(new TextBlock
        {
            Text = $"{request.TypeText}・{request.TaskKey} {request.TaskTitle}".TrimEnd(),
            FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap,
        });
        var details = new List<string> { $"申請人：{request.Requester?.DisplayName ?? "（不明）"}" };
        if (request.Changes.Count > 0)
            details.Add(string.Join("、", request.Changes));
        if (!string.IsNullOrWhiteSpace(request.Reason))
            details.Add($"理由：{request.Reason}");
        summary.Children.Add(new TextBlock
        {
            Text = string.Join("\n", details), TextWrapping = TextWrapping.Wrap,
            Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 4, 0, 0),
        });
        if (request.Type == "CREATE")
        {
            var logged = request.LoggedMinutes > 0 ? $"，連同已經記錄的 {WorkMath.HoursMinutes(request.LoggedMinutes)} 工時" : "";
            summary.Children.Add(new TextBlock
            {
                Text = $"駁回會刪除這個任務{logged}。", TextWrapping = TextWrapping.Wrap,
                Foreground = Brushes2.Get("ErrorBrush"), FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 8, 0, 0),
            });
        }

        _reject = new Button
        {
            Content = request.Type == "CREATE" ? "駁回並刪除任務" : "駁回",
            Style = (Style)FindResource("PrimaryButton"), Padding = new Thickness(16, 5, 16, 5),
        };
        _reject.Click += async (_, _) => await RejectAsync();
        var cancel = new Button
        {
            Content = "取消", Style = (Style)FindResource("FlatButton"), Padding = new Thickness(16, 5, 16, 5),
            Margin = new Thickness(8, 0, 0, 0), IsCancel = true,
        };

        Content = new StackPanel
        {
            Margin = new Thickness(20, 16, 20, 16),
            Children =
            {
                summary,
                new TextBlock { Text = "給申請人的說明（選填）", Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 0, 0, 4) },
                _note,
                new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Children = { _reject, cancel } },
            },
        };
        Loaded += (_, _) => _note.Focus();
    }

    /// <summary>送出駁回；結果（成功或失敗）由引擎用提示告知，這裡送出後就關掉。</summary>
    async Task RejectAsync()
    {
        _reject.IsEnabled = false;
        _note.IsEnabled = false;
        if (!_demo)
            await _engine.ReviewAsync(_request, approve: false, note: _note.Text.Trim());
        Close();
    }
}
