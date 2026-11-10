using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>一筆正在計時：DotBrush 是專案的顏色，ParentPrefix 是「上層任務 › 」（不知道或最上層是空的）。</summary>
public sealed record TeamTimingVm(string Key, string Title, string ParentPrefix, string Since, string Tip, bool CanOpen, Brush DotBrush)
{
    public Cursor? Cursor => CanOpen ? Cursors.Hand : null;
}

public sealed class TeamMemberVm
{
    public required TeamMember Item { get; init; }
    public string Name => Item.User.DisplayName;
    public string Initial => Name.Length > 0 ? Name[..1] : "?";
    public string Job => Item.JobTitle ?? "";
    public Brush DotBrush => Brushes2.Get(Item.OnLeave ? "WarnBrush" : Item.Online ? "RunBrush" : "LineBrush");

    public string State => Item.OnLeave ? $"請假 {string.Join("、", Item.Leaves)}" :
        Item.Timing.Count > 0 ? $"計時中 {Item.Timing.Count}" : Item.Online ? "在線" : "離線";

    public Brush StateBrush => Brushes2.Get(Item.OnLeave ? "WarnBrush" : Item.Timing.Count > 0 ? "RunBrush" : "MutedBrush");
    public Visibility StateVisibility => Visibility.Visible;

    public required IReadOnlyList<TeamTimingVm> Timing { get; init; }
}

/// <summary>
/// 團隊今天的狀況：誰在線、誰請假、誰正在計時哪個任務。
/// 正在計時用專案的顏色、標上層任務（team_schedule 沒給上層，打開這頁才對看得到的任務補讀 get_task，同一個任務只讀一次）。
/// </summary>
public partial class TeamPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;
    int _loadVersion;

    public TeamPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        InitializeComponent();
    }

    public Task ActivateAsync(object? arg) => LoadAsync();

    async Task LoadAsync()
    {
        var version = ++_loadVersion;
        Inputs.Show(StatusText, null);
        CountsText.Text = "讀取中…";
        try
        {
            var team = await _engine.Api.TeamScheduleAsync(CancellationToken.None);
            if (version != _loadVersion)
                return;
            Render(team);
            CountsText.Text = $"共 {team.Count} 人 · 在線 {team.Count(m => m.Online)} · 計時中 {team.Count(m => m.Timing.Count > 0)} · " +
                              $"請假 {team.Count(m => m.OnLeave)}（只列跟你有共同專案的人）";

            var missing = team.SelectMany(m => m.Timing)
                .Where(t => t.CanOpen && t.Task.Length > 0)
                .Select(t => t.Task)
                .Distinct(StringComparer.OrdinalIgnoreCase)
                .Where(key =>
                {
                    _engine.KnownParentTitle(key, out var known);
                    return !known;
                })
                .ToList();
            if (missing.Count == 0)
                return;
            await Task.WhenAll(missing.Select(async key =>
            {
                try
                {
                    await _engine.LoadParentTitleAsync(key, CancellationToken.None);
                }
                catch (Exception ex)
                {
                    Log.Error($"讀取 {key} 的上層任務失敗", ex);
                }
            }));
            if (version == _loadVersion)
                Render(team);
        }
        catch (Exception ex)
        {
            Log.Error("讀取團隊狀況失敗", ex);
            CountsText.Text = "";
            Inputs.Show(StatusText, $"讀取失敗：{ex.Message}");
        }
    }

    void Render(IReadOnlyList<TeamMember> team)
    {
        var me = _engine.User?.Username;
        var people = team
            .OrderBy(m => m.User.Username == me ? 0 : 1)
            .ThenBy(m => m.OnLeave ? 1 : 0)
            .ThenByDescending(m => m.Timing.Count)
            .ThenByDescending(m => m.Online)
            .Select(m => new TeamMemberVm { Item = m, Timing = m.Timing.Select(TimingVm).ToList() })
            .ToList();
        // 兩欄各自往下排，卡片高度不同也不會留白
        LeftList.ItemsSource = people.Where((_, i) => i % 2 == 0).ToList();
        RightList.ItemsSource = people.Where((_, i) => i % 2 == 1).ToList();
    }

    TeamTimingVm TimingVm(TeamTiming t)
    {
        var projectKey = KeyFinder.ProjectOf(t.Task);
        var project = _engine.ProjectOf(t.Project) ?? _engine.ProjectOf(projectKey);
        var brush = new SolidColorBrush(ProjectIcons.ColorOf(project?.IconColor, project?.Key ?? projectKey));
        brush.Freeze();
        var parent = _engine.KnownParentTitle(t.Task, out _);
        return new TeamTimingVm(
            t.Task, t.Title,
            parent is { Length: > 0 } ? parent + " › " : "",
            t.Since is { } since ? $"· {WorkMath.HoursMinutes((DateTimeOffset.Now - since).TotalMinutes)}" : "",
            $"{t.Project}{(parent is { Length: > 0 } ? $"\n上層：{parent}" : "")}{(t.CanOpen ? "\n點一下打開" : "")}",
            t.CanOpen,
            brush);
    }

    async void Reload_Click(object sender, RoutedEventArgs e) => await LoadAsync();

    void Timing_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is TeamTimingVm { CanOpen: true } t)
            _host.ShowPage("tasks", t.Key);
    }
}
