using System.Globalization;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Markup;
using HayakuBar.Config;
using HayakuBar.Core;

namespace HayakuBar.UI.Work;

/// <summary>我的任務清單的欄位：顯示哪些、順序、寬度（設定存在 config）；標題列拖右邊的線調整寬度。</summary>
public partial class TasksPage
{
    const double TimerColumnWidth = 28;

    List<TaskColumnConfig> _columns = [];

    void InitColumns()
    {
        _columns = TaskColumns.Normalize(_engine.Config.TaskColumns);
        // 列上的 ▶／❚❚ 按鈕：範本是程式產生的，事件統一在清單上接
        TaskList.AddHandler(ButtonBase.ClickEvent, new RoutedEventHandler((_, e) =>
        {
            if (e.OriginalSource is Button { Tag: TaskListRowVm } button)
                RowTimer_Click(button, e);
        }));
        ApplyColumns();
    }

    bool ColumnVisible(string key) => _columns.Any(c => c.Key == key && c.Visible);

    List<TaskColumnConfig> VisibleColumns => _columns.Where(c => c.Visible).ToList();

    static string WidthText(TaskColumnConfig c) =>
        c.Width <= 0 ? "*" : c.Width.ToString("0", CultureInfo.InvariantCulture);

    /// <summary>依目前的欄位設定重建列範本與標題列。</summary>
    void ApplyColumns()
    {
        TaskList.ItemTemplate = BuildRowTemplate(VisibleColumns, ColumnVisible("key"));
        BuildHeader();
    }

    void SaveColumns()
    {
        _engine.Config.TaskColumns = _columns.Select(c => new TaskColumnConfig { Key = c.Key, Visible = c.Visible, Width = c.Width }).ToList();
        _engine.Config.Save();
    }

    static DataTemplate BuildRowTemplate(List<TaskColumnConfig> columns, bool keyColumn)
    {
        var defs = new StringBuilder($"<ColumnDefinition Width=\"{TimerColumnWidth}\" />");
        var cells = new StringBuilder(
            "<Button Grid.Column=\"0\" Content=\"{Binding TimerGlyph}\" ToolTip=\"{Binding TimerTip}\" Tag=\"{Binding}\" " +
            "Style=\"{StaticResource IconButton}\" Foreground=\"{Binding TimerBrush}\" FontSize=\"10\" VerticalAlignment=\"Top\" Margin=\"-4,1,0,0\" />");
        for (var i = 0; i < columns.Count; i++)
        {
            var c = columns[i];
            var col = i + 1;
            defs.Append($"<ColumnDefinition Width=\"{WidthText(c)}\" />");
            const string cell = "Margin=\"0,0,8,0\" VerticalAlignment=\"Top\"";
            cells.Append(c.Key switch
            {
                "key" => $"<TextBlock Grid.Column=\"{col}\" {cell} Text=\"{{Binding Key}}\" FontWeight=\"SemiBold\" TextWrapping=\"NoWrap\" TextTrimming=\"CharacterEllipsis\" />",
                "title" => $"<StackPanel Grid.Column=\"{col}\" {cell}>" +
                           "<TextBlock TextTrimming=\"CharacterEllipsis\" ToolTip=\"{Binding Title}\">" +
                           (keyColumn ? "" : "<Run Text=\"{Binding Key, Mode=OneWay}\" FontWeight=\"SemiBold\" /> ") +
                           "<Run Text=\"{Binding Title, Mode=OneWay}\" /></TextBlock>" +
                           "<TextBlock Text=\"{Binding Context}\" FontSize=\"11\" Foreground=\"{DynamicResource MutedBrush}\" " +
                           "TextTrimming=\"CharacterEllipsis\" Margin=\"0,2,0,0\" Visibility=\"{Binding ContextVisibility}\" /></StackPanel>",
                "project" => $"<ContentControl Grid.Column=\"{col}\" {cell} Content=\"{{Binding ProjectBadge}}\" Focusable=\"False\" ToolTip=\"{{Binding ProjectText}}\" />",
                "status" => $"<Border Grid.Column=\"{col}\" {cell} Style=\"{{StaticResource Pill}}\" HorizontalAlignment=\"Left\" Visibility=\"{{Binding StatusVisibility}}\">" +
                            "<TextBlock Text=\"{Binding Status}\" FontSize=\"11\" TextTrimming=\"CharacterEllipsis\" /></Border>",
                "priority" => $"<TextBlock Grid.Column=\"{col}\" {cell} Text=\"{{Binding PriorityText}}\" FontSize=\"12\" />",
                "start" => $"<TextBlock Grid.Column=\"{col}\" {cell} Text=\"{{Binding StartText}}\" Foreground=\"{{Binding StartBrush}}\" FontSize=\"12\" />",
                "end" => $"<TextBlock Grid.Column=\"{col}\" {cell} Text=\"{{Binding DueText}}\" Foreground=\"{{Binding DueBrush}}\" FontSize=\"12\" />",
                "elapsed" => $"<TextBlock Grid.Column=\"{col}\" {cell} Text=\"{{Binding Elapsed}}\" FontFamily=\"{{StaticResource MonoFont}}\" />",
                _ => "",
            });
        }
        var xaml =
            "<DataTemplate xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\">" +
            $"<Grid Margin=\"0,1\"><Grid.ColumnDefinitions>{defs}</Grid.ColumnDefinitions>{cells}</Grid></DataTemplate>";
        return (DataTemplate)XamlReader.Parse(xaml);
    }

