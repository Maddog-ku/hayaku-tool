namespace HayakuBar.Core;

/// <summary>
/// 我的任務的日曆、甘特圖（Hayaku 1.5.2）：拖拉改預計日期的計算與日曆排版。純邏輯，畫面在 TasksPage.Calendar／Gantt。
/// 只有預計開始或只有預計結束的任務當作一天；兩個都沒有的不畫在日期上（另外列出來，拖到日期上就設成那天）。
/// </summary>
public static class TaskSchedule
{
    /// <summary>拖整條（起訖一起平移）、拖開始那端、拖結束那端。</summary>
    public enum Drag { Move, Start, End }

    /// <summary>畫出來的範圍；結束早於開始時對調；都沒有回 null。</summary>
    public static (DateOnly Start, DateOnly End)? Span(DateOnly? start, DateOnly? end) => (start, end) switch
    {
        ({ } s, { } e) => s <= e ? (s, e) : (e, s),
        ({ } s, null) => (s, s),
        (null, { } e) => (e, e),
        _ => null,
    };

    /// <summary>
    /// 拖了 days 天之後的預計開始／結束。平移時沒填的欄位維持沒填；拖開始（結束）那端不會越過另一端。
    /// 只有結束的任務拖開始那端往前，會補上預計開始；兩個都沒有的不變（用 <see cref="Place"/>）。
    /// </summary>
    public static (DateOnly? Start, DateOnly? End) Apply(DateOnly? start, DateOnly? end, Drag mode, int days)
    {
        if (days == 0 || Span(start, end) is not { } span)
            return (start, end);
        switch (mode)
        {
            case Drag.Move:
                return (start?.AddDays(days), end?.AddDays(days));
            case Drag.Start:
            {
                var s = span.Start.AddDays(days);
                if (end is not null && s > span.End)
                    s = span.End;
                return start is null && s == span.Start ? (start, end) : (s, end);
            }
            default:
            {
                var e = span.End.AddDays(days);
                if (start is not null && e < span.Start)
                    e = span.Start;
                return end is null && e == span.End ? (start, end) : (start, e);
            }
        }
    }

    /// <summary>沒有預計日期的任務拖到日期上（或在甘特圖拉出一段）：開始、結束照拖的範圍。</summary>
    public static (DateOnly Start, DateOnly End) Place(DateOnly from, DateOnly to) => from <= to ? (from, to) : (to, from);

    /// <summary>要送 update_task 的欄位（plannedStart、plannedEnd，只放有變的）；拖拉不會把日期清空。</summary>
    public static Dictionary<string, DateOnly> Changes(DateOnly? oldStart, DateOnly? oldEnd, DateOnly? newStart, DateOnly? newEnd)
    {
        var fields = new Dictionary<string, DateOnly>();
        if (newStart is { } s && s != oldStart)
            fields["plannedStart"] = s;
        if (newEnd is { } e && e != oldEnd)
            fields["plannedEnd"] = e;
        return fields;
    }

    /// <summary>那一週的星期一（日曆、甘特圖都從星期一開始）。</summary>
    public static DateOnly WeekStart(DateOnly day) => day.AddDays(-(((int)day.DayOfWeek + 6) % 7));

    /// <summary>月曆要畫的範圍：第一格（1 號那週的星期一）與週數（4～6 週）。</summary>
    public static (DateOnly First, int Weeks) MonthGrid(int year, int month)
    {
        var first = new DateOnly(year, month, 1);
        var start = WeekStart(first);
        var last = first.AddMonths(1).AddDays(-1);
        return (start, (last.DayNumber - start.DayNumber) / 7 + 1);
    }

    /// <summary>日曆一週裡的一條：從第 Column 天（0＝星期一）起佔 Days 天，排在第 Lane 列；FromBefore／ToAfter＝前一週、下一週還有。</summary>
    public sealed record Segment<T>(T Item, int Column, int Days, int Lane, bool FromBefore, bool ToAfter);

    /// <summary>
    /// 一週裡的任務排成不重疊的列：先開始的在上面，同一天開始的長的在上面，其餘照傳進來的順序。
    /// </summary>
    public static List<Segment<T>> WeekLanes<T>(IEnumerable<(T Item, DateOnly Start, DateOnly End)> items, DateOnly weekStart)
    {
        var weekEnd = weekStart.AddDays(6);
        var laneEnds = new List<int>(); // 每一列最後佔到第幾天
        var result = new List<Segment<T>>();
        foreach (var (item, s, e) in items.Where(x => x.Start <= weekEnd && x.End >= weekStart)
                     .OrderBy(x => x.Start).ThenByDescending(x => x.End))
        {
            var from = Math.Max(0, s.DayNumber - weekStart.DayNumber);
            var to = Math.Min(6, e.DayNumber - weekStart.DayNumber);
            var lane = laneEnds.FindIndex(last => last < from);
            if (lane < 0)
            {
                lane = laneEnds.Count;
                laneEnds.Add(to);
            }
            else
                laneEnds[lane] = to;
            result.Add(new(item, from, to - from + 1, lane, s < weekStart, e > weekEnd));
        }
        return result;
    }

    /// <summary>「11/12」「11/12 – 11/15」；只有一端時標出另一端未指定。</summary>
    public static string RangeText(DateOnly? start, DateOnly? end) => (start, end) switch
    {
        ({ } s, { } e) when s == e => Md(s),
        ({ } s, { } e) => $"{Md(s)} – {Md(e)}",
        ({ } s, null) => $"{Md(s)} 開始（結束未指定）",
        (null, { } e) => $"{Md(e)} 結束（開始未指定）",
        _ => "沒有預計日期",
    };

    /// <summary>「11/12」（不受地區設定的日期分隔符號影響）。</summary>
    public static string Md(DateOnly d) => $"{d.Month}/{d.Day}";
}
