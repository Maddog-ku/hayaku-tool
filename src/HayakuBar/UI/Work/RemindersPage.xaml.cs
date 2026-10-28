using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

public sealed class ReminderListVm
{
    public required ReminderFull Item { get; init; }
    public required string Tab { get; init; }

    public string Text => Item.Text;

    public string When => Item.At is not { } at ? "—" :
        at.ToLocalTime().Date == DateTime.Today ? $"今天 {at.ToLocalTime():HH:mm}" :
        at.ToLocalTime().Date == DateTime.Today.AddDays(1) ? $"明天 {at.ToLocalTime():HH:mm}" :
        at.ToLocalTime().ToString("M/d HH:mm");

    public Brush WhenBrush => Brushes2.Get(Item.Paused || Tab == "fired" ? "MutedBrush" : "TextBrush");

    public string Detail => string.Join(" · ", new[]
    {
        Item.Task,
        Item.RepeatText,
        Item.Paused ? "已暫停" : null,
        Item.Editable ? null : "別人設的",
    }.Where(s => !string.IsNullOrWhiteSpace(s)));

    public Visibility DetailVisibility => Detail.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
    public Visibility SnoozeVisibility => Tab != "repeat" ? Visibility.Visible : Visibility.Collapsed;
    public Visibility PauseVisibility => Tab == "repeat" && Item.Editable ? Visibility.Visible : Visibility.Collapsed;
    public string PauseText => Item.Paused ? "恢復" : "暫停";
    public Visibility TaskVisibility => Item.Task is null ? Visibility.Collapsed : Visibility.Visible;
    public Visibility DeleteVisibility => Item.Editable ? Visibility.Visible : Visibility.Collapsed;
}