    /// <summary>標題列：跟列一樣的欄寬；固定寬度的欄右邊有一條可以拖的線。</summary>
    void BuildHeader()
    {
        ColumnHeaderGrid.Children.Clear();
        ColumnHeaderGrid.ColumnDefinitions.Clear();
        ColumnHeaderGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(TimerColumnWidth) });
        var visible = VisibleColumns;
        for (var i = 0; i < visible.Count; i++)
        {
            var c = visible[i];
            var def = new ColumnDefinition { Width = c.Width <= 0 ? new GridLength(1, GridUnitType.Star) : new GridLength(c.Width) };
            ColumnHeaderGrid.ColumnDefinitions.Add(def);
            var label = new TextBlock
            {
                Text = TaskColumns.LabelOf(c.Key),
                FontSize = 11,
                Foreground = Brushes2.Get("MutedBrush"),
                TextTrimming = TextTrimming.CharacterEllipsis,
                Margin = new Thickness(0, 0, 8, 0),
            };
            Grid.SetColumn(label, i + 1);
            ColumnHeaderGrid.Children.Add(label);
            if (c.Width <= 0)
                continue;
            var thumb = new Thumb
            {
                Width = 7,
                Cursor = Cursors.SizeWE,
                HorizontalAlignment = HorizontalAlignment.Right,
                Margin = new Thickness(0, 0, -3, 0),
                ToolTip = "拖曳調整寬度",
                Template = ResizeGrip(),
            };
            thumb.DragDelta += (_, e) =>
            {
                c.Width = Math.Clamp(c.Width + e.HorizontalChange, TaskColumns.MinWidth, TaskColumns.MaxWidth);
                def.Width = new GridLength(c.Width);
            };
            thumb.DragCompleted += (_, _) =>
            {
                SaveColumns();
                TaskList.ItemTemplate = BuildRowTemplate(VisibleColumns, ColumnVisible("key"));
            };
            Grid.SetColumn(thumb, i + 1);
            ColumnHeaderGrid.Children.Add(thumb);
        }
    }

    /// <summary>拖曳線：平常是一條淡淡的線，滑過變明顯。</summary>
    static ControlTemplate ResizeGrip() =>
        (ControlTemplate)XamlReader.Parse(
            "<ControlTemplate TargetType=\"Thumb\" xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\">" +
            "<Border Background=\"Transparent\"><Border x:Name=\"Line\" Width=\"1\" Background=\"{DynamicResource LineBrush}\" Margin=\"0,1\" " +
            "xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\" /></Border>" +
            "<ControlTemplate.Triggers><Trigger Property=\"IsMouseOver\" Value=\"True\">" +
            "<Setter TargetName=\"Line\" Property=\"Background\" Value=\"{DynamicResource AccentBrush}\" /></Trigger></ControlTemplate.Triggers>" +
            "</ControlTemplate>");

    /// <summary>「欄位」：勾要顯示的欄、上下調整順序、填寬度，專案欄可以只顯示代碼。</summary>
    void Columns_Click(object sender, RoutedEventArgs e)
    {
        var draft = _columns.Select(c => new TaskColumnConfig { Key = c.Key, Visible = c.Visible, Width = c.Width }).ToList();
        var codeOnly = _engine.Config.TaskProjectCodeOnly;
        var window = new Window
        {
            Title = "清單欄位",
            Owner = Window.GetWindow(this),
            WindowStartupLocation = WindowStartupLocation.CenterOwner,
            SizeToContent = SizeToContent.WidthAndHeight,
            ResizeMode = ResizeMode.NoResize,
            ShowInTaskbar = false,
            FontFamily = (System.Windows.Media.FontFamily)FindResource("UiFont"),
            Background = Brushes2.Get("BgBrush"),
            Foreground = Brushes2.Get("TextBrush"),
        };
        window.SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(window);
        var rowsPanel = new StackPanel();
        var codeBox = new CheckBox { Content = "專案欄只顯示代碼（例 F2P）", IsChecked = codeOnly, Margin = new Thickness(0, 10, 0, 0) };

        void Render()
        {
            rowsPanel.Children.Clear();
            for (var i = 0; i < draft.Count; i++)
            {
                var c = draft[i];
                var row = new Grid { Margin = new Thickness(0, 2, 0, 2) };
                foreach (var w in new[] { 140.0, 34, 34, 90 })
                    row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(w) });
                var check = new CheckBox
                {
                    Content = TaskColumns.LabelOf(c.Key),
                    IsChecked = c.Visible,
                    IsEnabled = c.Key != "title",
                    VerticalAlignment = VerticalAlignment.Center,
                    ToolTip = c.Key == "title" ? "名稱一定要顯示" : null,
                };
                check.Click += (_, _) => c.Visible = check.IsChecked == true;
                row.Children.Add(check);
                var up = new Button { Content = "↑", Style = (Style)FindResource("IconButton"), IsEnabled = i > 0, ToolTip = "往前（往左）" };
                up.Click += (_, _) =>
                {
                    TaskColumns.Move(draft, c.Key, -1);
                    Render();
                };
                Grid.SetColumn(up, 1);
                row.Children.Add(up);
                var down = new Button { Content = "↓", Style = (Style)FindResource("IconButton"), IsEnabled = i < draft.Count - 1, ToolTip = "往後（往右）" };
                down.Click += (_, _) =>
                {
                    TaskColumns.Move(draft, c.Key, 1);
                    Render();
                };
                Grid.SetColumn(down, 2);
                row.Children.Add(down);
                FrameworkElement width = c.Key == "title"
                    ? new TextBlock { Text = "填滿", Foreground = Brushes2.Get("MutedBrush"), VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(6, 0, 0, 0) }
                    : WidthBox(c);
                Grid.SetColumn(width, 3);
                row.Children.Add(width);
                rowsPanel.Children.Add(row);
            }
        }

        FrameworkElement WidthBox(TaskColumnConfig c)
        {
            var box = new TextBox { Text = c.Width.ToString("0", CultureInfo.InvariantCulture), Width = 56, ToolTip = $"寬度（{TaskColumns.MinWidth:0}～{TaskColumns.MaxWidth:0}）" };
            box.LostKeyboardFocus += (_, _) =>
            {
                if (double.TryParse(box.Text, NumberStyles.Float, CultureInfo.InvariantCulture, out var w))
                    c.Width = Math.Clamp(w, TaskColumns.MinWidth, TaskColumns.MaxWidth);
                box.Text = c.Width.ToString("0", CultureInfo.InvariantCulture);
            };
            var panel = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(6, 0, 0, 0) };
            panel.Children.Add(box);
            panel.Children.Add(new TextBlock { Text = " px", Foreground = Brushes2.Get("MutedBrush"), VerticalAlignment = VerticalAlignment.Center });
            return panel;
        }

        Render();
        var ok = new Button { Content = "套用", Style = (Style)FindResource("PrimaryButton"), Padding = new Thickness(16, 5, 16, 5), IsDefault = true };
        ok.Click += (_, _) => window.DialogResult = true;
        var cancel = new Button { Content = "取消", Style = (Style)FindResource("FlatButton"), Padding = new Thickness(16, 5, 16, 5), Margin = new Thickness(8, 0, 0, 0), IsCancel = true };
        var reset = new Button { Content = "恢復預設", Style = (Style)FindResource("LinkButton"), VerticalAlignment = VerticalAlignment.Center };
        reset.Click += (_, _) =>
        {
            draft = TaskColumns.Normalize(null);
            codeBox.IsChecked = false;
            Render();
        };
        var buttons = new DockPanel { Margin = new Thickness(0, 16, 0, 0) };
        DockPanel.SetDock(reset, Dock.Left);
        buttons.Children.Add(reset);
        var right = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right };
        right.Children.Add(ok);
        right.Children.Add(cancel);
        buttons.Children.Add(right);
        window.Content = new StackPanel
        {
            Margin = new Thickness(20, 16, 20, 16),
            Children =
            {
                new TextBlock { Text = "勾要顯示的欄位，用 ↑↓ 調整順序；寬度也可以在清單的標題列直接拖。", Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 0, 0, 10) },
                rowsPanel,
                codeBox,
                buttons,
            },
        };
        if (window.ShowDialog() != true)
            return;
        Keyboard.ClearFocus();
        _columns = TaskColumns.Normalize(draft);
        _engine.Config.TaskProjectCodeOnly = codeBox.IsChecked == true;
        SaveColumns();
        ApplyColumns();
        RenderList(keepSelection: true);
    }
}
