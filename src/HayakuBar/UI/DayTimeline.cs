using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// 一天的時間軸：底色是班表時段，綠色是有計時的時段（並行只算一次），班表內沒計到的空檔標紅，
/// 今天會畫出「現在」的位置。滑鼠移上去會顯示那個時間點在跑哪些任務。
/// </summary>
public sealed class DayTimeline : FrameworkElement
{
    const double BarHeight = 8;
    const double LabelGap = 3;

    DateTime _day = DateTime.Today;
    DateTime _now = DateTime.Now;
    IReadOnlyList<WorkSegment> _segments = [];
    IReadOnlyList<TimeEntry> _entries = [];
    List<(DateTime Start, DateTime End)> _covered = [];
    List<(DateTime Start, DateTime End)> _gaps = [];
    DateTime _from;
    DateTime _to;

    public DayTimeline()
    {
        ToolTipService.SetInitialShowDelay(this, 0);
        ToolTipService.SetShowDuration(this, 60000);
        ToolTip = "";
        Loaded += (_, _) => ThemeManager.Changed += InvalidateVisual;
        Unloaded += (_, _) => ThemeManager.Changed -= InvalidateVisual;
    }

    /// <summary>畫出不足班表的空檔（工時紀錄、懸停摘要用）。</summary>
    public bool ShowGaps { get; set; } = true;

    public void SetData(DateTime day, IReadOnlyList<WorkSegment>? segments, IReadOnlyList<TimeEntry> entries, DateTime now)
    {
        _day = day.Date;
        _now = now;
        _segments = segments ?? [];
        _entries = entries;
        _covered = WorkMath.Covered(entries, _day, now);
        _gaps = ShowGaps ? WorkMath.Gaps(_segments, entries, _day, now) : [];

        // 範圍：班表與計時段的最早～最晚，取整點；什麼都沒有時顯示 9–18
        var points = _segments.SelectMany(s => new[] { _day + s.Start.ToTimeSpan(), _day + s.End.ToTimeSpan() })
            .Concat(_covered.SelectMany(c => new[] { c.Start, c.End }))
            .ToList();
        if (_day == now.Date)
            points.Add(now);
        var from = points.Count > 0 ? points.Min() : _day.AddHours(9);
        var to = points.Count > 0 ? points.Max() : _day.AddHours(18);
        _from = _day.AddHours(Math.Floor((from - _day).TotalHours));
        _to = _day.AddHours(Math.Min(24, Math.Ceiling((to - _day).TotalHours)));
        if (_to - _from < TimeSpan.FromHours(4))
            _to = _from.AddHours(4);
        InvalidateVisual();
    }

    protected override Size MeasureOverride(Size availableSize) =>
        new(Math.Min(200, availableSize.Width), BarHeight + LabelGap + 12); // 寬度跟著容器撐開

    double X(DateTime t) => (t - _from).TotalMinutes / Math.Max(1, (_to - _from).TotalMinutes) * ActualWidth;

    static Brush Res(string key) => (Brush)Application.Current.Resources[key];

    protected override void OnRender(DrawingContext dc)
    {
        var w = ActualWidth;
        if (w <= 0)
            return;
        var bar = new Rect(0, 0, w, BarHeight);
        dc.DrawRoundedRectangle(Res("SubtleBrush"), null, bar, 4, 4);

        var segBrush = Res("LineBrush");
        foreach (var s in _segments)
            Fill(dc, segBrush, _day + s.Start.ToTimeSpan(), _day + s.End.ToTimeSpan(), 1);

        var gapBrush = Res("ErrorBrush").Clone();
        gapBrush.Opacity = 0.35;
        foreach (var (s, e) in _gaps)
            Fill(dc, gapBrush, s, e, 1);

        var runBrush = Res("RunBrush");
        foreach (var (s, e) in _covered)
            Fill(dc, runBrush, s, e, 0);

        if (_day == _now.Date && _now >= _from && _now <= _to)
        {
            var x = Math.Round(X(_now)) + 0.5;
            dc.DrawLine(new Pen(Res("AccentBrush"), 2), new Point(x, -2), new Point(x, BarHeight + 2));
        }

        // 整點刻度：寬度夠就每小時，不夠就每 2、3 小時
        var hours = (int)Math.Round((_to - _from).TotalHours);
        var step = w / Math.Max(1, hours) >= 26 ? 1 : w / Math.Max(1, hours) >= 14 ? 2 : 3;
        var dpi = VisualTreeHelper.GetDpi(this).PixelsPerDip;
        var muted = Res("MutedBrush");
        for (var h = 0; h <= hours; h += step)
        {
            var t = _from.AddHours(h);
            var text = new FormattedText(t.Hour == 0 && t > _day ? "24" : t.Hour.ToString(CultureInfo.InvariantCulture),
                CultureInfo.InvariantCulture, FlowDirection.LeftToRight, new Typeface("Segoe UI"), 9.5, muted, dpi);
            var x = Math.Clamp(X(t) - text.Width / 2, 0, w - text.Width);
            dc.DrawText(text, new Point(x, BarHeight + LabelGap));
        }
    }

    void Fill(DrawingContext dc, Brush brush, DateTime start, DateTime end, double inset)
    {
        if (end <= _from || start >= _to)
            return;
        var x1 = X(start < _from ? _from : start);
        var x2 = X(end > _to ? _to : end);
        if (x2 - x1 < 1)
            x2 = x1 + 1;
        dc.DrawRoundedRectangle(brush, null, new Rect(x1, inset, x2 - x1, BarHeight - inset * 2), 3, 3);
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
        var x = e.GetPosition(this).X;
        var t = _from + TimeSpan.FromMinutes((_to - _from).TotalMinutes * Math.Clamp(x / Math.Max(1, ActualWidth), 0, 1));
        ToolTip = Describe(t);
    }

    string Describe(DateTime t)
    {
        var tasks = _entries
            .Where(e => WorkMath.Intervals([e], _day, _now).Any(i => i.Start <= t && t < i.End))
            .Select(e => e.Task)
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToList();
        var head = t.ToString("HH:mm", CultureInfo.InvariantCulture);
        if (tasks.Count > 0)
            return $"{head}  計時中：{string.Join("、", tasks)}";
        if (_gaps.Any(g => g.Start <= t && t < g.End))
            return $"{head}  班表時段，沒有計時";
        return $"{head}  沒有計時";
    }
}
