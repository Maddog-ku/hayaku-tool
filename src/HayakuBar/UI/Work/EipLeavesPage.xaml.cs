using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Eip;
using HayakuBar.Infra;

namespace HayakuBar.UI.Work;

public sealed class EipLeaveRowVm
{
    public required EipLeaveRecord Record { get; init; }
    public required bool Synced { get; init; }

    EipLeave L => Record.Leave;

    /// <summary>特休、事假…；不是一般請假（銷假、補假）時加註。</summary>
    public string Title => (L.Type.Length > 0 ? L.Type : L.Kind) + (L.Kind is "請假" or "" ? "" : $"（{L.Kind}）");

    public string Period
    {
        get
        {
            var start = L.Start.ToString("M/d（ddd）HH:mm", Inputs.Zh);
            var end = L.End.Date == L.Start.Date ? L.End.ToString("HH:mm") : L.End.ToString("M/d（ddd）HH:mm", Inputs.Zh);
            return $"{start} – {end}" + (L.Subtotal.Length > 0 ? $"　{L.Subtotal}" : "");
        }
    }

    public string Detail => string.Join(" · ", new[] { $"單號 {L.No}", L.Reason }.Where(s => s.Length > 0));
    public string State => Record.State;
    public Brush StateBrush => Brushes2.Get(Record.State switch { "核准" => "RunBrush", "駁回" => "ErrorBrush", _ => "WarnBrush" });
    public Visibility SyncedVisibility => Synced ? Visibility.Visible : Visibility.Collapsed;
}

/// <summary>EIP 請假紀錄：直接讀 EIP 的假單，可以切換月份，用來確認同步讀到的資料。</summary>
public partial class EipLeavesPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    DateOnly _month = new(DateTime.Today.Year, DateTime.Today.Month, 1);
    int _version;

    public EipLeavesPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
    }

    public Task ActivateAsync(object? arg) => LoadAsync(reload: false);

    async Task LoadAsync(bool reload)
    {
        var version = ++_version;
        var month = _month;
        MonthText.Text = month.ToString("yyyy 年 M 月");
        ThisMonthButton.Visibility = month == new DateOnly(DateTime.Today.Year, DateTime.Today.Month, 1) ? Visibility.Collapsed : Visibility.Visible;
        ReloadButton.IsEnabled = false;
        Inputs.Show(ListStatus, null);
        EmptyText.Text = "讀取中…";
        EmptyText.Visibility = Visibility.Visible;
        LeaveList.ItemsSource = null;
        SummaryText.Text = "";
        try
        {
            var year = await _engine.LoadEipYearAsync(month.Year, reload);
            if (version != _version)
                return;
            var from = month.ToDateTime(TimeOnly.MinValue);
            var to = month.AddMonths(1).ToDateTime(TimeOnly.MinValue);
            var rows = year
                .Where(r => r.Leave.Start < to && r.Leave.End > from)
                .OrderBy(r => r.Leave.Start)
                .Select(r => new EipLeaveRowVm { Record = r, Synced = _engine.IsEipSynced(r.Leave.No) })
                .ToList();
            LeaveList.ItemsSource = rows;
            EmptyText.Text = "這個月 EIP 上沒有你的假單";
            EmptyText.Visibility = rows.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            SummaryText.Text = $"這個月 {rows.Count} 筆 · {month.Year} 年共 {year.Count} 筆";
        }
        catch (Exception ex)
        {
            Log.Error("讀取 EIP 假單失敗", ex);
            if (version != _version)
                return;
            EmptyText.Visibility = Visibility.Collapsed;
            Inputs.Show(ListStatus, ex is EipLoginException ? ex.Message : $"讀取失敗：{ex.Message}");
        }
        finally
        {
            if (version == _version)
                ReloadButton.IsEnabled = true;
        }
    }

    async void Prev_Click(object sender, RoutedEventArgs e)
    {
        _month = _month.AddMonths(-1);
        await LoadAsync(reload: false);
    }

    async void Next_Click(object sender, RoutedEventArgs e)
    {
        _month = _month.AddMonths(1);
        await LoadAsync(reload: false);
    }

    async void ThisMonth_Click(object sender, RoutedEventArgs e)
    {
        _month = new DateOnly(DateTime.Today.Year, DateTime.Today.Month, 1);
        await LoadAsync(reload: false);
    }

    async void Reload_Click(object sender, RoutedEventArgs e) => await LoadAsync(reload: true);
}
