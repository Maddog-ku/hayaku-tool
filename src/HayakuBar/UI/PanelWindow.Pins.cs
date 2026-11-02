using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// 托盤上的一個釘選任務或會議；正在計時的是深色。會議的 ▶ 是加入會議（開會前 10 分鐘起），
/// 已取消、已結束的會議變灰、不能加入。
/// </summary>
public sealed class PinRowVm : INotifyPropertyChanged
{
    string _elapsed = "";
    bool _busy;

    public required PinItem Item { get; init; }
    public required int Number { get; init; }
    public required bool Running { get; init; }
    public string Key => Item.Key;
    public string Title => Item.IsMeeting ? $"🗓 {Item.Title}" : Item.Title;
    public string Index => Number <= 9 ? Number.ToString() : "";

    /// <summary>現在的時間（判斷會議能不能加入）。</summary>
    public DateTimeOffset Now { get; init; } = DateTimeOffset.Now;

    bool Gone => Item.IsMeeting && (Item.Cancelled || Item.Ended(Now));

    string MeetingTime => Item.StartAt is { } s
        ? $"{s.ToLocalTime():M/d HH:mm}" + (Item.EndAt is { } e ? $"–{e.ToLocalTime():HH:mm}" : "")
        : "";

    public string Tip => Item.IsMeeting
        ? $"會議 {Item.Key} {Item.Title}\n{MeetingTime}" +
          (Item.Cancelled ? "\n已取消" : Item.Ended(Now) ? "\n已結束" : Item.CanJoin(Now) ? "\n▶ 加入會議並計時" : "\n開會前 10 分鐘起可以加入") +
          (Number <= 9 && Item.CanJoin(Now) ? $"\nAlt+{Number}：加入" : "") + "\n點一下看詳細，右鍵更多"
        : $"{Item.Key} {Item.Title}\n{Item.Project} · {Item.Status}" +
          (Item.CanTime ? "" : "\n不能計時（分類任務、已完成或只能檢視）") +
          (Number <= 9 ? $"\nAlt+{Number}：切換到這個任務" : "") + "\n點一下看詳細，右鍵更多";

    public double ChipOpacity => Gone ? 0.45 : 1;

    public Brush ChipBrush => Brushes2.Get(Running ? "RunCardBrush" : "SubtleBrush");
    public Thickness ChipBorder => new(Running ? 0 : 1);
    public Brush TextBrush => Brushes2.Get(Running ? "RunCardTextBrush" : "TextBrush");
    public Brush MutedBrush => Brushes2.Get(Running ? "RunCardMutedBrush" : "MutedBrush");
    /// <summary>▶ 能不能按：任務看能不能計時，會議看現在能不能加入。</summary>
    public bool CanStart => Item.IsMeeting ? Item.CanJoin(Now) : Item.CanTime;

    public Brush ActionBrush => Running ? Brushes2.Get("ErrorBrush") : Brushes2.Get(CanStart ? "RunBrush" : "MutedBrush");
    public string ActionGlyph => _busy ? "…" : Running ? "❚❚" : "▶";

    public string ActionTip => Item.IsMeeting
        ? Running ? "離開會議" : Item.Cancelled ? "會議已取消" : Item.Ended(Now) ? "會議已結束" : CanStart ? "加入會議並計時" : "開會前 10 分鐘起可以加入"
        : Running ? "暫停" : "切換到這個任務（暫停其他的）";

    public bool CanAct => !_busy && (Running || CanStart);

    public bool Busy
    {
        get => _busy;
        set
        {
            _busy = value;
            PropertyChanged?.Invoke(this, new(nameof(ActionGlyph)));
            PropertyChanged?.Invoke(this, new(nameof(CanAct)));
        }
    }

    public string Elapsed
    {
        get => _elapsed;
        set
        {
            if (_elapsed == value)
                return;
            _elapsed = value;
            PropertyChanged?.Invoke(this, new(nameof(Elapsed)));
        }
    }

