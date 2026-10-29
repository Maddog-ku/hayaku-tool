using System.Globalization;
using HayakuBar.Mcp;

namespace HayakuBar.Eip;

/// <summary>要在 Hayaku 登記的一筆請假：DAY 是整天（可跨天），RANGE 是某一天的指定時段。</summary>
public sealed record LeavePiece(string Span, DateOnly Date, DateOnly? EndDate = null, TimeOnly? Start = null, TimeOnly? End = null)
{
    static readonly CultureInfo Zh = new("zh-TW");

    public DateTime From => Span == "DAY" ? Date.ToDateTime(TimeOnly.MinValue) : Date.ToDateTime(Start!.Value);

    public DateTime To => Span == "DAY" ? (EndDate ?? Date).AddDays(1).ToDateTime(TimeOnly.MinValue) : Date.ToDateTime(End!.Value);

    /// <summary>add_leave 的參數。</summary>
    public Dictionary<string, object?> Args(string note)
    {
        var args = new Dictionary<string, object?> { ["span"] = Span, ["date"] = Date.ToString("yyyy-MM-dd") };
        if (EndDate is { } end && end > Date)
            args["endDate"] = end.ToString("yyyy-MM-dd");
        if (Span == "RANGE")
        {
            args["start"] = Start!.Value.ToString("HH:mm");
            args["end"] = End!.Value.ToString("HH:mm");
        }
        if (note.Length > 0)
            args["note"] = note.Length > 500 ? note[..500] : note;
        return args;
    }

    /// <summary>11/3（一）整天、11/3（一）到 11/5（三）整天、11/3（一）13:30–17:30。</summary>
    public string Describe()
    {
        var first = Date.ToString("M/d（ddd）", Zh);
        if (Span == "RANGE")
            return $"{first} {Start:HH\\:mm}–{End:HH\\:mm}";
        return EndDate is { } end && end > Date ? $"{first} 到 {end.ToString("M/d（ddd）", Zh)} 整天" : $"{first} 整天";
    }
}

/// <summary>EIP 假單 → Hayaku 請假的規則（純函式，方便測試）。</summary>
public static class LeavePlanner
{
    /// <summary>要同步的性質：請假、補假（銷假用來抵掉請假，黃卡不是請假）。</summary>
    static readonly string[] LeaveKinds = ["請假", "補假"];

    /// <summary>
    /// 有效的請假：只留請假／補假（同一單號只算一次），被銷假整段蓋掉的不算。
    /// 只銷掉一部分的照原本的請假登記（EIP 的格式沒有樣本，不猜）。
    /// </summary>
    public static List<EipLeave> Effective(IEnumerable<EipLeave> rows)
    {
        var all = rows.ToList();
        var cancels = all.Where(r => r.Kind == "銷假").ToList();
        return all
            .Where(r => LeaveKinds.Contains(r.Kind) && r.No.Length > 0)
            .DistinctBy(r => r.No)
            .Where(r => !cancels.Any(c => c.Start <= r.Start && c.End >= r.End))
            .OrderBy(r => r.Start)
            .ToList();
    }

    /// <summary>after 之後第一個要上班的日子（有上班時段，或已經請假的工作日）；班表範圍內找不到回 null。</summary>
    public static DateOnly? NextWorkday(IEnumerable<DaySchedule> days, DateOnly after) =>
        days.Where(d => d.Date > after && d.Holiday is null && (d.Working.Count > 0 || d.OnLeave))
            .OrderBy(d => d.Date)
            .Select(d => (DateOnly?)d.Date)
            .FirstOrDefault();

    /// <summary>
    /// 某一天的上班時段：班表有就用班表；已經請假的日子班表可能是空的，改用平常的時段；
    /// 假日、放假是空的；班表範圍外的平日用平常的時段，週末是空的。
    /// </summary>
    public static Func<DateOnly, IReadOnlyList<WorkSegment>> Working(IReadOnlyList<DaySchedule> days)
    {
        var typical = days.FirstOrDefault(d => d.Working.Count > 0)?.Working ?? [];
        return date =>
        {
            if (days.FirstOrDefault(d => d.Date == date) is not { } s)
                return date.DayOfWeek is DayOfWeek.Saturday or DayOfWeek.Sunday ? [] : typical;
            if (s.Working.Count > 0)
                return s.Working;
            return s.OnLeave && s.Holiday is null ? typical : [];
        };
    }