/// <summary>提醒：新增（一次或重複、可綁任務）、延後、暫停重複、刪除。</summary>
public partial class RemindersPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    string _tab = "upcoming";
    int _version;

    public RemindersPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        ResetForm();
    }

    /// <summary>arg：任務編號 → 預先填好「綁任務」。</summary>
    public Task ActivateAsync(object? arg)
    {
        if (arg is string key && KeyFinder.Parse(key) is { } k)
        {
            TaskBox.Text = k;
            var title = _engine.TitleOf(k);
            if (MessageInput.Text.Length == 0 && title is not null)
                MessageInput.Text = title;
            MessageInput.Focus();
            MessageInput.SelectAll();
        }
        return LoadAsync();
    }

    void ResetForm()
    {
        var next = DateTime.Now.AddHours(1);
        next = new DateTime(next.Year, next.Month, next.Day, next.Hour, next.Minute < 30 ? 0 : 30, 0);
        DateBox.Text = next.Date == DateTime.Today ? "今天" : next.ToString("M/d");
        TimeBox.Text = next.ToString("HH:mm");
        MessageInput.Text = "";
        TaskBox.Text = "";
        RepeatBox.SelectedIndex = 0;
    }

    async Task LoadAsync()
    {
        var version = ++_version;
        EmptyText.Text = "讀取中…";
        EmptyText.Visibility = Visibility.Visible;
        Inputs.Show(ListStatus, null);
        try
        {
            var items = await _engine.Api.ListRemindersFullAsync(_tab, null, CancellationToken.None);
            if (version != _version)
                return;
            var rows = items
                .OrderBy(r => _tab == "fired" ? -(r.At?.ToUnixTimeSeconds() ?? 0) : r.At?.ToUnixTimeSeconds() ?? long.MaxValue)
                .Select(r => new ReminderListVm { Item = r, Tab = _tab })
                .ToList();
            ReminderList.ItemsSource = rows;
            EmptyText.Text = _tab switch
            {
                "fired" => "沒有已提醒的紀錄",
                "repeat" => "沒有重複提醒",
                _ => "沒有即將到來的提醒，在上面新增一個",
            };
            EmptyText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            FilterText.Text = rows.Count > 0 ? $"{rows.Count} 則" : "";
        }
        catch (Exception ex)
        {
            Log.Error("讀取提醒失敗", ex);
            if (version == _version)
            {
                EmptyText.Visibility = Visibility.Collapsed;
                Inputs.Show(ListStatus, $"讀取失敗：{ex.Message}");
            }
        }
    }

    async void Tab_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string tab } || ReminderList is null)
            return;
        _tab = tab;
        await LoadAsync();
    }

    void Quick_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not string tag)
            return;
        DateTime at;
        if (tag == "tomorrow")
        {
            var start = _engine.Schedule?.Working.FirstOrDefault()?.Start ?? new TimeOnly(9, 0);
            at = DateTime.Today.AddDays(1) + start.ToTimeSpan();
        }
        else
        {
            at = DateTime.Now.AddMinutes(int.Parse(tag));
        }
        DateBox.Text = at.Date == DateTime.Today ? "今天" : at.Date == DateTime.Today.AddDays(1) ? "明天" : at.ToString("M/d");
        TimeBox.Text = at.ToString("HH:mm");
        RepeatBox.SelectedIndex = 0;
        MessageInput.Focus();
    }

    async void Form_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        await AddAsync();
    }

    async void Add_Click(object sender, RoutedEventArgs e) => await AddAsync();

    async Task AddAsync()
    {
        var message = MessageInput.Text.Trim();
        var taskText = TaskBox.Text.Trim();
        string? task = null;
        if (taskText.Length > 0 && (task = KeyFinder.Parse(taskText)) is null)
        {
            Inputs.Show(FormStatus, "綁任務請填任務編號，例 F2P-97");
            return;
        }
        if (message.Length == 0 && task is null)
        {
            Inputs.Show(FormStatus, "請填提醒內容");
            MessageInput.Focus();
            return;
        }
        if (Inputs.ParseDate(DateBox.Text) is not { } date)
        {
            Inputs.Show(FormStatus, "日期看不懂，請用 今天、明天、11/5 這類寫法");
            return;
        }
        if (Inputs.ParseTime(TimeBox.Text) is not { } time)
        {
            Inputs.Show(FormStatus, "時間請用 HH:mm，例 14:30");
            return;
        }
        var args = new Dictionary<string, object?>();
        if (message.Length > 0)
            args["message"] = message;
        if (task is not null)
            args["task"] = task;
        var repeat = (RepeatBox.SelectedItem as ComboBoxItem)?.Tag as string ?? "none";
        if (repeat == "none")
        {
            var at = Inputs.Local(date, time);
            if (at < DateTimeOffset.Now)
            {
                Inputs.Show(FormStatus, "這個時間已經過了");
                return;
            }
            args["at"] = Inputs.Iso(at);
        }
        else
        {
            object rule = repeat switch
            {
                "daily" => new { freq = "DAILY", interval = 1, workdaysOnly = false },
                "workdays" => new { freq = "DAILY", interval = 1, workdaysOnly = true },
                "weekly" => new { freq = "WEEKLY", interval = 1, weekdays = new[] { ((int)date.DayOfWeek + 6) % 7 } },
                _ => (object)new { freq = "MONTHLY", interval = 1, monthDay = date.Day },
            };
            args["repeat"] = new { rule, startDate = Inputs.DateText(date), timeOfDay = time.ToString("HH:mm") };
        }
        if (!await Inputs.Run(AddButton, FormStatus, () => _engine.Api.CreateReminderAsync(args, CancellationToken.None),
                $"已新增：{date.ToString("M/d", Inputs.Zh)} {time:HH:mm} {(message.Length > 0 ? message : task)}"))
            return;
        ResetForm();
        await _engine.AfterExternalChangeAsync();
        await LoadAsync();
    }

    async void Snooze_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not ReminderListVm vm)
            return;
        if (await Inputs.Run((Button)sender, ListStatus, () => _engine.Api.SnoozeReminderAsync(vm.Item.Id, 10, CancellationToken.None)))
            await LoadAsync();
    }

    async void Pause_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not ReminderListVm vm)
            return;
        var args = new Dictionary<string, object?> { ["id"] = vm.Item.Id, ["paused"] = !vm.Item.Paused };
        if (await Inputs.Run((Button)sender, ListStatus, () => _engine.Api.UpdateReminderAsync(args, CancellationToken.None)))
            await LoadAsync();
    }

    void OpenTask_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is ReminderListVm { Item.Task: { } task })
            (Window.GetWindow(this) as WorkspaceWindow)?.ShowPage("tasks", task);
    }

    async void Delete_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not ReminderListVm vm)
            return;
        var ok = MessageBox.Show(Window.GetWindow(this)!, $"刪除提醒「{vm.Text}」？", "刪除提醒",
            MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok != MessageBoxResult.OK)
            return;
        if (await Inputs.Run((Button)sender, ListStatus, () => _engine.Api.DeleteReminderAsync(vm.Item.Id, CancellationToken.None)))
        {
            await _engine.AfterExternalChangeAsync();
            await LoadAsync();
        }
    }
}