    public event PropertyChangedEventHandler? PropertyChanged;
}

/// <summary>面板的釘選托盤（▶ 一鍵切換、Alt＋數字）、待我審核卡片、任務分頁的「只顯示沒有子任務的」。</summary>
public partial class PanelWindow
{
    readonly List<PinRowVm> _pins = [];

    void RenderPins()
    {
        var running = _engine.RunningKeys();
        _pins.Clear();
        var number = 1;
        foreach (var p in _engine.Pins)
            _pins.Add(new PinRowVm { Item = p, Number = number++, Running = running.Contains(p.Key) });
        PinList.ItemsSource = _pins.ToList();
        PinSection.Visibility = _pins.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        UpdatePinElapsed(DateTime.Now);
    }

    void UpdatePinElapsed(DateTime now)
    {
        foreach (var vm in _pins)
        {
            // 今天的計時列表有這個任務就自己算（計時中會跳秒），沒有就用托盤給的今天累積
            var minutes = WorkMath.TaskMinutes(_engine.Entries, vm.Key, now.Date, now);
            if (minutes <= 0)
                minutes = vm.Item.TodayMinutes;
            // 還沒開始的會議顯示開始時間
            if (minutes <= 0 && vm.Item is { IsMeeting: true, StartAt: { } start } && start > now)
                vm.Elapsed = start.LocalDateTime.Date == now.Date ? $"{start.ToLocalTime():HH:mm}" : $"{start.ToLocalTime():M/d HH:mm}";
            else
                vm.Elapsed = minutes <= 0 ? "" : vm.Running ? WorkMath.Clock(minutes) : WorkMath.HoursText(minutes);
        }
    }

