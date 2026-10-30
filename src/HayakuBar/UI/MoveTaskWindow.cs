using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// 移動任務：在同一個專案的任務樹裡挑新的上層，或移到最上層。
/// 自己與自己底下的任務不能當上層；MCP 沒有跨專案移動，要移到別的專案請到網頁。
/// </summary>
public sealed class MoveTaskWindow : Window
{
    sealed record Row(string? Key, string Text, int Depth, double Opacity)
    {
        public Thickness Indent => new(Depth * 18, 0, 0, 0);
    }

    readonly TimerEngine _engine;
    readonly string _task;
    readonly string? _currentParent;
    readonly TextBox _filter = new() { Margin = new Thickness(0, 0, 0, 8) };
    readonly CheckBox _hideDone = new() { Content = "不顯示完成及不處理", VerticalAlignment = VerticalAlignment.Center };
    readonly ListBox _list = new() { Height = 360, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    readonly TextBlock _message = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 6, 0, 0) };
    List<ProjectTask> _tasks = [];

    /// <summary>選的新上層；null＝最上層。</summary>
    public new string? Parent { get; private set; }

    public MoveTaskWindow(TimerEngine engine, string task, string? currentParent)
    {
        _engine = engine;
        _task = task;
        _currentParent = currentParent;
        Title = $"移動 {task}";
        Width = 520;
        SizeToContent = SizeToContent.Height;
        ResizeMode = ResizeMode.NoResize;
        WindowStartupLocation = WindowStartupLocation.CenterOwner;
        ShowInTaskbar = false;
        FontFamily = (System.Windows.Media.FontFamily)FindResource("UiFont");
        Background = Brushes2.Get("BgBrush");
        Foreground = Brushes2.Get("TextBrush");
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);

        _hideDone.IsChecked = engine.Config.TreeHideDone;
        _hideDone.Click += (_, _) =>
        {
            engine.Config.TreeHideDone = _hideDone.IsChecked == true;
            engine.Config.Save();
            Render();
        };
        _filter.TextChanged += (_, _) => Render();
        _list.ItemTemplate = (DataTemplate)System.Windows.Markup.XamlReader.Parse(
            "<DataTemplate xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\">" +
            "<TextBlock Text=\"{Binding Text}\" Margin=\"{Binding Indent}\" Opacity=\"{Binding Opacity}\" TextTrimming=\"CharacterEllipsis\" Padding=\"2,2\" />" +
            "</DataTemplate>");
        _list.MouseDoubleClick += (_, _) => Accept();
        _list.KeyDown += (_, e) =>
        {
            if (e.Key == Key.Enter)
                Accept();
        };

        var ok = new Button { Content = "移到這裡", Style = (Style)FindResource("PrimaryButton"), Padding = new Thickness(16, 5, 16, 5), IsDefault = true };
        ok.Click += (_, _) => Accept();
        var cancel = new Button { Content = "取消", Style = (Style)FindResource("FlatButton"), Padding = new Thickness(16, 5, 16, 5), Margin = new Thickness(8, 0, 0, 0), IsCancel = true };
        var top = new DockPanel();
        DockPanel.SetDock(_hideDone, Dock.Right);
        top.Children.Add(_hideDone);
        _filter.Margin = new Thickness(0, 0, 10, 8);
        top.Children.Add(_filter);
        Content = new StackPanel
        {
            Margin = new Thickness(20, 16, 20, 16),
            Children =
            {
                new TextBlock
                {
                    Text = $"選 {task} 的新上層（同一個專案）。要移到別的專案請在網頁上操作。",
                    TextWrapping = TextWrapping.Wrap, Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 0, 0, 10),
                },
                top,
                _list,
                _message,
                new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 12, 0, 0), Children = { ok, cancel } },
            },
        };
        Loaded += async (_, _) => await LoadAsync();
    }

    async Task LoadAsync()
    {
        _message.Text = "讀取中…";
        try
        {
            _tasks = await _engine.Api.ListProjectTasksAsync(KeyFinder.ProjectOf(_task), CancellationToken.None);
            _message.Text = "";
            Render();
            _filter.Focus();
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {_task} 的專案任務失敗", ex);
            _message.Text = $"讀不到專案的任務：{ex.Message}";
            _message.Foreground = Brushes2.Get("ErrorBrush");
        }
    }

    /// <summary>自己和自己底下的任務（不能當上層）。</summary>
    HashSet<string> SelfAndDescendants()
    {
        var set = new HashSet<string>(StringComparer.OrdinalIgnoreCase) { _task };
        bool added;
        do
        {
            added = false;
            foreach (var t in _tasks)
                if (t.Parent is { } p && set.Contains(p) && set.Add(t.Key))
                    added = true;
        } while (added);
        return set;
    }

    void Render()
    {
        var excluded = SelfAndDescendants();
        var roots = TaskTree.Build(_tasks.Where(t => !excluded.Contains(t.Key)), _hideDone.IsChecked != true, _filter.Text);
        var rows = new List<Row> { new(null, "⤒ 最上層（不屬於任何任務）" + (_currentParent is null ? "　· 目前" : ""), 0, 1) };
        void Add(IEnumerable<TaskTreeNode> nodes, int depth)
        {
            foreach (var n in nodes)
            {
                var t = n.Task;
                var current = string.Equals(t.Key, _currentParent, StringComparison.OrdinalIgnoreCase) ? "　· 目前" : "";
                var category = t.IsCategory ? "　· 分類" : "";
                rows.Add(new Row(t.Key, $"{t.Key} {t.Title}{category}{current}", depth, t.StatusCategory is "DONE" or "CANCELLED" ? 0.55 : 1));
                Add(n.Children, depth + 1);
            }
        }
        Add(roots, 0);
        _list.ItemsSource = rows;
        _list.SelectedIndex = 0;
    }

    void Accept()
    {
        if (_list.SelectedItem is not Row row)
            return;
        if (string.Equals(row.Key, _currentParent, StringComparison.OrdinalIgnoreCase))
        {
            _message.Text = "已經在這裡了，選別的位置";
            _message.Foreground = Brushes2.Get("WarnBrush");
            return;
        }
        Parent = row.Key;
        DialogResult = true;
    }
}
