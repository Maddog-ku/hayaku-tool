using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

public sealed record TeamTimingVm(string Key, string Title, string Since, string Tip, bool CanOpen)
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

    public IReadOnlyList<TeamTimingVm> Timing => Item.Timing.Select(t => new TeamTimingVm(
        t.Task, t.Title,
        t.Since is { } since ? $"· {WorkMath.HoursMinutes((DateTimeOffset.Now - since).TotalMinutes)}" : "",
        $"{t.Project}{(t.CanOpen ? "\n點一下打開" : "")}",
        t.CanOpen)).ToList();
}

/// <summary>團隊今天的狀況：誰在線、誰請假、誰正在計時哪個任務。</summary>
public partial class TeamPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;

    public TeamPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        InitializeComponent();
    }

    public Task ActivateAsync(object? arg) => LoadAsync();

    async Task LoadAsync()
    {
        Inputs.Show(StatusText, null);
        CountsText.Text = "讀取中…";
        try
        {
            var team = await _engine.Api.TeamScheduleAsync(CancellationToken.None);
            var me = _engine.User?.Username;
            var people = team
                .OrderBy(m => m.User.Username == me ? 0 : 1)
                .ThenBy(m => m.OnLeave ? 1 : 0)
                .ThenByDescending(m => m.Timing.Count)
                .ThenByDescending(m => m.Online)
                .Select(m => new TeamMemberVm { Item = m })
                .ToList();
            // 兩欄各自往下排，卡片高度不同也不會留白
            LeftList.ItemsSource = people.Where((_, i) => i % 2 == 0).ToList();
            RightList.ItemsSource = people.Where((_, i) => i % 2 == 1).ToList();
            CountsText.Text = $"共 {team.Count} 人 · 在線 {team.Count(m => m.Online)} · 計時中 {team.Count(m => m.Timing.Count > 0)} · " +
                              $"請假 {team.Count(m => m.OnLeave)}（只列跟你有共同專案的人）";
        }
        catch (Exception ex)
        {
            Log.Error("讀取團隊狀況失敗", ex);
            CountsText.Text = "";
            Inputs.Show(StatusText, $"讀取失敗：{ex.Message}");
        }
    }

    async void Reload_Click(object sender, RoutedEventArgs e) => await LoadAsync();

    void Timing_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is TeamTimingVm { CanOpen: true } t)
            _host.ShowPage("tasks", t.Key);
    }
}
