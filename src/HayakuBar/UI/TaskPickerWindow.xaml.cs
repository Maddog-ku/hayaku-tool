using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

public sealed record PickItem(string Key, string Title, string Detail);

public partial class TaskPickerWindow : Window
{
    readonly List<PickItem> _all;

    public TaskPickerWindow(IEnumerable<TaskItem> tasks, string prompt, bool multiSelect, bool showMakeDefault)
    {
        InitializeComponent();
        PromptText.Text = prompt;
        TaskList.SelectionMode = multiSelect ? SelectionMode.Extended : SelectionMode.Single;
        MakeDefaultBox.Visibility = showMakeDefault ? Visibility.Visible : Visibility.Collapsed;
        _all = tasks.Select(t => new PickItem(t.Key, t.Title, $"{t.Project} · {t.Status}")).ToList();
        TaskList.ItemsSource = _all;
        Loaded += (_, _) => SearchBox.Focus();
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);
    }

    public IReadOnlyList<string> SelectedKeys { get; private set; } = [];

    public bool MakeDefault => MakeDefaultBox.IsChecked == true;

    void SearchBox_TextChanged(object sender, TextChangedEventArgs e)
    {
        var q = SearchBox.Text.Trim();
        TaskList.ItemsSource = q.Length == 0
            ? _all
            : _all.Where(t => t.Key.Contains(q, StringComparison.OrdinalIgnoreCase)
                              || t.Title.Contains(q, StringComparison.OrdinalIgnoreCase)
                              || t.Detail.Contains(q, StringComparison.OrdinalIgnoreCase)).ToList();
    }

    void Ok_Click(object sender, RoutedEventArgs e) => Accept();

    void TaskList_MouseDoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (TaskList.SelectedItem is not null)
            Accept();
    }

    void Accept()
    {
        var keys = TaskList.SelectedItems.OfType<PickItem>().Select(p => p.Key).ToList();
        if (keys.Count == 0 && KeyFinder.Parse(SearchBox.Text) is { } typed)
            keys.Add(typed);
        if (keys.Count == 0)
        {
            SearchBox.Focus();
            return;
        }
        SelectedKeys = keys;
        DialogResult = true;
    }
}
