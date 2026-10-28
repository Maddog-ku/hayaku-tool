using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.UI.Work;

/// <summary>我的統計：期間內實際工時、應出勤、完成與逾期任務、各專案工時。</summary>
public partial class StatsPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    string _preset = "thisMonth";
    int _version;

    public StatsPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        DefaultPreset.IsChecked = true;
    }

    public Task ActivateAsync(object? arg) => LoadAsync();

    async void Preset_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string preset } || ProjectList is null)
            return;
        _preset = preset;
        await LoadAsync();
    }

    async Task LoadAsync()
    {
        var version = ++_version;
        Inputs.Show(StatusText, null);
        ActualText.Text = ExpectedText.Text = DoneText.Text = OverdueText.Text = "…";
        try
        {
            var s = await _engine.Api.MyStatsAsync(_preset, CancellationToken.None);
            if (version != _version)
                return;
            ActualText.Text = WorkMath.HoursText(s.ActualMinutes);
            ExpectedText.Text = WorkMath.HoursText(s.ExpectedMinutes);
            WorkdaysText.Text = $"{s.Workdays} 個工作天，有計時 {s.ActiveDays} 天";
            DoneText.Text = s.Done.ToString();
            AssignedText.Text = $"指派給我 {s.Assigned} 個";
            OverdueText.Text = s.Overdue.ToString();
            OverdueText.Foreground = Brushes2.Get(s.Overdue > 0 ? "ErrorBrush" : "TextBrush");
            var max = Math.Max(1, s.ByProject.Select(p => p.Minutes).DefaultIfEmpty(0).Max());
            ProjectList.ItemsSource = s.ByProject.Select(p => new
            {
                Name = p.Project,
                Hours = WorkMath.HoursMinutes(p.Minutes),
                Ratio = (double)p.Minutes / max,
            }).ToList();
            ProjectEmpty.Visibility = s.ByProject.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        }
        catch (Exception ex)
        {
            Log.Error("讀取統計失敗", ex);
            if (version == _version)
                Inputs.Show(StatusText, $"讀取失敗：{ex.Message}");
        }
    }

    void Web_Click(object sender, RoutedEventArgs e) => Web.Open(_engine.WebUrl("/stats"));
}