    /// <summary>
    /// 把一張假單換成 Hayaku 的請假：涵蓋當天整個上班時段的算整天，連續的整天合併成一筆（中間的假日不打斷）；
    /// 其他用指定時段。不用上班的日子、完全落在上班時段外的部分略過。
    /// </summary>
    public static List<LeavePiece> Plan(EipLeave leave, Func<DateOnly, IReadOnlyList<WorkSegment>> working)
    {
        var pieces = new List<LeavePiece>();
        DateOnly? runStart = null, runEnd = null;

        void Flush()
        {
            if (runStart is { } s && runEnd is { } e)
                pieces.Add(new LeavePiece("DAY", s, e > s ? e : null));
            runStart = runEnd = null;
        }

        var last = DateOnly.FromDateTime(leave.End.AddTicks(-1));
        for (var day = DateOnly.FromDateTime(leave.Start); day <= last; day = day.AddDays(1))
        {
            var dayStart = day.ToDateTime(TimeOnly.MinValue);
            var from = leave.Start > dayStart ? leave.Start : dayStart;
            var to = leave.End < dayStart.AddDays(1) ? leave.End : dayStart.AddDays(1);
            var segs = working(day);
            if (to <= from || segs.Count == 0)
                continue; // 不用上班的日子：不打斷連續的整天

            var workStart = dayStart + segs[0].Start.ToTimeSpan();
            var workEnd = dayStart + segs[^1].End.ToTimeSpan();
            if (from <= workStart && to >= workEnd)
            {
                runStart ??= day;
                runEnd = day;
                continue;
            }
            if (to <= workStart || from >= workEnd)
                continue;
            Flush();
            // 跨天假單的頭尾那天會延伸到午夜，裁到當天的上班時間
            var start = from > workStart ? from : workStart;
            var end = to < workEnd ? to : workEnd;
            pieces.Add(new LeavePiece("RANGE", day, Start: TimeOnly.FromDateTime(start), End: TimeOnly.FromDateTime(end)));
        }
        Flush();
        return pieces;
    }

    public static bool Overlaps(DateTime aFrom, DateTime aTo, DateTime bFrom, DateTime bTo) => aFrom < bTo && bFrom < aTo;

    /// <summary>
    /// 現在要不要做下班後的工作（一天一次）：今天要上班（請假、放假、週末一律不做）、今天還沒做過，
    /// 而且已經下班（班表下班時間到、手動按了下班），或從沒做過、昨天以前就漏了（那天電腦沒開到下班）。
    /// </summary>
    public static bool IsDue(DaySchedule today, bool clockedOut, DateTime now, string lastSyncDay)
    {
        var date = DateOnly.FromDateTime(now);
        if (today.Working.Count == 0 || lastSyncDay == date.ToString("yyyy-MM-dd"))
            return false;
        if (!DateOnly.TryParseExact(lastSyncDay, "yyyy-MM-dd", out var last) || last < date.AddDays(-1))
            return true;
        if (clockedOut)
            return true;
        return now >= date.ToDateTime(today.Working[^1].End);
    }

    /// <summary>
    /// 這次做完算哪一天的（存成 lastSyncDay）：已經下班就是今天；上班前或上班中補做的只算昨天，
    /// 今天下班時照樣再做一次（白天才在 EIP 請的假）。
    /// </summary>
    public static DateOnly CoveredDay(DaySchedule today, bool clockedOut, DateTime now)
    {
        var date = DateOnly.FromDateTime(now);
        var afterWork = clockedOut || today.Working.Count == 0 || now >= date.ToDateTime(today.Working[^1].End);
        return afterWork ? date : date.AddDays(-1);
    }
}
