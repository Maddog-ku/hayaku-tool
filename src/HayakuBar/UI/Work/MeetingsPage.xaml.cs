using System.ComponentModel;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

public sealed class AttendeeVm : INotifyPropertyChanged
{
    bool _selected;
    public required string Username { get; init; }
    public required string Name { get; init; }

    public bool Selected
    {
        get => _selected;
        set
        {
            _selected = value;
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(Selected)));
        }
    }

    public event PropertyChangedEventHandler? PropertyChanged;
}

public sealed class MeetingListVm
{
    public required MeetingItem Item { get; init; }
    public required bool Past { get; init; }

    public string Title => Item.Title;
    public string Day => Inputs.ShortDate(DateOnly.FromDateTime(Item.StartAt.LocalDateTime));
    public string Time => $"{Item.StartAt.ToLocalTime():HH:mm}–{Item.EndAt.ToLocalTime():HH:mm}";

    public string Detail => string.Join(" · ", new[] { Item.Project, Item.Location, Item.MyStatus }
        .Where(s => !string.IsNullOrWhiteSpace(s)));

    bool Ongoing => DateTimeOffset.Now >= Item.StartAt && DateTimeOffset.Now < Item.EndAt;

    public Brush TimeBrush => Brushes2.Get(Ongoing ? "AccentBrush" : Past ? "MutedBrush" : "TextBrush");

    public Visibility JoinVisibility =>
        !Past && !Item.InMeeting && DateTimeOffset.Now >= Item.StartAt.AddMinutes(-15) && DateTimeOffset.Now < Item.EndAt
            ? Visibility.Visible : Visibility.Collapsed;

    public Visibility LeaveVisibility => Item.InMeeting ? Visibility.Visible : Visibility.Collapsed;
    public Visibility CancelVisibility => !Past && DateTimeOffset.Now < Item.StartAt ? Visibility.Visible : Visibility.Collapsed;
}

