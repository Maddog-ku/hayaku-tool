using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// 問題回報的全文（get_feedback，要「問題回報」權限）：說明、處理說明、環境、處理紀錄、建立的任務，截圖點了直接看。
/// 處理（轉任務、改狀態）MCP 做不到，留「到網頁處理」。
/// </summary>
public sealed class FeedbackWindow : Window
{
    readonly TimerEngine _engine;
    readonly int _number;
    readonly StackPanel _body = new();

    public FeedbackWindow(TimerEngine engine, int number)
    {
        _engine = engine;
        _number = number;
        Title = $"問題回報 #{number}";
        Width = 640;
        Height = 620;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        FontFamily = (FontFamily)FindResource("UiFont");
        FontSize = 13;
        Background = Brushes2.Get("BgBrush");
        Foreground = Brushes2.Get("TextBrush");
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);
        Content = new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = new Border { Padding = new Thickness(20, 16, 20, 16), Child = _body } };
        _body.Children.Add(Text("讀取中…", muted: true));
        Loaded += async (_, _) => await LoadAsync();
    }

    static TextBlock Text(string text, bool muted = false, bool bold = false, double size = 13) => new()
    {
        Text = text, TextWrapping = TextWrapping.Wrap, FontSize = size, FontWeight = bold ? FontWeights.SemiBold : FontWeights.Normal,
        Foreground = Brushes2.Get(muted ? "MutedBrush" : "TextBrush"), Margin = new Thickness(0, 0, 0, 6),
    };

    TextBlock Group(string title) => new()
    {
        Text = title, Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(0, 12, 0, 4),
    };

    async Task LoadAsync()
    {
        FeedbackDetail f;
        try
        {
            f = await _engine.Api.GetFeedbackAsync(_number, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取問題回報 #{_number} 失敗", ex);
            _body.Children.Clear();
            _body.Children.Add(new TextBlock { Text = $"讀不到：{ex.Message}（要有「問題回報」權限）", Foreground = Brushes2.Get("ErrorBrush"), TextWrapping = TextWrapping.Wrap });
            AddWebButton();
            return;
        }
        _body.Children.Clear();
        _body.Children.Add(Text($"#{f.Number} {f.Title}", bold: true, size: 16));
        _body.Children.Add(Text(string.Join("・", new[]
        {
            f.Status, f.Reporter, f.CreatedAt is { } at ? at.ToLocalTime().ToString("yyyy-MM-dd HH:mm") : null,
        }.Where(x => !string.IsNullOrEmpty(x))), muted: true));
        _body.Children.Add(Box(f.Body.Length > 0 ? f.Body : "（沒有說明）"));
        if (!string.IsNullOrWhiteSpace(f.Resolution))
        {
            _body.Children.Add(Group("處理說明"));
            _body.Children.Add(Box(f.Resolution));
        }
        if (f.Environment.Count > 0)
        {
            _body.Children.Add(Group("環境"));
            foreach (var (label, value) in f.Environment)
                _body.Children.Add(Text($"{label}：{value}", muted: true));
        }
        if (f.ScreenshotIds.Count > 0)
        {
            _body.Children.Add(Group($"截圖（{f.ScreenshotIds.Count}）"));
            var panel = new WrapPanel();
            var n = 1;
            foreach (var id in f.ScreenshotIds)
            {
                var label = $"🖼 截圖 {n++}";
                var b = new Button { Content = label, Style = (Style)FindResource("FlatButton"), Padding = new Thickness(10, 4, 10, 4), Margin = new Thickness(0, 0, 6, 6) };
                b.Click += (_, _) => FilePreviewWindow.Show(_engine, id, $"#{f.Number} {label[2..]}", this);
                panel.Children.Add(b);
            }
            _body.Children.Add(panel);
        }
        if (f.CreatedTasks.Count > 0)
        {
            _body.Children.Add(Group("建立的任務"));
            var text = new TextBlock { TextWrapping = TextWrapping.Wrap };
            foreach (var key in f.CreatedTasks)
            {
                var link = new Hyperlink(new Run(key));
                link.Click += (_, _) => _engine.OpenTaskInWorkspace(key);
                text.Inlines.Add(link);
                text.Inlines.Add(new Run("　"));
            }
            _body.Children.Add(text);
        }
        if (f.History.Count > 0)
        {
            _body.Children.Add(Group("處理紀錄"));
            foreach (var h in f.History)
                _body.Children.Add(Text(h, muted: true));
        }
        AddWebButton();
    }

    Border Box(string text) => new()
    {
        Background = Brushes2.Get("SubtleBrush"), CornerRadius = new CornerRadius(6), Padding = new Thickness(10, 8, 10, 8), Margin = new Thickness(0, 4, 0, 4),
        Child = new TextBox { Style = (Style)FindResource("PlainText"), Text = text },
    };

    void AddWebButton()
    {
        var web = new Button
        {
            Content = "到網頁處理（轉任務、改狀態，從通知打開）↗", Style = (Style)FindResource("LinkButton"), HorizontalAlignment = HorizontalAlignment.Left,
            Margin = new Thickness(0, 12, 0, 0),
        };
        web.Click += (_, _) => Web.Open(_engine.WebUrl("/notifications"));
        _body.Children.Add(web);
    }
}
