using System.Windows;
using System.Windows.Controls;

namespace HayakuBar.UI.Work;

/// <summary>筆記、團隊空間、論壇頁共用的小元件：按鈕、淡色文字、唯讀內文、輸入框、標題列、兩欄版面。</summary>
static class PageParts
{
    public static Button Button(FrameworkElement owner, string text, bool primary = false) => new()
    {
        Content = text, Style = (Style)owner.FindResource(primary ? "PrimaryButton" : "FlatButton"),
        Padding = new Thickness(12, 5, 12, 5), Margin = new Thickness(0, 0, 6, 0), VerticalAlignment = VerticalAlignment.Center,
    };

    public static TextBlock Muted(string text = "", double size = 12)
    {
        var t = new TextBlock { Text = text, FontSize = size, TextWrapping = TextWrapping.Wrap };
        t.SetResourceReference(TextBlock.ForegroundProperty, "MutedBrush");
        return t;
    }

    /// <summary>唯讀、可選取複製的 Markdown 內文（不轉排版，照原文顯示）。</summary>
    public static TextBox Reader(FrameworkElement owner) => new()
    {
        Style = (Style)owner.FindResource("PlainText"), FontSize = 13, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled,
    };

    /// <summary>有提示字的輸入框；multiline 時可以換行。</summary>
    public static TextBox Input(FrameworkElement owner, string hint, bool multiline = false)
    {
        var box = new TextBox { Style = (Style)owner.FindResource("HintBox"), Tag = hint, Margin = new Thickness(0, 0, 0, 6) };
        if (multiline)
        {
            box.AcceptsReturn = true;
            box.TextWrapping = TextWrapping.Wrap;
            box.MinHeight = 70;
            box.MaxHeight = 220;
            box.VerticalContentAlignment = VerticalAlignment.Top;
            box.VerticalScrollBarVisibility = ScrollBarVisibility.Auto;
        }
        return box;
    }

    /// <summary>頁標題，右邊放按鈕（重新整理、在網頁開啟…）。</summary>
    public static Grid TitleBar(FrameworkElement owner, string title, params UIElement[] buttons)
    {
        var bar = new Grid { Margin = new Thickness(0, 0, 0, 10) };
        bar.Children.Add(new TextBlock { Text = title, Style = (Style)owner.FindResource("PageTitle"), Margin = new Thickness(0), VerticalAlignment = VerticalAlignment.Center });
        var right = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center };
        foreach (var b in buttons)
            right.Children.Add(b);
        bar.Children.Add(right);
        return bar;
    }

    /// <summary>左邊清單（固定寬）、右邊內容的兩欄。</summary>
    public static Grid TwoColumns(UIElement left, UIElement right, double leftWidth = 300)
    {
        var grid = new Grid();
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(leftWidth) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(18) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        Grid.SetColumn(right, 2);
        grid.Children.Add(left);
        grid.Children.Add(right);
        return grid;
    }

    /// <summary>上方固定幾列、最後一個元素填滿剩下空間。</summary>
    public static DockPanel Stack(Thickness margin, params UIElement[] parts)
    {
        var dock = new DockPanel { Margin = margin, LastChildFill = true };
        for (var i = 0; i < parts.Length; i++)
        {
            if (i < parts.Length - 1)
                DockPanel.SetDock(parts[i], Dock.Top);
            dock.Children.Add(parts[i]);
        }
        return dock;
    }

    /// <summary>清單的一列：粗體標題＋淡色說明。</summary>
    public static ListBoxItem Row(string title, string? meta, object tag)
    {
        var panel = new StackPanel { Margin = new Thickness(2, 3, 2, 3) };
        panel.Children.Add(new TextBlock { Text = title, FontWeight = FontWeights.SemiBold, TextTrimming = TextTrimming.CharacterEllipsis });
        if (!string.IsNullOrWhiteSpace(meta))
            panel.Children.Add(Muted(meta, 11));
        return new ListBoxItem { Content = panel, Tag = tag, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    }

    public static ListBox List() => new()
    {
        BorderThickness = new Thickness(0), Background = System.Windows.Media.Brushes.Transparent,
        HorizontalContentAlignment = HorizontalAlignment.Stretch,
    };

    public static string Size(long? bytes) => bytes switch
    {
        null => "",
        < 1024 => $"{bytes} B",
        < 1024 * 1024 => $"{bytes / 1024.0:0.#} KB",
        _ => $"{bytes / 1024.0 / 1024.0:0.#} MB",
    };
}