/// <summary>會議：接下來／參加過的、加入並計時、離開、取消、新增會議。</summary>
public partial class MeetingsPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    bool _past;
    int _version;

    public MeetingsPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
    }

    public async Task ActivateAsync(object? arg)
    {
        if (arg is "new")
            await ShowFormAsync();
        await LoadAsync();
    }

    async Task LoadAsync()
    {
        var version = ++_version;
        EmptyText.Text = "讀取中…";
        EmptyText.Visibility = Visibility.Visible;
        Inputs.Show(ListStatus, null);
        try
        {
            var items = _past
                ? await _engine.Api.PastMeetingsAsync(CancellationToken.None)
                : await _engine.Api.UpcomingMeetingsAsync(CancellationToken.None);
            if (version != _version)
                return;
            var rows = items.Select(m => new MeetingListVm { Item = m, Past = _past }).ToList();
            MeetingList.ItemsSource = rows;
            EmptyText.Text = _past ? "還沒有參加過的會議" : "接下來沒有會議";
            EmptyText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        }
        catch (Exception ex)
        {
            Log.Error("讀取會議失敗", ex);
            if (version == _version)
            {
                EmptyText.Visibility = Visibility.Collapsed;
                Inputs.Show(ListStatus, $"讀取失敗：{ex.Message}");
            }
        }
    }

    async void Tab_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string tab } || MeetingList is null)
            return;
        _past = tab == "past";
        await LoadAsync();
    }

    async void Join_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not MeetingListVm vm)
            return;
        ((Button)sender).IsEnabled = false;
        await _engine.JoinMeetingAsync(vm.Item);
        Inputs.Show(ListStatus, _engine.Error);
        await LoadAsync();
    }

    async void Leave_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not MeetingListVm vm)
            return;
        if (await Inputs.Run((Button)sender, ListStatus, () => _engine.Api.LeaveMeetingAsync(vm.Item.Key, CancellationToken.None),
                "已離開會議，停止計時"))
        {
            await _engine.AfterExternalChangeAsync();
            await LoadAsync();
        }
    }

    void Open_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is MeetingListVm vm)
            Web.Open(_engine.MeetingUrl(vm.Item.Id));
    }

    async void CancelMeeting_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not MeetingListVm vm)
            return;
        var ok = MessageBox.Show(Window.GetWindow(this)!, $"取消會議「{vm.Title}」？會通知所有參加者。", "取消會議",
            MessageBoxButton.OKCancel, MessageBoxImage.Warning);
        if (ok != MessageBoxResult.OK)
            return;
        if (await Inputs.Run((Button)sender, ListStatus, () => _engine.Api.CancelMeetingAsync(vm.Item.Key, CancellationToken.None),
                "已取消會議"))
        {
            await _engine.AfterExternalChangeAsync();
            await LoadAsync();
        }
    }

    // ───────────── 新增 ─────────────

    async void New_Click(object sender, RoutedEventArgs e) => await ShowFormAsync();

    async Task ShowFormAsync()
    {
        FormCard.Visibility = Visibility.Visible;
        NewButton.Visibility = Visibility.Collapsed;
        Inputs.Show(FormStatus, null);
        var start = DateTime.Now.AddHours(1);
        start = new DateTime(start.Year, start.Month, start.Day, start.Hour, 0, 0);
        DateInput.Text = start.Date == DateTime.Today ? "今天" : start.ToString("M/d");
        StartInput.Text = start.ToString("HH:mm");
        EndInput.Text = start.AddHours(1).ToString("HH:mm");
        TitleInput.Focus();
        if (ProjectBox.ItemsSource is null)
        {
            try
            {
                var projects = await _engine.Api.ListProjectsAsync(CancellationToken.None);
                ProjectBox.ItemsSource = projects;
                var preferred = _engine.RunningKeys().Select(KeyFinder.ProjectOf).FirstOrDefault();
                ProjectBox.SelectedItem = projects.FirstOrDefault(p => p.Key == preferred) ?? projects.FirstOrDefault();
            }
            catch (Exception ex)
            {
                Inputs.Show(FormStatus, $"讀不到專案清單：{ex.Message}");
            }
        }
    }

    async void ProjectBox_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (ProjectBox.SelectedItem is not ProjectInfo project)
            return;
        try
        {
            var detail = await _engine.GetProjectAsync(project.Key);
            var me = _engine.User?.Username;
            AttendeeList.ItemsSource = detail.Members
                .Select(m => new AttendeeVm { Username = m.Username, Name = m.DisplayName, Selected = m.Username == me })
                .ToList();
        }
        catch (Exception ex)
        {
            Inputs.Show(FormStatus, $"讀不到專案成員：{ex.Message}");
        }
    }

    void Cancel_Click(object sender, RoutedEventArgs e)
    {
        FormCard.Visibility = Visibility.Collapsed;
        NewButton.Visibility = Visibility.Visible;
    }

    async void Create_Click(object sender, RoutedEventArgs e)
    {
        if (ProjectBox.SelectedItem is not ProjectInfo project)
        {
            Inputs.Show(FormStatus, "請選專案");
            return;
        }
        var title = TitleInput.Text.Trim();
        if (title.Length == 0)
        {
            Inputs.Show(FormStatus, "請填標題");
            TitleInput.Focus();
            return;
        }
        if (Inputs.ParseDate(DateInput.Text) is not { } date)
        {
            Inputs.Show(FormStatus, "日期看不懂，請用 今天、明天、11/5 這類寫法");
            return;
        }
        if (Inputs.ParseTime(StartInput.Text) is not { } start || Inputs.ParseTime(EndInput.Text) is not { } end)
        {
            Inputs.Show(FormStatus, "開始、結束請用 HH:mm，例 14:00");
            return;
        }
        if (end <= start)
        {
            Inputs.Show(FormStatus, "結束要晚於開始（會議要在同一天）");
            return;
        }
        var attendees = (AttendeeList.ItemsSource as IEnumerable<AttendeeVm> ?? []).Where(a => a.Selected).Select(a => a.Username).ToList();
        if (attendees.Count == 0)
            attendees.Add("me");
        var args = new Dictionary<string, object?>
        {
            ["project"] = project.Key,
            ["title"] = title,
            ["startAt"] = Inputs.Iso(Inputs.Local(date, start)),
            ["endAt"] = Inputs.Iso(Inputs.Local(date, end)),
            ["attendees"] = attendees,
        };
        if (LocationInput.Text.Trim().Length > 0)
            args["location"] = LocationInput.Text.Trim();
        if (NoteInput.Text.Trim().Length > 0)
            args["note"] = NoteInput.Text.Trim();
        if (!await Inputs.Run(CreateButton, FormStatus, () => _engine.Api.CreateMeetingAsync(args, CancellationToken.None)))
            return;
        TitleInput.Text = LocationInput.Text = NoteInput.Text = "";
        FormCard.Visibility = Visibility.Collapsed;
        NewButton.Visibility = Visibility.Visible;
        Inputs.Show(ListStatus, $"已建立會議「{title}」", error: false);
        await _engine.AfterExternalChangeAsync();
        await LoadAsync();
    }
}
