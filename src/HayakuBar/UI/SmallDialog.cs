using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace HayakuBar.UI;

/// <summary>
/// 獨立的小視窗（不開工作台也能用，例如從提示按的「開始修」「退回」）：固定在最上層、不佔工作列、跟著深淺色。
/// 內容用 Body 放，下面一排按鈕，訊息列顯示錯誤。
/// </summary>
public class SmallDialog : Window
{
    protected readonly StackPanel Body = new();
    protected readonly TextBlock StatusText = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    protected readonly StackPanel Buttons = new() { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right };

    public SmallDialog(string title, double width = 440)
    {
        Title = title;
        Width = width;
        SizeToContent = SizeToContent.Height;
        ResizeMode = ResizeMode.NoResize;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        Topmost = true;
        ShowInTaskbar = false;
        FontFamily = (FontFamily)FindResource("UiFont");
        FontSize = 13;
        Background = Brushes2.Get("BgBrush");
        Foreground = Brushes2.Get("TextBrush");
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);
        Content = new StackPanel { Margin = new Thickness(20, 16, 20, 16), Children = { Body, StatusText, Buttons } };
    }

    protected Button AddButton(string text, bool primary, Func<Task>? onClick = null, bool cancel = false)
    {
        var button = new Button
        {
            Content = text, Style = (Style)FindResource(primary ? "PrimaryButton" : "FlatButton"), Padding = new Thickness(16, 5, 16, 5),
            Margin = new Thickness(Buttons.Children.Count > 0 ? 8 : 0, 0, 0, 0), IsCancel = cancel, IsDefault = primary,
        };
        if (onClick is not null)
            button.Click += async (_, _) => await onClick();
        else if (cancel)
            button.Click += (_, _) => Close(); // 不是 ShowDialog 開的，IsCancel 不會自己關
        Buttons.Children.Add(button);
        return button;
    }

    protected TextBlock Label(string text) =>
        new() { Text = text, Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 0, 0, 4) };

    protected static TextBox MultiLine(double minHeight = 64) => new()
    {
        AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = minHeight, MaxHeight = 220,
        VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Margin = new Thickness(0, 0, 0, 12),
    };

    protected void ShowError(string message)
    {
        StatusText.Text = message;
        StatusText.Foreground = Brushes2.Get("ErrorBrush");
        StatusText.Visibility = Visibility.Visible;
    }

    /// <summary>按鈕全部停用、執行動作；失敗時顯示錯誤並恢復，成功時關掉視窗。</summary>
    protected async Task RunAsync(Func<Task> action)
    {
        foreach (var b in Buttons.Children.OfType<Button>())
            b.IsEnabled = false;
        try
        {
            await action();
            Close();
        }
        catch (Exception ex)
        {
            Infra.Log.Error($"{Title}失敗", ex);
            ShowError(ex.Message);
            foreach (var b in Buttons.Children.OfType<Button>())
                b.IsEnabled = true;
        }
    }
}
