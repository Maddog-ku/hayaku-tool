using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>接下來不用上班的日子（請假、放假、週末）存在本機，那幾天不問 Hayaku 就知道今天不用上班。</summary>
public static class OffDayCache
{
    const string Leave = "請假";

    /// <summary>從班表挑出 today 之後沒有上班時段的日子：日期 → 「請假」、假日名稱或空字串。</summary>
    public static Dictionary<string, string> From(IEnumerable<DaySchedule> days, DateOnly today) =>
        days.Where(d => d.Date > today && d.Working.Count == 0)
            .GroupBy(d => d.Date)
            .ToDictionary(g => g.Key.ToString("yyyy-MM-dd"), g => g.First().Holiday ?? (g.First().OnLeave ? Leave : ""));

    /// <summary>today 是快取裡不用上班的日子：回傳當作今天班表的 DaySchedule（沒有上班時段）；否則 null。</summary>
    public static DaySchedule? Today(IReadOnlyDictionary<string, string> offDays, DateOnly today) =>
        offDays.TryGetValue(today.ToString("yyyy-MM-dd"), out var label)
            ? new DaySchedule(today, [], 0, label == Leave, label is "" or Leave ? null : label)
            : null;
}
