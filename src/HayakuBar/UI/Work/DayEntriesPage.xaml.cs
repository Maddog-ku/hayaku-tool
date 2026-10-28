using System.ComponentModel;
using System.Globalization;
using System.Runtime.CompilerServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Mcp;
using HayakuBar.UI;

namespace HayakuBar.UI.Work;

/// <summary>一段計時紀錄（可直接改起訖、備註）。</summary>
public sealed class EntryRowVm : INotifyPropertyChanged
{
    string _startText = "";
    string _endText = "";
    string? _note;

    public required TimeEntry Entry { get; init; }
    public string Key => Entry.Task;
    public string Title => Entry.Title;
    public bool Running => Entry.Running;
    public Brush DotBrush => Brushes2.Get(Running ? "RunBrush" : "LineBrush");
    public Visibility EndVisibility => Running ? Visibility.Collapsed : Visibility.Visible;
    public Visibility RunningVisibility => Running ? Visibility.Visible : Visibility.Collapsed;
    public string DurationText { get; init; } = "";

    /// <summary>使用者改了起訖或備註（欄位名稱）。</summary>
    public event Action<EntryRowVm, string>? Edited;

    public string StartText
    {
        get => _startText;
        set => Change(ref _startText, value);
    }

    public string EndText
    {
        get => _endText;
        set => Change(ref _endText, value);
    }

    public string? Note
    {
        get => _note;
        set => Change(ref _note, value ?? "");
    }

    public void Init(string start, string end, string? note) => (_startText, _endText, _note) = (start, end, note ?? "");

    void Change<T>(ref T field, T value, [CallerMemberName] string? name = null)
    {
        if (EqualityComparer<T>.Default.Equals(field, value))
            return;
        field = value;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
        Edited?.Invoke(this, name!);
    }

    public event PropertyChangedEventHandler? PropertyChanged;
}

