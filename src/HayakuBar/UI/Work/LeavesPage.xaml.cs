using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

public sealed class LeaveRowVm
{
    public required LeaveItem Item { get; init; }
    public required bool Mine { get; init; }

    /// <summary>由 EIP 請假同步建立的。</summary>
    public bool FromEip { get; init; }
    public string Name => Item.User.DisplayName + (Mine ? "（我）" : "");
    public string Initial => Item.User.DisplayName.Length > 0 ? Item.User.DisplayName[..1] : "?";
    public string Period => Item.Period + (FromEip ? "　· 從 EIP 同步" : "");
    public string Lost => Item.LostText;
    public Visibility CancelVisibility => Mine && Item.EndAt > DateTimeOffset.Now ? Visibility.Visible : Visibility.Collapsed;
}

/// <summary>請假：列出期間內誰請假、登記自己的請假、取消。</summary>
public partial class LeavesPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    string _range = "2w";
    string _span = "DAY";
    int _version;

    public LeavesPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        var tomorrow = DateOnly.FromDateTime(DateTime.Today).AddDays(1);
        Dates.SetRange(tomorrow, tomorrow);
    }

    public Task ActivateAsync(object? arg) => LoadAsync();

    (DateOnly From, DateOnly To) Range()
    {
        var today = DateOnly.FromDateTime(DateTime.Today);
        var first = new DateOnly(today.Year, today.Month, 1);
        return _range switch
        {
            "month" => (first, first.AddMonths(1).AddDays(-1)),
            "next" => (first.AddMonths(1), first.AddMonths(2).AddDays(-1)),
            _ => (today, today.AddDays(13)),
        };
    }

    async Task LoadAsync()
    {
        var version = ++_version;
        EmptyText.Text = "讀取中…";
        EmptyText.Visibility = Visibility.Visible;
        Inputs.Show(ListStatus, null);
        var (from, to) = Range();
        try
        {
            var items = await _engine.Api.ListLeavesAsync(from, to, CancellationToken.None);
            if (version != _version)
                return;
            var me = _engine.User?.Username;
            var rows = items
                .Select(l => new LeaveRowVm { Item = l, Mine = l.User.Username == me, FromEip = _engine.EipSync.IsSynced(l.Id) })
                .Where(r => MineOnly.IsChecked != true || r.Mine)
                .ToList();
            LeaveList.ItemsSource = rows;
            EmptyText.Text = MineOnly.IsChecked == true ? "這段期間你沒有請假" : "這段期間沒有人請假";
            EmptyText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        }
        catch (Exception ex)
        {
            Log.Error("讀取請假失敗", ex);
            if (version == _version)
            {
                EmptyText.Visibility = Visibility.Collapsed;
                Inputs.Show(ListStatus, $"讀取失敗：{ex.Message}");
            }
        }
    }

    async void Range_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string range } || LeaveList is null)
            return;
        _range = range;
        await LoadAsync();
    }

    async void MineOnly_Click(object sender, RoutedEventArgs e) => await LoadAsync();

    void Span_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string span } || RangePanel is null)
            return;
        _span = span;
        Dates.Single = span != "DAY"; // 整天可以請一段；半天、指定時段只有一天
        RangePanel.Visibility = span == "RANGE" ? Visibility.Visible : Visibility.Collapsed;
        DateLabel.Text = span == "DAY" ? "第一天 ～ 最後一天（只請一天就點同一天）" : "日期";
    }

    async void Add_Click(object sender, RoutedEventArgs e)
    {
        if (Dates.Start is not { } date)
        {
            Inputs.Show(FormStatus, "請選日期");
            return;
        }
        var args = new Dictionary<string, object?> { ["span"] = _span, ["date"] = Inputs.DateText(date) };
        var summary = date.ToString("M/d（ddd）", Inputs.Zh);
        switch (_span)
        {
            case "DAY" when Dates.End is { } endDate && endDate > date:
                args["endDate"] = Inputs.DateText(endDate);
                summary += $" 到 {endDate.ToString("M/d（ddd）", Inputs.Zh)} 整天";
                break;
            case "DAY":
                summary += " 整天";
                break;
            case "HALF_AM":
                summary += " 上午";
                break;
            case "HALF_PM":
                summary += " 下午";
                break;
            case "RANGE":
                if (Inputs.ParseTime(StartInput.Text) is not { } start || Inputs.ParseTime(EndInput.Text) is not { } end || end <= start)
                {
                    Inputs.Show(FormStatus, "時段請用 HH:mm，而且結束要晚於開始");
                    return;
                }
                args["start"] = start.ToString("HH:mm");
                args["end"] = end.ToString("HH:mm");
                summary += $" {start:HH:mm}–{end:HH:mm}";
                break;
        }
        if (NoteInput.Text.Trim().Length > 0)
            args["note"] = NoteInput.Text.Trim();
        var ok = MessageBox.Show(Window.GetWindow(this)!, $"登記請假：{summary}？", "登記請假", MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok != MessageBoxResult.OK)
            return;
        if (!await Inputs.Run(AddButton, FormStatus, () => _engine.Api.AddLeaveAsync(args, CancellationToken.None), $"已登記：{summary}"))
            return;
        NoteInput.Text = StartInput.Text = EndInput.Text = "";
        await _engine.AfterExternalChangeAsync();
        await LoadAsync();
    }

    async void Cancel_Click(object sender, RoutedEventArgs e)
    {
        if ((sender as Button)?.Tag is not LeaveRowVm row)
            return;
        var ok = MessageBox.Show(Window.GetWindow(this)!, $"取消請假：{row.Period}？", "取消請假", MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok != MessageBoxResult.OK)
            return;
        if (await Inputs.Run((Button)sender, ListStatus, () => _engine.Api.CancelLeaveAsync(row.Item.Id, CancellationToken.None), "已取消請假"))
        {
            await _engine.AfterExternalChangeAsync();
            await LoadAsync();
        }
    }
}
