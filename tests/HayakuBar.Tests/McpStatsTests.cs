using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class McpStatsTests
{
    [Fact]
    public void Format_多的排前面_同數依名稱()
    {
        var counts = new Dictionary<string, int> { ["list_notifications"] = 30, ["get_running_timers"] = 120, ["list_time_entries"] = 120 };
        Assert.Equal("MCP 請求（過去 60 分鐘）：共 270 次 · get_running_timers 120、list_time_entries 120、list_notifications 30",
            McpStats.Format(counts, TimeSpan.FromHours(1)));
    }

    [Fact]
    public void Format_不到一分鐘算一分鐘() =>
        Assert.StartsWith("MCP 請求（過去 1 分鐘）：共 1 次", McpStats.Format(new Dictionary<string, int> { ["whoami"] = 1 }, TimeSpan.FromSeconds(5)));
}
