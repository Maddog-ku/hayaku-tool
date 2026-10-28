using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Controls;
using HayakuBar.Claude;
using HayakuBar.Config;
using HayakuBar.Core;
using HayakuBar.Infra;
using Microsoft.Win32;

namespace HayakuBar.UI;

public sealed class RepoItem : INotifyPropertyChanged
{
    string? _defaultTask;
    string _hooksText = "";
    bool _enabled = true;

    public required string Path { get; init; }

    public string? DefaultTask
    {
        get => _defaultTask;
        set => Set(ref _defaultTask, value);
    }

    public bool Enabled
    {
        get => _enabled;
        set => Set(ref _enabled, value);
    }

    public string HooksText
    {
        get => _hooksText;
        set => Set(ref _hooksText, value);
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    void Set<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
            return;
        field = value;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
    }
}

public partial class SettingsWindow : Window
{
    readonly TimerEngine _engine;
    readonly ObservableCollection<RepoItem> _repos = [];

    public SettingsWindow(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        var c = engine.Config;
        AutoScheduleBox.IsChecked = c.AutoSchedule;
        ClaudeEnabledBox.IsChecked = c.ClaudeEnabled;
        ClaudeOnlyWorkBox.IsChecked = c.ClaudeOnlyDuringWork;
        PollBox.Text = c.PollSeconds.ToString();
        StartupBox.IsChecked = StartupRegistration.IsEnabled();
        AnchorBox.SelectedValue = c.TaskbarAnchor;
        if (AnchorBox.SelectedIndex < 0)
            AnchorBox.SelectedValue = "BeforeTray";
        OffsetBox.Text = Math.Round(c.TaskbarOffset).ToString();
        if (string.Equals(c.TaskbarLayout, "OneLine", StringComparison.OrdinalIgnoreCase))
            OneLineRadio.IsChecked = true;
        else
            TwoLineRadio.IsChecked = true;
        foreach (var r in c.Repos)
            _repos.Add(new RepoItem { Path = r.Path, DefaultTask = r.DefaultTask, Enabled = r.Enabled, HooksText = HooksStatus(r.Path) });
        RepoList.ItemsSource = _repos;
        UpdateAccount();
        _engine.Changed += UpdateAccount;
        Closed += (_, _) => _engine.Changed -= UpdateAccount;
    }

    RepoItem? Selected => RepoList.SelectedItem as RepoItem;

    void UpdateAccount()
    {
        AccountText.Text = _engine.LoggedIn
            ? $"已登入：{_engine.User?.DisplayName}（{_engine.User?.Username}）"
            : _engine.LoggingIn ? "等待瀏覽器登入…" : "尚未登入";
        AccountButton.Content = _engine.LoggedIn ? "登出" : "登入";
        AccountButton.IsEnabled = !_engine.LoggingIn;
    }

    async void Account_Click(object sender, RoutedEventArgs e)
    {
        if (_engine.LoggedIn)
            await _engine.LogoutAsync();
        else
            await _engine.LoginAsync();
    }

    string HooksStatus(string path)
    {
        try
        {
            return HookInstaller.IsInstalled(path, _engine.Config.HookPort) ? "已安裝" : "未安裝";
        }
        catch
        {
            return "讀取失敗";
        }
    }

    void AddRepo_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog { Title = "選擇用 Claude Code 開發的 repo 資料夾" };
        if (dialog.ShowDialog(this) != true)
            return;
        var path = ClaudeMonitor.Normalize(dialog.FolderName);
        if (_repos.Any(r => string.Equals(r.Path, path, StringComparison.OrdinalIgnoreCase)))
            return;
        var item = new RepoItem { Path = path, HooksText = HooksStatus(path) };
        _repos.Add(item);
        RepoList.SelectedItem = item;
        if (!Directory.Exists(Path.Combine(ClaudeMonitor.ProjectsRoot, ClaudeMonitor.Encode(path))))
            MessageBox.Show(this, "這個資料夾目前還沒有 Claude Code 的 session 紀錄。之後在這裡開 Claude Code 就會自動偵測。",
                "HayakuBar", MessageBoxButton.OK, MessageBoxImage.Information);
    }

    void RemoveRepo_Click(object sender, RoutedEventArgs e)
    {
        if (Selected is { } item)
            _repos.Remove(item);
    }

    void RepoList_SelectionChanged(object sender, SelectionChangedEventArgs e) =>
        DefaultTaskBox.Text = Selected?.DefaultTask ?? "";

    void DefaultTaskBox_LostFocus(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } item)
            return;
        var text = DefaultTaskBox.Text.Trim();
        item.DefaultTask = text.Length == 0 ? null : KeyFinder.Parse(text) ?? text.ToUpperInvariant();
        DefaultTaskBox.Text = item.DefaultTask ?? "";
    }

    void InstallHooks_Click(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } item)
        {
            MessageBox.Show(this, "先在清單選一個 repo。", "HayakuBar");
            return;
        }
        var file = HookInstaller.SettingsPath(item.Path);
        var ok = MessageBox.Show(this,
            $"會在下面這個檔案加入 UserPromptSubmit／Stop／SessionEnd 三個 hook（原檔會備份成 .hayakubar.bak）：\n\n{file}\n\n確定要安裝嗎？",
            "安裝 Claude hooks", MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok != MessageBoxResult.OK)
            return;
        try
        {
            HookInstaller.Install(item.Path, _engine.Config.HookPort);
            item.HooksText = HooksStatus(item.Path);
        }
        catch (Exception ex)
        {
            Log.Error("安裝 hooks 失敗", ex);
            MessageBox.Show(this, $"安裝失敗：{ex.Message}", "HayakuBar", MessageBoxButton.OK, MessageBoxImage.Error);
        }
    }

    void UninstallHooks_Click(object sender, RoutedEventArgs e)
    {
        if (Selected is not { } item)
            return;
        try
        {
            HookInstaller.Uninstall(item.Path, _engine.Config.HookPort);
            item.HooksText = HooksStatus(item.Path);
        }
        catch (Exception ex)
        {
            Log.Error("移除 hooks 失敗", ex);
            MessageBox.Show(this, $"移除失敗：{ex.Message}", "HayakuBar", MessageBoxButton.OK, MessageBoxImage.Error);
        }
    }

    void Save_Click(object sender, RoutedEventArgs e)
    {
        DefaultTaskBox_LostFocus(sender, e);
        var c = _engine.Config;
        c.AutoSchedule = AutoScheduleBox.IsChecked == true;
        c.ClaudeEnabled = ClaudeEnabledBox.IsChecked == true;
        c.ClaudeOnlyDuringWork = ClaudeOnlyWorkBox.IsChecked == true;
        if (int.TryParse(PollBox.Text, out var poll))
            c.PollSeconds = Math.Clamp(poll, 10, 600);
        if (AnchorBox.SelectedValue is string anchor)
            c.TaskbarAnchor = anchor;
        c.TaskbarLayout = OneLineRadio.IsChecked == true ? "OneLine" : "TwoLine";
        if (double.TryParse(OffsetBox.Text, out var offset))
            c.TaskbarOffset = Math.Clamp(offset, -3000, 3000);
        c.Repos = _repos.Select(r => new RepoConfig { Path = r.Path, DefaultTask = r.DefaultTask, Enabled = r.Enabled }).ToList();
        try
        {
            StartupRegistration.Set(StartupBox.IsChecked == true);
        }
        catch (Exception ex)
        {
            Log.Error("設定開機啟動失敗", ex);
        }
        _engine.ApplyConfig();
        Close();
    }
}
