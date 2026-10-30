using System.Text.Json;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class StatsTests
{
    [Fact]
    public void 依專案的統計()
    {
        var r = JsonDocument.Parse("""
            {"perspective":"project","range":{"from":"2026-10-01","to":"2026-10-31","granularity":"week","buckets":["2026-09-28","2026-10-05"]},
             "projects":[{"key":"F2P","name":"臉部辨識"},{"key":"REV","name":"內部"}],
             "rows":[{"project":"REV","actual":{"minutes":76},"taskCounts":{"TODO":2}},
                     {"project":"F2P","actual":{"minutes":11300},"estimated":{"minutes":1710},"taskCounts":{"DONE":43,"IN_PROGRESS":16},"overdueCount":11}],
             "cells":[{"project":"F2P","bucket":"2026-09-28","duration":{"minutes":60}},{"project":"F2P","bucket":"2026-09-28","duration":{"minutes":30}}],
             "summary":{"actual":{"minutes":11376},"estimated":{"minutes":1710},"taskCounts":{"DONE":43,"IN_PROGRESS":16,"TODO":2},"overdueCount":11}}
            """).RootElement;
        var s = HayakuApi.ParseStats(r);
        Assert.True(s.ByProject);
        Assert.Equal(["F2P", "REV"], s.Rows.Select(x => x.Key)); // 工時多的在前
        Assert.Equal(("臉部辨識", 11300, 43, 59), (s.Rows[0].Name, s.Rows[0].ActualMinutes, s.Rows[0].Done, s.Rows[0].Assigned));
        Assert.Equal(90, Assert.Single(s.Cells).Minutes);
        Assert.Equal((11376, 43, 61, 11), (s.ActualMinutes, s.Done, s.TaskTotal, s.Overdue));
        Assert.Equal(["2026-09-28", "2026-10-05"], s.Buckets);
    }
}
