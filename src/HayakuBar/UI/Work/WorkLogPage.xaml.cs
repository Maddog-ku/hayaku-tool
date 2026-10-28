using System.Text;
using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>某天的工作日誌：照計時紀錄整理的任務與時數，加上自己在網頁寫的日誌。</summary>
public partial class WorkLogPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    DateOnly _day = DateOnly.FromDateTime(DateTime.Today);
    WorkLog? _log;
    int _version;

    public WorkLogPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
    }

    public Task ActivateAsync(object? arg)
    {
        if (arg is DateOnly d)
            _day = d;
        return LoadAsync();
    }

    async Task LoadAsync()
    {
        var version = ++_version;
        DateText.Text = _day.ToString("M/d（ddd）", Inputs.Zh) + (_day == DateOnly.FromDateTime(DateTime.Today) ? " 今天" : "");
        TodayButton.IsEnabled = _day != DateOnly.FromDateTime(DateTime.Today);
        Inputs.Show(StatusText, null);
        TotalText.Text = "讀取中…";
        try
        {
            var log = await _engine.Api.GetWorkLogAsync(_day, CancellationToken.None);
            if (version != _version)
                return;
            _log = log;
            Render(log);
        }
        catch (Exception ex)
        {
            Log.Error("讀取工作日誌失敗", ex);
            if (version == _version)
            {
                TotalText.Text = "—";
                Inputs.Show(StatusText, $"讀取失敗：{ex.Message}");
            }
        }
    }

    void Render(WorkLog log)
    {
        TotalText.Text = $"共 {WorkMath.HoursMinutes(log.TotalMinutes)}";
        EntryList.ItemsSource = log.Entries.Select(e => new
        {
            e.Task,
            Hours = WorkMath.HoursMinutes(e.Minutes),
            Ranges = string.Join("、", e.Ranges.Select(r => $"{r.Start.ToLocalTime():HH:mm}–{r.End.ToLocalTime():HH:mm}")),
        }).ToList();
        EmptyText.Visibility = log.Entries.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        NoteText.Text = log.Markdown;
        NoteSection.Visibility = string.IsNullOrWhiteSpace(log.Markdown) ? Visibility.Collapsed : Visibility.Visible;
        CopyButton.IsEnabled = log.Entries.Count > 0;
    }

    void Copy_Click(object sender, RoutedEventArgs e)
    {
        if (_log is null)
            return;
        var sb = new StringBuilder();
        sb.AppendLine($"{_day:yyyy/M/d} 工作日誌（共 {WorkMath.HoursMinutes(_log.TotalMinutes)}）");
        foreach (var entry in _log.Entries)
            sb.AppendLine($"- {entry.Task}　{WorkMath.HoursMinutes(entry.Minutes)}");
        if (!string.IsNullOrWhiteSpace(_log.Markdown))
            sb.AppendLine().AppendLine(_log.Markdown.Trim());
        try
        {
            Clipboard.SetText(sb.ToString());
            Inputs.Show(StatusText, "已複製到剪貼簿", error: false);
        }
        catch (Exception ex)
        {
            Inputs.Show(StatusText, $"複製失敗：{ex.Message}");
        }
    }

    void Entries_Click(object sender, RoutedEventArgs e) =>
        (Window.GetWindow(this) as WorkspaceWindow)?.OpenDay(_day);

    void Web_Click(object sender, RoutedEventArgs e) => Web.Open(_engine.WebUrl("/notebook"));

    async void Prev_Click(object sender, RoutedEventArgs e)
    {
        _day = _day.AddDays(-1);
        await LoadAsync();
    }

    async void Next_Click(object sender, RoutedEventArgs e)
    {
        _day = _day.AddDays(1);
        await LoadAsync();
    }

    async void Today_Click(object sender, RoutedEventArgs e)
    {
        _day = DateOnly.FromDateTime(DateTime.Today);
        await LoadAsync();
    }
}