/// <summary>工作台的「工時紀錄」：查看、修改、補登某一天的計時紀錄。</summary>
public partial class DayEntriesPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    DateOnly _day;
    DaySchedule? _schedule;
    bool _busy;

    public DayEntriesPage(TimerEngine engine)
    {
        _engine = engine;
        _day = DateOnly.FromDateTime(DateTime.Today);
        InitializeComponent();
    }

    /// <summary>arg：要看的日期（DateOnly）；沒給就是上次看的那天。</summary>
    public Task ActivateAsync(object? arg)
    {
        if (arg is DateOnly day)
            _day = day;
        NewTask.ItemsSource = _engine.MyTasks.Select(t => $"{t.Key}  {t.Title}").ToList();
        return LoadAsync();
    }

    async Task ShowDayAsync(DateOnly day)
    {
        _day = day;
        await LoadAsync();
    }

    static readonly CultureInfo Zh = Inputs.Zh;

    async Task LoadAsync()
    {
        DateText.Text = _day.ToString("M/d（ddd）", Zh);
        TodayButton.IsEnabled = _day != DateOnly.FromDateTime(DateTime.Today);
        LoadingText.Visibility = Visibility.Visible;
        EmptyText.Visibility = Visibility.Collapsed;
        ShowError(null);
        try
        {
            var day = _day;
            var (entries, schedule) = await _engine.LoadDayAsync(day);
            if (day != _day)
                return; // 讀取途中又換了一天
            _schedule = schedule;
            Render(entries);
        }
        catch (Exception ex)
        {
            ShowError($"讀取失敗：{ex.Message}");
            EntryList.ItemsSource = null;
        }
        finally
        {
            LoadingText.Visibility = Visibility.Collapsed;
        }
    }

    void Render(List<TimeEntry> entries)
    {
        var now = DateTime.Now;
        var dayStart = _day.ToDateTime(TimeOnly.MinValue);
        var rows = entries.Select(e =>
        {
            var minutes = WorkMath.UnionMinutes([e], dayStart, now);
            var row = new EntryRowVm { Entry = e, DurationText = WorkMath.Clock(minutes)[..^3] };
            row.Init(e.StartedAt?.ToLocalTime().ToString("HH:mm") ?? "", e.EndedAt?.ToLocalTime().ToString("HH:mm") ?? "", e.Note);
            row.Edited += OnRowEdited;
            return row;
        }).ToList();
        EntryList.ItemsSource = rows;
        EmptyText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;

        var worked = WorkMath.UnionMinutes(entries, dayStart, now);
        var expected = _schedule?.ExpectedMinutes ?? 0;
        TotalText.Text = $"{WorkMath.HoursMinutes(worked)}" + (expected > 0 ? $" / {WorkMath.HoursMinutes(expected)}" : "");
        TotalText.Foreground = Brushes2.Get(expected > 0 && worked < expected - 5 ? "AccentBrush" : "TextBrush");
        ScheduleText.Text = _schedule is null || _schedule.Working.Count == 0
            ? (_schedule?.Holiday is { } h ? $"放假（{h}）" : "不上班")
            : "班表 " + string.Join("、", _schedule.Working.Select(s => $"{s.Start:HH:mm}–{s.End:HH:mm}")) +
              (expected > 0 && worked < expected ? $"　還差 {WorkMath.HoursMinutes(expected - worked)}" : "");

        Timeline.SetData(dayStart, _schedule?.Working, entries, now);
        RenderGaps(entries, dayStart, now);
    }

    /// <summary>班表時段裡還沒有任何計時的空檔（5 分鐘以上），點一下就帶入補登欄位。</summary>
    void RenderGaps(List<TimeEntry> entries, DateTime dayStart, DateTime now)
    {
        GapsPanel.Children.Clear();
        if (_schedule is null)
            return;
        var gaps = WorkMath.Gaps(_schedule.Working, entries, dayStart, now);
        if (gaps.Count == 0)
        {
            GapsPanel.Children.Add(new TextBlock
            {
                Text = "班表時段都有紀錄了。",
                Foreground = Brushes2.Get("MutedBrush"),
                FontSize = 12,
            });
            return;
        }
        GapsPanel.Children.Add(new TextBlock
        {
            Text = "沒有紀錄的時段：",
            Foreground = Brushes2.Get("MutedBrush"),
            FontSize = 12,
            VerticalAlignment = VerticalAlignment.Center,
            Margin = new Thickness(0, 0, 6, 0),
        });
        foreach (var (start, end) in gaps)
        {
            var chip = new Button
            {
                Content = $"{start:HH:mm}–{end:HH:mm} ＋",
                Style = (Style)FindResource("FlatButton"),
                Margin = new Thickness(0, 2, 6, 2),
                Padding = new Thickness(10, 3, 10, 3),
                ToolTip = "帶入補登欄位",
            };
            chip.Click += (_, _) =>
            {
                NewStart.Text = start.ToString("HH:mm");
                NewEnd.Text = end.ToString("HH:mm");
                NewTask.Focus();
            };
            GapsPanel.Children.Add(chip);
        }
    }

    DateTimeOffset? ParseTime(string text)
    {
        if (!TimeOnly.TryParseExact(text.Trim(), ["H:mm", "HH:mm", "Hmm", "HHmm"], CultureInfo.InvariantCulture, DateTimeStyles.None, out var t))
            return null;
        return new DateTimeOffset(_day.ToDateTime(t), TimeZoneInfo.Local.GetUtcOffset(_day.ToDateTime(t)));
    }

    async void OnRowEdited(EntryRowVm row, string field)
    {
        if (_busy)
            return;
        DateTimeOffset? start = null, end = null;
        string? note = null;
        switch (field)
        {
            case nameof(EntryRowVm.StartText):
                start = ParseTime(row.StartText);
                if (start is null)
                {
                    ShowError("開始時間格式不對，請用 HH:mm（例 09:30）");
                    return;
                }
                break;
            case nameof(EntryRowVm.EndText):
                end = ParseTime(row.EndText);
                if (end is null)
                {
                    ShowError("結束時間格式不對，請用 HH:mm（例 17:30）");
                    return;
                }
                break;
            case nameof(EntryRowVm.Note):
                note = row.Note ?? "";
                break;
            default:
                return;
        }
        var effectiveStart = start ?? row.Entry.StartedAt;
        var effectiveEnd = end ?? row.Entry.EndedAt;
        if (effectiveStart is { } s && effectiveEnd is { } e && e <= s)
        {
            ShowError("結束時間要晚於開始時間");
            return;
        }
        await RunAsync(() => _engine.UpdateTimeEntryAsync(row.Entry.Id, start, end, note));
    }

    async void Delete_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not EntryRowVm row)
            return;
        var ok = MessageBox.Show(Window.GetWindow(this)!, $"刪除 {row.Key} {row.StartText}–{(row.Running ? "進行中" : row.EndText)} 這段紀錄？", "刪除紀錄",
            MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok == MessageBoxResult.OK)
            await RunAsync(() => _engine.DeleteTimeEntryAsync(row.Entry.Id));
    }

    async void Add_Click(object sender, RoutedEventArgs e)
    {
        if (KeyFinder.Parse(NewTask.Text) is not { } task)
        {
            ShowError("請輸入任務編號，例 F2P-97");
            NewTask.Focus();
            return;
        }
        if (ParseTime(NewStart.Text) is not { } start || ParseTime(NewEnd.Text) is not { } end)
        {
            ShowError("請填開始與結束時間（HH:mm）");
            return;
        }
        if (end <= start)
        {
            ShowError("結束時間要晚於開始時間");
            return;
        }
        if (await RunAsync(() => _engine.AddTimeEntryAsync(task, start, end, NewNote.Text)))
        {
            NewStart.Text = NewEnd.Text = NewNote.Text = "";
        }
    }

    /// <summary>執行修改並重新讀取；失敗時顯示 Hayaku 回的錯誤。</summary>
    async Task<bool> RunAsync(Func<Task> action)
    {
        _busy = true;
        ShowError(null);
        IsEnabled = false;
        try
        {
            await action();
            if (_engine.Error is { } error)
            {
                ShowError(error);
                await LoadAsync();
                return false;
            }
            await LoadAsync();
            return true;
        }
        finally
        {
            IsEnabled = true;
            _busy = false;
        }
    }

    void ShowError(string? message)
    {
        ErrorText.Text = message ?? "";
        ErrorText.Visibility = message is null ? Visibility.Collapsed : Visibility.Visible;
    }

    void CommitOnEnter(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter || sender is not TextBox box)
            return;
        box.GetBindingExpression(TextBox.TextProperty)?.UpdateSource();
        Keyboard.ClearFocus();
        e.Handled = true;
    }

    async void PrevDay_Click(object sender, RoutedEventArgs e) => await ShowDayAsync(_day.AddDays(-1));

    async void NextDay_Click(object sender, RoutedEventArgs e) => await ShowDayAsync(_day.AddDays(1));

    async void Today_Click(object sender, RoutedEventArgs e) => await ShowDayAsync(DateOnly.FromDateTime(DateTime.Today));
}
