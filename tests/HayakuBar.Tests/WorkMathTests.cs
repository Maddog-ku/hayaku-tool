using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class WorkMathTests
{
    static TimeEntry Entry(string task, DateTime start, DateTime? end, bool running = false) =>
        new(Guid.NewGuid().ToString(), task, task, new DateTimeOffset(start), end is null ? null : new DateTimeOffset(end.Value), null, running, null);

    [Fact]
    public void ParseSegments_讀得懂Hayaku的時段格式()
    {
        var segs = WorkMath.ParseSegments("08:30–12:00、13:00–17:30");
        Assert.Equal(2, segs.Count);
        Assert.Equal(new TimeOnly(8, 30), segs[0].Start);
        Assert.Equal(new TimeOnly(17, 30), segs[1].End);
    }

    [Theory]
    [InlineData("不上班")]
    [InlineData("")]
    [InlineData(null)]
    public void ParseSegments_不上班是空的(string? text) => Assert.Empty(WorkMath.ParseSegments(text));

    [Fact]
    public void UnionMinutes_並行計時只算一次()
    {
        var day = new DateTime(2026, 10, 28);
        var now = day.AddHours(10);
        var entries = Enumerable.Range(0, 6)
            .Select(i => Entry($"F2P-{i}", day.AddHours(8).AddMinutes(30), null, running: true))
            .ToList();
        Assert.Equal(90, WorkMath.UnionMinutes(entries, day, now), precision: 3);
    }

    [Fact]
    public void UnionMinutes_午休空檔不算()
    {
        var day = new DateTime(2026, 10, 27);
        var now = day.AddHours(20);
        var entries = new[]
        {
            Entry("A", day.AddHours(8.5), day.AddHours(12)),
            Entry("B", day.AddHours(8.5), day.AddHours(12)),
            Entry("A", day.AddHours(13), day.AddHours(17.5)),
            Entry("C", day.AddHours(16), day.AddHours(18)),
        };
        // 08:30–12:00 (210) + 13:00–18:00 (300)
        Assert.Equal(510, WorkMath.UnionMinutes(entries, day, now), precision: 3);
        Assert.Equal(480, WorkMath.TaskMinutes(entries, "A", day, now), precision: 3);
    }

    [Fact]
    public void UnionMinutes_跨日的計時只算今天的部分()
    {
        var day = new DateTime(2026, 10, 28);
        var entries = new[] { Entry("A", day.AddHours(-1), null, running: true) };
        Assert.Equal(60, WorkMath.UnionMinutes(entries, day, day.AddHours(1)), precision: 3);
    }

    [Theory]
    [InlineData(480, "8h")]
    [InlineData(208, "3.5h")]
    [InlineData(0, "0h")]
    public void HoursText_格式(double minutes, string expected) => Assert.Equal(expected, WorkMath.HoursText(minutes));

    [Fact]
    public void Phase_判斷上班午休下班()
    {
        var schedule = new DaySchedule(new DateOnly(2026, 10, 28), WorkMath.ParseSegments("08:30–12:00、13:00–17:30"), 480, false, null);
        var day = new DateTime(2026, 10, 28);
        Assert.Equal(DayPhaseKind.Before, WorkMath.Phase(schedule, day.AddHours(8), false).Kind);
        Assert.Equal(DayPhaseKind.Working, WorkMath.Phase(schedule, day.AddHours(9), false).Kind);
        Assert.Equal(DayPhaseKind.Break, WorkMath.Phase(schedule, day.AddHours(12.5), false).Kind);
        Assert.Equal(DayPhaseKind.After, WorkMath.Phase(schedule, day.AddHours(18), false).Kind);
        Assert.Equal(DayPhaseKind.After, WorkMath.Phase(schedule, day.AddHours(9), true).Kind);
    }
    [Theory]
    [InlineData(2.13, "2.1")]
    [InlineData(3.58, "3.6")]
    [InlineData(4, "4")]
    [InlineData(2.25, "2.3")]
    [InlineData(0.04, "0")]
    public void HoursNumber_只顯示到小數點後一位(double hours, string expected) =>
        Assert.Equal(expected, WorkMath.HoursNumber(hours));

    [Fact]
    public void Covered_重疊的計時合併成一段()
    {
        var day = new DateTime(2026, 10, 28);
        var entries = new[]
        {
            Entry("A", day.AddHours(9), day.AddHours(11)),
            Entry("B", day.AddHours(10), day.AddHours(12)),
            Entry("C", day.AddHours(13), day.AddHours(14)),
        };
        var covered = WorkMath.Covered(entries, day, day.AddHours(20));
        Assert.Equal([(day.AddHours(9), day.AddHours(12)), (day.AddHours(13), day.AddHours(14))], covered);
    }

    [Fact]
    public void Gaps_只列班表內沒計到的空檔且算到現在()
    {
        var day = new DateTime(2026, 10, 28);
        var segs = WorkMath.ParseSegments("08:30–12:00、13:00–17:30");
        var entries = new[]
        {
            Entry("A", day.AddHours(9), day.AddHours(11.5)),
            Entry("B", day.AddHours(11.5).AddMinutes(2), day.AddHours(12)), // 2 分鐘的縫不算
            Entry("C", day.AddHours(12), day.AddHours(13.5)),                // 午休的計時不影響
        };
        var gaps = WorkMath.Gaps(segs, entries, day, day.AddHours(15));
        Assert.Equal([(day.AddHours(8.5), day.AddHours(9)), (day.AddHours(13.5), day.AddHours(15))], gaps);
    }
}
