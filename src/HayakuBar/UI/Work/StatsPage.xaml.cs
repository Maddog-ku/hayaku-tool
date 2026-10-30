using System.Globalization;
using System.IO;
using System.Text;
using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 統計（跟網頁統計頁同一套數字）：依專案或依人、期間（本週…今年或自訂）、各專案／各人的工時長條與比例、明細表，可以匯出 CSV。
/// 打開這頁或換條件才讀，一次一個請求。
/// </summary>
public partial class StatsPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    string _perspective = "project";
    string _preset = "thisMonth";
    int _version;
    StatsReport? _report;
    bool _ready; // 建構時勾預設選項不要觸發讀取，打開頁面（ActivateAsync）才讀

    public StatsPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        ByProjectRadio.IsChecked = true;
        DefaultPreset.IsChecked = true;
        var today = DateOnly.FromDateTime(DateTime.Today);
        CustomRange.SetRange(today.AddDays(1 - today.Day), today);
        _ready = true;
    }

    public Task ActivateAsync(object? arg) => LoadAsync();

    async void Perspective_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string perspective } || !_ready)
            return;
        _perspective = perspective;
        await LoadAsync();
    }

    async void Preset_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is not RadioButton { Tag: string preset } || !_ready)
            return;
        _preset = preset;
        CustomRange.Visibility = preset == "custom" ? Visibility.Visible : Visibility.Collapsed;
        await LoadAsync();
    }

    async void CustomRange_Applied() => await LoadAsync();

    /// <summary>區間的粒度：一個月內一天一欄，四個月內一週一欄，再長一個月一欄（CSV 才不會太寬）。</summary>
    internal static string GranularityFor(string preset, DateOnly? from, DateOnly? to) => preset switch
    {
        "thisYear" => "month",
        "custom" when from is { } f && to is { } t => (t.DayNumber - f.DayNumber) switch
        {
            <= 31 => "day",
            <= 120 => "week",
            _ => "month",
        },
        _ => "day",
    };

    async Task LoadAsync()
    {
        var version = ++_version;
        Inputs.Show(StatusText, null);
        ActualText.Text = SecondText.Text = DoneText.Text = OverdueText.Text = "…";
        ExportButton.IsEnabled = false;
        DateOnly? from = null, to = null;
        if (_preset == "custom")
        {
            (from, to) = (CustomRange.Start, CustomRange.End ?? CustomRange.Start);
            if (from is null)
            {
                Inputs.Show(StatusText, "請選自訂的期間");
                return;
            }
        }
        try
        {
            var report = await _engine.Api.StatsAsync(_perspective, _preset == "custom" ? null : _preset, from, to,
                GranularityFor(_preset, from, to), CancellationToken.None);
            if (version != _version)
                return;
            _report = report;
            Render(report);
            ExportButton.IsEnabled = report.Rows.Count > 0;
        }
        catch (Exception ex)
        {
            Log.Error("讀取統計失敗", ex);
            if (version == _version)
                Inputs.Show(StatusText, $"讀取失敗：{ex.Message}");
        }
    }

    void Render(StatsReport r)
    {
        RangeText.Text = r.From is { } f && r.To is { } t
            ? $"{f:yyyy/MM/dd} – {t:yyyy/MM/dd}" + (r.ByProject ? "　只算已核准的任務；超出權限的專案、人會自動略過" : "　只算已核准的任務；只看得到自己或有權限的人")
            : "";
        ActualText.Text = WorkMath.HoursText(r.ActualMinutes);
        DoneText.Text = r.Done.ToString();
        OverdueText.Text = r.Overdue.ToString();
        OverdueText.Foreground = Brushes2.Get(r.Overdue > 0 ? "ErrorBrush" : "TextBrush");
        if (r.ByProject)
        {
            ActualDesc.Text = "專案裡所有人的計時";
            SecondLabel.Text = "預計工時";
            SecondText.Text = WorkMath.HoursText(r.EstimatedMinutes);
            SecondDesc.Text = "這段期間任務的預計工時";
            DoneDesc.Text = r.TaskTotal > 0 ? $"共 {r.TaskTotal} 個任務，完成率 {Percent(r.Done, r.TaskTotal)}" : "沒有任務";
            OverdueDesc.Text = "還沒完成、預計結束已過";
            BarsTitle.Text = "各專案工時";
            BarNote.Text = "各專案照實計算；同一個人同時計時兩個專案時兩邊都算。";
        }
        else
        {
            ActualDesc.Text = r.Rows.Count > 1 ? $"{r.Rows.Count} 人合計，每人同時計時只算一次" : "同時計時只算一次";
            SecondLabel.Text = "應出勤";
            SecondText.Text = WorkMath.HoursText(r.ExpectedMinutes);
            SecondDesc.Text = r.Rows.Count == 1 ? $"{r.Rows[0].Workdays} 個工作天，有計時 {r.Rows[0].ActiveDays} 天" : "上班時段扣掉假日與請假";
            DoneDesc.Text = $"指派 {r.TaskTotal} 個";
            OverdueDesc.Text = "指派了、還沒完成";
            BarsTitle.Text = "各人工時";
            BarNote.Text = "每個人的實際工時，同時計時只算一次。";
        }

        var total = Math.Max(1, r.Rows.Sum(x => x.ActualMinutes));
        var max = Math.Max(1, r.Rows.Select(x => x.ActualMinutes).DefaultIfEmpty(0).Max());
        var bars = r.Rows.Where(x => x.ActualMinutes > 0).ToList();
        BarList.ItemsSource = bars.Select(x => new
        {
            x.Name,
            Hours = WorkMath.HoursMinutes(x.ActualMinutes),
            Share = Percent(x.ActualMinutes, total),
            Ratio = (double)x.ActualMinutes / max,
        }).ToList();
        BarEmpty.Visibility = bars.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        RenderTable(r);
    }

    void RenderTable(StatsReport r)
    {
        DetailTable.Children.Clear();
        DetailTable.RowDefinitions.Clear();
        DetailTable.ColumnDefinitions.Clear();
        var table = Table(r);
        DetailEmpty.Visibility = table.Count <= 1 ? Visibility.Visible : Visibility.Collapsed;
        if (table.Count <= 1)
            return;
        for (var c = 0; c < table[0].Count; c++)
            DetailTable.ColumnDefinitions.Add(new ColumnDefinition { Width = c == 0 ? new GridLength(1, GridUnitType.Star) : GridLength.Auto });
        for (var row = 0; row < table.Count; row++)
        {
            DetailTable.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            for (var c = 0; c < table[row].Count; c++)
            {
                var cell = new TextBlock
                {
                    Text = table[row][c],
                    Margin = new Thickness(c == 0 ? 0 : 16, 0, 0, 6),
                    TextAlignment = c == 0 ? TextAlignment.Left : TextAlignment.Right,
                    TextTrimming = TextTrimming.CharacterEllipsis,
                    Foreground = Brushes2.Get(row == 0 ? "MutedBrush" : "TextBrush"),
                    FontSize = row == 0 ? 12 : 13,
                };
                if (c > 0 && row > 0)
                    cell.FontFamily = (System.Windows.Media.FontFamily)FindResource("MonoFont");
                Grid.SetRow(cell, row);
                Grid.SetColumn(cell, c);
                DetailTable.Children.Add(cell);
            }
        }
    }

    static string Percent(double part, double whole) =>
        whole <= 0 ? "—" : (part / whole).ToString("0%", CultureInfo.InvariantCulture);

    static string Hours(int minutes) => (minutes / 60.0).ToString("0.##", CultureInfo.InvariantCulture);

    /// <summary>明細表（第一列是標題）：畫面上的表與 CSV 共用。工時用小時（小數）。</summary>
    internal static List<List<string>> Table(StatsReport r)
    {
        var rows = new List<List<string>>();
        if (r.ByProject)
        {
            rows.Add(["專案", "實際（小時）", "預計（小時）", "累計（小時）", "完成／任務", "完成率", "逾期"]);
            rows.AddRange(r.Rows.Select(x => new List<string>
            {
                x.Name, Hours(x.ActualMinutes), Hours(x.EstimatedMinutes), Hours(x.TotalLoggedMinutes), $"{x.Done}/{x.Assigned}",
                x.CompletionRate is { } rate ? rate.ToString("0%", CultureInfo.InvariantCulture) : Percent(x.Done, x.Assigned),
                x.Overdue.ToString(CultureInfo.InvariantCulture),
            }));
        }
        else
        {
            rows.Add(["人", "實際（小時）", "應出勤（小時）", "達成", "工作天", "有計時天數", "完成／指派", "逾期"]);
            rows.AddRange(r.Rows.Select(x => new List<string>
            {
                x.Name, Hours(x.ActualMinutes), Hours(x.ExpectedMinutes), Percent(x.ActualMinutes, x.ExpectedMinutes),
                x.Workdays.ToString(CultureInfo.InvariantCulture), x.ActiveDays.ToString(CultureInfo.InvariantCulture), $"{x.Done}/{x.Assigned}",
                x.Overdue.ToString(CultureInfo.InvariantCulture),
            }));
        }
        return rows;
    }

    /// <summary>
    /// CSV：明細表，空一行，再來每段期間（日／週／月）的工時（列是專案或人，欄是期間，單位小時）。
    /// 欄位有逗號、引號、換行時加引號。
    /// </summary>
    internal static string Csv(StatsReport r)
    {
        var sb = new StringBuilder();
        void Line(IEnumerable<string> cells) => sb.Append(string.Join(",", cells.Select(Escape))).Append("\r\n");
        foreach (var row in Table(r))
            Line(row);
        if (r.Buckets.Count > 0 && r.Cells.Count > 0)
        {
            sb.Append("\r\n");
            var unit = r.Granularity switch { "week" => "每週（起日）", "month" => "每月", _ => "每天" };
            Line([$"{(r.ByProject ? "專案" : "人")}／{unit}（小時）", .. r.Buckets]);
            var byKey = r.Cells.GroupBy(c => c.Key).ToDictionary(g => g.Key, g => g.ToDictionary(c => c.Bucket, c => c.Minutes));
            foreach (var row in r.Rows.Where(x => byKey.ContainsKey(x.Key)))
                Line([row.Name, .. r.Buckets.Select(b => byKey[row.Key].TryGetValue(b, out var m) ? Hours(m) : "0")]);
        }
        return sb.ToString();

        static string Escape(string s) =>
            s.IndexOfAny([',', '"', '\n', '\r']) >= 0 ? "\"" + s.Replace("\"", "\"\"") + "\"" : s;
    }

    void Export_Click(object sender, RoutedEventArgs e)
    {
        if (_report is not { } r)
            return;
        var range = r.From is { } f && r.To is { } t ? $"_{f:yyyy-MM-dd}_{t:yyyy-MM-dd}" : "";
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            Title = "匯出統計",
            FileName = $"Hayaku統計_{(r.ByProject ? "依專案" : "依人")}{range}.csv",
            Filter = "CSV 檔案 (*.csv)|*.csv",
            DefaultExt = ".csv",
        };
        if (dialog.ShowDialog(Window.GetWindow(this)) != true)
            return;
        try
        {
            // 加 BOM，Excel 直接打開中文才不會變亂碼
            File.WriteAllText(dialog.FileName, Csv(r), new UTF8Encoding(encoderShouldEmitUTF8Identifier: true));
            Inputs.Show(StatusText, $"已匯出到 {dialog.FileName}", error: false);
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
        {
            Log.Error("匯出統計失敗", ex);
            Inputs.Show(StatusText, $"存檔失敗：{ex.Message}");
        }
    }

    void Web_Click(object sender, RoutedEventArgs e) => Web.Open(_engine.WebUrl("/stats"));
}
