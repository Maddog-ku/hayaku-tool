using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

public sealed class DayCellVm
{
    public DateOnly? Date { get; init; }
    public string DayText { get; init; } = "";
    public string Tag { get; init; } = "";
    public string Value { get; init; } = "";
    public string Tip { get; init; } = "";
    public Brush Background { get; init; } = Brushes.Transparent;
    public Brush Border { get; init; } = Brushes.Transparent;
    public Thickness BorderThickness { get; init; }
    public Brush ValueBrush { get; init; } = Brushes2.Get("TextBrush");
    public FontWeight DayWeight { get; init; } = FontWeights.Normal;
    public Cursor? Cursor { get; init; }
    public double Opacity { get; init; } = 1;
}

/// <summary>我的工時月曆：每天填了多少、應出勤多少；點一天開工時紀錄補登。</summary>
public partial class HoursPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    readonly WorkspaceWindow _host;
    DateOnly _month = new(DateTime.Today.Year, DateTime.Today.Month, 1);
    int _version;

    public HoursPage(TimerEngine engine, WorkspaceWindow host)
    {
        _engine = engine;
        _host = host;
        InitializeComponent();
    }

    public Task ActivateAsync(object? arg)
    {
        if (arg is DateOnly d)
            _month = new DateOnly(d.Year, d.Month, 1);
        return LoadAsync();
    }

    async Task LoadAsync()
    {
        var version = ++_version;
        var month = _month;
        MonthText.Text = month.ToString("yyyy 年 M 月");
        ThisMonthButton.IsEnabled = month != new DateOnly(DateTime.Today.Year, DateTime.Today.Month, 1);
        LoadingText.Text = "讀取中…";
        LoadingText.Visibility = Visibility.Visible;
        var last = month.AddMonths(1).AddDays(-1);
        try
        {
            var schedule = await _engine.Api.GetScheduleRangeAsync(month, last, CancellationToken.None);
            var entries = await _engine.Api.ListTimeEntriesAsync(month, last, CancellationToken.None);
            if (version != _version)
                return;
            Render(month, schedule, entries);
        }
        catch (Exception ex)
        {
            Log.Error("讀取工時月曆失敗", ex);
            if (version == _version)
                LoadingText.Text = $"讀取失敗：{ex.Message}";
            return;
        }
        LoadingText.Visibility = Visibility.Collapsed;
    }

    void Render(DateOnly month, List<DaySchedule> schedule, List<TimeEntry> entries)
    {
        var today = DateOnly.FromDateTime(DateTime.Today);
        var now = DateTime.Now;
        var byDay = schedule.GroupBy(s => s.Date).ToDictionary(g => g.Key, g => g.First());
        var timed = entries.Where(e => e.StartedAt is not null).ToList();
        var cells = new List<DayCellVm>();
        var lead = ((int)month.DayOfWeek + 6) % 7; // 週一開始
        for (var i = 0; i < lead; i++)
            cells.Add(new DayCellVm());

        double filledTotal = 0;
        var expectedTotal = 0;
        var expectedToDate = 0;
        double missingToDate = 0;
        int partialDays = 0, emptyDays = 0;
        for (var d = month; d.Month == month.Month; d = d.AddDays(1))
        {
            var day = d.ToDateTime(TimeOnly.MinValue);
            var worked = d > today ? 0 : WorkMath.UnionMinutes(timed, day, d == today ? now : day.AddDays(1));
            var s = byDay.GetValueOrDefault(d);
            var expected = s?.ExpectedMinutes ?? 0;
            filledTotal += worked;
            expectedTotal += expected;
            var past = d < today;
            if (d <= today)
            {
                expectedToDate += expected;
                missingToDate += Math.Max(0, expected - worked);
            }

            string tag = s?.Holiday ?? (s?.OnLeave == true ? "請假" : "");
            Brush bg;
            string value;
            var valueBrush = Brushes2.Get("TextBrush");
            if (expected == 0)
            {
                bg = Brushes2.Get("OffDayBrush");
                value = worked > 0 ? WorkMath.HoursText(worked) : "";
            }
            else if (d > today)
            {
                bg = Brushes2.Get("BgBrush");
                value = $"應 {WorkMath.HoursText(expected)}";
                valueBrush = Brushes2.Get("MutedBrush");
            }
            else if (worked >= expected - 5)
            {
                bg = Brushes2.Get("FilledBrush");
                value = $"{WorkMath.HoursText(worked)} / {WorkMath.HoursText(expected)}";
            }
            else if (worked > 0)
            {
                bg = Brushes2.Get(past ? "PartialBrush" : "BgBrush");
                value = $"{WorkMath.HoursText(worked)} / {WorkMath.HoursText(expected)}";
                if (past)
                    partialDays++;
            }
            else
            {
                bg = Brushes2.Get(past ? "MissingBrush" : "BgBrush");
                value = past ? $"沒填 · 應 {WorkMath.HoursText(expected)}" : $"0 / {WorkMath.HoursText(expected)}";
                valueBrush = Brushes2.Get(past ? "ErrorBrush" : "MutedBrush");
                if (past)
                    emptyDays++;
            }
            var isToday = d == today;
            cells.Add(new DayCellVm
            {
                Date = d,
                DayText = d.Day.ToString(),
                Tag = isToday ? "今天" : tag,
                Value = value,
                Tip = $"{d.ToString("M/d（ddd）", Inputs.Zh)}：已填 {WorkMath.HoursMinutes(worked)}" +
                      (expected > 0 ? $"／應出勤 {WorkMath.HoursMinutes(expected)}" : "") + "\n點一下查看、補登",
                Background = bg,
                Border = Brushes2.Get(isToday ? "AccentBrush" : "LineBrush"),
                BorderThickness = new Thickness(isToday ? 2 : 1),
                ValueBrush = valueBrush,
                DayWeight = isToday ? FontWeights.Bold : FontWeights.SemiBold,
                Cursor = Cursors.Hand,
                Opacity = d > today ? 0.75 : 1,
            });
        }
        while (cells.Count % 7 != 0)
            cells.Add(new DayCellVm());
        DayGrid.ItemsSource = cells;

        FilledText.Text = WorkMath.HoursText(filledTotal);
        ExpectedText.Text = WorkMath.HoursText(expectedTotal);
        ExpectedToDateText.Text = $"到今天 {WorkMath.HoursMinutes(expectedToDate)}";
        MissingText.Text = WorkMath.HoursText(missingToDate);
        MissingText.Foreground = Brushes2.Get(missingToDate > 5 ? "ErrorBrush" : "TextBrush");
        UnfilledText.Text = $"{partialDays + emptyDays} 天";
        UnfilledDetail.Text = $"不足 {partialDays} · 沒填 {emptyDays}";
    }

    void Day_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is DayCellVm { Date: { } d })
            _host.OpenDay(d);
    }

    async void Prev_Click(object sender, RoutedEventArgs e)
    {
        _month = _month.AddMonths(-1);
        await LoadAsync();
    }

    async void Next_Click(object sender, RoutedEventArgs e)
    {
        _month = _month.AddMonths(1);
        await LoadAsync();
    }

    async void ThisMonth_Click(object sender, RoutedEventArgs e)
    {
        _month = new DateOnly(DateTime.Today.Year, DateTime.Today.Month, 1);
        await LoadAsync();
    }
}