    async void PinAction_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not PinRowVm vm)
            return;
        e.Handled = true;
        await RunPinAsync(vm);
    }

    async Task RunPinAsync(PinRowVm vm, bool keepOthers = false)
    {
        vm.Busy = true;
        if (vm.Item.IsMeeting)
        {
            var meeting = PinnedMeeting(vm.Item);
            if (vm.Running)
                await _engine.LeaveMeetingAsync(meeting);
            else if (vm.Item.CanJoin(DateTimeOffset.Now))
                await _engine.JoinMeetingAsync(meeting);
            vm.Busy = false;
            return;
        }
        if (vm.Running && !keepOthers)
            await _engine.PauseTaskAsync(vm.Key);
        else
            await _engine.SwitchTaskAsync(vm.Key, keepOthers);
        vm.Busy = false;
    }

    /// <summary>托盤上的會議 → 加入、離開用的會議資料（今天的會議清單裡有就用那份）。</summary>
    MeetingItem PinnedMeeting(PinItem pin) =>
        _engine.Meetings.FirstOrDefault(m => string.Equals(m.Key, pin.Key, StringComparison.OrdinalIgnoreCase))
        ?? new MeetingItem(pin.Key, pin.Key, pin.Title, pin.Project, pin.StartAt ?? DateTimeOffset.Now, pin.EndAt ?? DateTimeOffset.Now.AddHours(1),
            null, null);

    void Pin_Click(object sender, MouseButtonEventArgs e)
    {
        for (var d = e.OriginalSource as DependencyObject; d is not null && d != sender; d = VisualTreeHelper.GetParent(d))
            if (d is Button)
                return;
        if ((sender as FrameworkElement)?.Tag is PinRowVm vm)
            RequestWorkspace(vm.Item.IsMeeting ? "meetings" : "tasks", vm.Key);
    }

    void Pin_RightClick(object sender, MouseButtonEventArgs e)
    {
        if (sender is not FrameworkElement { Tag: PinRowVm vm } element)
            return;
        e.Handled = true;
        var menu = new ContextMenu();
        void Add(string header, Action action, bool enabled = true)
        {
            var item = new MenuItem { Header = header, IsEnabled = enabled };
            item.Click += (_, _) => action();
            menu.Items.Add(item);
        }
        if (vm.Item.IsMeeting)
        {
            if (vm.Running)
                Add("⏏  離開會議", () => _ = RunPinAsync(vm));
            else
                Add("▶  加入會議並計時", () => _ = RunPinAsync(vm), vm.CanStart);
            Add("☰  查看詳細", () => RequestWorkspace("meetings", vm.Key));
            Add("↗  在網頁開啟", () => Web.Open(_engine.MeetingUrl(PinnedMeeting(vm.Item).Id)));
        }
        else
        {
            if (vm.Running)
                Add("❚❚  暫停", () => _ = RunPinAsync(vm));
            else
            {
                Add("▶  切換到這個（暫停其他的）", () => _ = RunPinAsync(vm), vm.Item.CanTime);
                Add("▶  同時計時（不暫停其他的）", () => _ = RunPinAsync(vm, keepOthers: true), vm.Item.CanTime);
            }
            Add("☰  查看詳細", () => RequestWorkspace("tasks", vm.Key));
            Add("↗  在網頁開啟", () => Web.Open(_engine.TaskUrl(vm.Key)));
        }
        menu.Items.Add(new Separator { Margin = new Thickness(4), Background = Brushes2.Get("LineBrush") });
        Add("📌  取消釘選", () => _ = _engine.UnpinTaskAsync(vm.Key));
        OpenMenu(menu, MenuAnchor(element), "pin:" + vm.Key);
    }

    /// <summary>Alt＋1～9：切換到托盤上第幾個任務（跟網頁一樣）。</summary>
    bool HandlePinHotkey(KeyEventArgs e)
    {
        var key = e.Key == Key.System ? e.SystemKey : e.Key;
        if (Keyboard.Modifiers != ModifierKeys.Alt || key is < Key.D1 or > Key.D9)
            return false;
        var index = key - Key.D1;
        if (index >= _pins.Count)
            return false;
        var vm = _pins[index];
        if (!vm.Running && vm.CanStart)
            _ = RunPinAsync(vm);
        return true;
    }

    // ───────────── 待我審核 ─────────────

    void RenderReviews()
    {
        var pending = _engine.PendingReviews;
        ReviewCard.Visibility = pending.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        var projects = pending.Select(c => c.Project).Distinct().Count();
        ReviewText.Text = $"待我審核 {pending.Count} 筆" + (projects > 1 ? $"（{projects} 個專案）" : "");
        RenderTestCard();
    }

    void ReviewCard_Click(object sender, MouseButtonEventArgs e) => RequestWorkspace("reviews");

    /// <summary>今天分頁的測試卡片：我要修的測試結果（🔥 標第 3 輪起的）、要測的測試任務。</summary>
    void RenderTestCard()
    {
        var fixes = _engine.MyFixes;
        var tests = _engine.MyTests;
        var hot = fixes.Count(t => t.Test.Hot);
        var parts = new List<string>();
        if (fixes.Count > 0)
            parts.Add($"🐞 要修 {fixes.Count}" + (hot > 0 ? $"（🔥{hot}）" : ""));
        if (tests.Count > 0)
            parts.Add($"🧪 要測 {tests.Count}");
        TestCard.Visibility = parts.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        TestText.Text = string.Join("　", parts);
        _testCardTarget = fixes.Count > 0 ? "fix" : "test";
    }

    string _testCardTarget = "fix";

    void TestCard_Click(object sender, MouseButtonEventArgs e) => RequestWorkspace("tests", _testCardTarget);

    // ───────────── 任務分頁：只顯示沒有子任務的 ─────────────

    void PanelLeafOnly_Click(object sender, RoutedEventArgs e)
    {
        _engine.Config.TaskLeafOnly = PanelLeafOnlyBox.IsChecked == true;
        _engine.Config.Save();
        RenderTasks();
    }
}
