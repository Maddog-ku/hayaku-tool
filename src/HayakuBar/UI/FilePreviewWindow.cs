using System.IO;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// 直接看檔案內容（read_file）：截圖、白板、設計稿這類圖片顯示圖，PDF、Word、Excel、CSV、文字檔顯示文字（很長的按「繼續讀」）。
/// 同一個檔案讀過就用快取；Hayaku 每次讀都留稽核紀錄，系統管理員可以關掉這個功能。
/// </summary>
public sealed class FilePreviewWindow : Window
{
    readonly TimerEngine _engine;
    readonly string _id;
    readonly string? _webUrl;
    readonly ContentControl _host = new();
    readonly TextBlock _status = new() { Foreground = Brushes2.Get("MutedBrush"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 6, 0, 0) };
    readonly StackPanel _actions = new() { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 8, 0, 0) };
    TextBox? _text;
    int? _next;
    FileContent? _content;

    /// <summary>webUrl：讀不到時的退路（到網頁下載）。</summary>
    public FilePreviewWindow(TimerEngine engine, string id, string name, string? webUrl = null)
    {
        _engine = engine;
        _id = id;
        _webUrl = webUrl;
        Title = name;
        Width = 860;
        Height = 640;
        WindowStartupLocation = WindowStartupLocation.CenterScreen;
        FontFamily = (FontFamily)FindResource("UiFont");
        FontSize = 13;
        Background = Brushes2.Get("BgBrush");
        Foreground = Brushes2.Get("TextBrush");
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);
        var root = new DockPanel { Margin = new Thickness(16, 12, 16, 12) };
        DockPanel.SetDock(_actions, Dock.Bottom);
        DockPanel.SetDock(_status, Dock.Bottom);
        root.Children.Add(_actions);
        root.Children.Add(_status);
        root.Children.Add(_host);
        Content = root;
        _host.Content = new TextBlock { Text = "讀取中…", Foreground = Brushes2.Get("MutedBrush"), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        Loaded += async (_, _) => await LoadAsync(0);
        KeyDown += (_, e) =>
        {
            if (e.Key == System.Windows.Input.Key.Escape)
                Close();
        };
    }

    async Task LoadAsync(int offset)
    {
        FileContent content;
        try
        {
            content = await _engine.Files.ReadAsync(_id, offset);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取檔案 {_id} 失敗", ex);
            if (offset == 0)
                _host.Content = new TextBlock { Text = $"讀不到內容：{ex.Message}", Foreground = Brushes2.Get("ErrorBrush"), TextWrapping = TextWrapping.Wrap };
            else
                _status.Text = $"讀不到後面的內容：{ex.Message}";
            RenderActions();
            return;
        }
        _content = content;
        if (content.Name is { Length: > 0 } name && offset == 0)
            Title = name;
        if (content.Image is { } bytes)
            ShowImage(bytes);
        else
            AppendText(content.Text ?? "", offset == 0);
        _next = content.NextOffset;
        _status.Text = string.Join("　", new[]
        {
            content.Note,
            _next is { } n ? $"已讀到第 {n:N0} 字，還有後面" : null,
        }.Where(x => !string.IsNullOrWhiteSpace(x)));
        RenderActions();
    }

    void ShowImage(byte[] bytes)
    {
        var image = new BitmapImage();
        image.BeginInit();
        image.CacheOption = BitmapCacheOption.OnLoad;
        image.StreamSource = new MemoryStream(bytes);
        image.EndInit();
        image.Freeze();
        _host.Content = new ScrollViewer
        {
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            Content = new Image { Source = image, Stretch = Stretch.Uniform, StretchDirection = StretchDirection.DownOnly },
        };
    }

    void AppendText(string text, bool first)
    {
        if (first || _text is null)
        {
            _text = new TextBox
            {
                Style = (Style)FindResource("PlainText"), Text = text.Length > 0 ? text : "（沒有文字；掃描的 PDF 沒有文字可讀）",
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
                FontFamily = (FontFamily)FindResource("UiFont"),
            };
            _host.Content = new Border { Background = Brushes2.Get("SubtleBrush"), CornerRadius = new CornerRadius(6), Padding = new Thickness(12, 10, 12, 10), Child = _text };
        }
        else
            _text.AppendText(text);
    }

    void RenderActions()
    {
        _actions.Children.Clear();
        Button Add(string text, Action click, bool primary = false)
        {
            var b = new Button
            {
                Content = text, Style = (Style)FindResource(primary ? "PrimaryButton" : "FlatButton"), Padding = new Thickness(14, 5, 14, 5),
                Margin = new Thickness(0, 0, 8, 0),
            };
            b.Click += (_, _) => click();
            _actions.Children.Add(b);
            return b;
        }
        if (_next is { } next)
            Add("繼續讀", () => _ = LoadAsync(next), primary: true);
        if (_text is not null)
            Add("複製文字", () =>
            {
                try
                {
                    Clipboard.SetText(_text.Text);
                    _status.Text = "已複製";
                }
                catch (System.Runtime.InteropServices.ExternalException)
                {
                    _status.Text = "剪貼簿被別的程式占用，再試一次";
                }
            });
        if (_content?.Image is { } bytes)
            Add("另存圖片…", () => SaveImage(bytes));
        if (_webUrl is not null)
            Add("到網頁下載 ↗", () => Web.Open(_webUrl));
        Add("關閉", Close);
    }

    void SaveImage(byte[] bytes)
    {
        var ext = _content?.MimeType switch
        {
            "image/jpeg" => ".jpg",
            "image/webp" => ".webp",
            "image/gif" => ".gif",
            _ => ".png",
        };
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            FileName = Path.GetFileNameWithoutExtension(Title) + ext,
            Filter = $"圖片 (*{ext})|*{ext}",
        };
        if (dialog.ShowDialog(this) != true)
            return;
        File.WriteAllBytes(dialog.FileName, bytes);
        _status.Text = $"已存到 {dialog.FileName}（長邊縮到 1568 px 的版本；原檔請到網頁下載）";
    }

    /// <summary>開預覽；同一個檔案已經開著就拉到前面。</summary>
    public static void Show(TimerEngine engine, string id, string name, Window? owner = null, string? webUrl = null)
    {
        if (Application.Current.Windows.OfType<FilePreviewWindow>().FirstOrDefault(w => w._id == id) is { } open)
        {
            open.Activate();
            return;
        }
        new FilePreviewWindow(engine, id, name, webUrl) { Owner = owner }.Show();
    }
}
