using System.Globalization;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

public enum DayPhaseKind { Off, Before, Working, Break, After }

public sealed record DayPhase(DayPhaseKind Kind, string Text);

public static class WorkMath
{
    static readonly char[] SegmentSeparators = ['、', ',', '，', ';'];
    static readonly char[] RangeSeparators = ['–', '—', '-', '~'];
    static readonly string[] TimeFormats = ["H:mm", "HH:mm"];

    /// <summary>解析 Hayaku 的時段文字，例如「08:30–12:00、13:00–17:30」；「不上班」回傳空集合。</summary>
    public static IReadOnlyList<WorkSegment> ParseSegments(string? text)
    {
        var list = new List<WorkSegment>();
        if (string.IsNullOrWhiteSpace(text))
            return list;
        foreach (var part in text.Split(SegmentSeparators, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            var range = part.Split(RangeSeparators, StringSplitOptions.TrimEntries);
            if (range.Length == 2 && TryTime(range[0], out var start) && TryTime(range[1], out var end) && end > start)
                list.Add(new WorkSegment(start, end));
        }
        list.Sort((a, b) => a.Start.CompareTo(b.Start));
        return list;
    }

    static bool TryTime(string s, out TimeOnly t) =>
        TimeOnly.TryParseExact(s, TimeFormats, CultureInfo.InvariantCulture, DateTimeStyles.None, out t);

    /// <summary>
    /// 某天的實際工時（分鐘）：把所有計時段聯集起來，重疊的時間只算一次。
    /// 進行中的計時算到 now；沒有起訖時間的段落（手動補登）直接加上時數。
    /// </summary>
    public static double UnionMinutes(IEnumerable<TimeEntry> entries, DateTime day, DateTime now)
    {
        var extra = entries.Where(e => e.StartedAt is null).Sum(e => e.DurationMinutes ?? 0);
        return Covered(entries, day, now).Sum(i => (i.End - i.Start).TotalMinutes) + extra;
    }

    /// <summary>某天各計時段（裁到當天、進行中算到 now）；沒有起訖時間的段落不算。</summary>
    public static List<(DateTime Start, DateTime End)> Intervals(IEnumerable<TimeEntry> entries, DateTime day, DateTime now)
    {
        var dayStart = day.Date;
        var dayEnd = dayStart.AddDays(1);
        var list = new List<(DateTime Start, DateTime End)>();
        foreach (var e in entries)
        {
            if (e.StartedAt is not { } started)
                continue;
            var s = started.LocalDateTime;
            var end = e.Running
                ? now
                : e.EndedAt?.LocalDateTime ?? (e.DurationMinutes is { } d ? s.AddMinutes(d) : s);
            if (s < dayStart) s = dayStart;
            if (end > dayEnd) end = dayEnd;
            if (end > now) end = now;
            if (end > s)
                list.Add((s, end));
        }
        list.Sort((a, b) => a.Start.CompareTo(b.Start));
        return list;
    }

    /// <summary>某天有在計時的時段（所有計時段的聯集，依時間排序、不重疊）。</summary>
    public static List<(DateTime Start, DateTime End)> Covered(IEnumerable<TimeEntry> entries, DateTime day, DateTime now)
    {
        var merged = new List<(DateTime Start, DateTime End)>();
        foreach (var (s, e) in Intervals(entries, day, now))
        {
            if (merged.Count > 0 && s <= merged[^1].End)
            {
                if (e > merged[^1].End)
                    merged[^1] = (merged[^1].Start, e);
            }
            else
            {
                merged.Add((s, e));
            }
        }
        return merged;
    }

    /// <summary>班表時段裡沒有任何計時的空檔（到 now 為止、至少 minMinutes 分鐘）。</summary>
    public static List<(DateTime Start, DateTime End)> Gaps(IReadOnlyList<WorkSegment> segments, IEnumerable<TimeEntry> entries,
        DateTime day, DateTime now, double minMinutes = 5)
    {
        var covered = Covered(entries, day, now);
        var gaps = new List<(DateTime Start, DateTime End)>();
        foreach (var seg in segments)
        {
            var cursor = day.Date + seg.Start.ToTimeSpan();
            var segEnd = day.Date + seg.End.ToTimeSpan();
            if (segEnd > now)
                segEnd = now;
            foreach (var (s, e) in covered)
            {
                if (e <= cursor || s >= segEnd)
                    continue;
                if (s > cursor)
                    gaps.Add((cursor, s));
                cursor = e;
            }
            if (cursor < segEnd)
                gaps.Add((cursor, segEnd));
        }
        return gaps.Where(g => (g.End - g.Start).TotalMinutes >= minMinutes).ToList();
    }

    public static double TaskMinutes(IEnumerable<TimeEntry> entries, string task, DateTime day, DateTime now) =>
        UnionMinutes(entries.Where(e => string.Equals(e.Task, task, StringComparison.OrdinalIgnoreCase)), day, now);

    /// <summary>
    /// 工時（跟 Hayaku 網頁一樣：先捨去不滿一分鐘的秒數，再換成小時到小數點後兩位）：
    /// 41.6 分 → "0.68h"；364.8 分 → "6.07h"；8 小時 → "8h"。
    /// </summary>
    public static string HoursText(double minutes) =>
        Math.Round(Math.Floor(minutes) / 60, 2, MidpointRounding.AwayFromZero).ToString("0.##", CultureInfo.InvariantCulture) + "h";

    /// <summary>小時數只顯示到小數點後一位（計算時保留原本的精確度）：2.13 → "2.1"、4 → "4"。</summary>
    public static string HoursNumber(double hours) => Math.Round(hours, 1, MidpointRounding.AwayFromZero).ToString("0.#", CultureInfo.InvariantCulture);

    /// <summary>"1:23:45"</summary>
    public static string Clock(double minutes)
    {
        var ts = TimeSpan.FromMinutes(Math.Max(0, minutes));
        return $"{(int)ts.TotalHours}:{ts.Minutes:00}:{ts.Seconds:00}";
    }

    /// <summary>"3 小時 27 分"</summary>
    public static string HoursMinutes(double minutes)
    {
        var total = (int)Math.Round(Math.Max(0, minutes));
        var (h, m) = (total / 60, total % 60);
        return h == 0 ? $"{m} 分" : m == 0 ? $"{h} 小時" : $"{h} 小時 {m} 分";
    }

    public static DayPhase Phase(DaySchedule? schedule, DateTime now, bool clockedOut)
    {
        if (schedule is null)
            return new DayPhase(DayPhaseKind.Off, "");
        if (schedule.Holiday is { } holiday)
            return new DayPhase(DayPhaseKind.Off, $"今天放假（{holiday}）");
        if (schedule.Working.Count == 0)
            return new DayPhase(DayPhaseKind.Off, schedule.OnLeave ? "今天請假" : "今天不上班");
        if (clockedOut)
            return new DayPhase(DayPhaseKind.After, "已手動下班");

        var t = TimeOnly.FromDateTime(now);
        var segs = schedule.Working;
        if (t < segs[0].Start)
            return new DayPhase(DayPhaseKind.Before, $"{segs[0].Start:HH:mm} 上班");
        if (t >= segs[^1].End)
            return new DayPhase(DayPhaseKind.After, "已過下班時間");
        var current = segs.FirstOrDefault(s => s.Start <= t && t < s.End);
        if (current is not null)
            return new DayPhase(DayPhaseKind.Working, $"上班中 · 到 {current.End:HH:mm}");
        var next = segs.First(s => s.Start > t);
        return new DayPhase(DayPhaseKind.Break, $"休息中 · {next.Start:HH:mm} 繼續");
    }
}
