using System.Text.Json;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

/// <summary>樣本取自實際 Hayaku MCP 回應。</summary>
public class HayakuApiParseTests
{
    [Fact]
    public void ParseDay_讀取上班時段與應出勤()
    {
        const string json = """
            {"date":"2026-10-28","weekday":"週三","holiday":null,"scheduled":"08:30–12:00、13:00–17:30",
             "working":"08:30–12:00、13:00–17:30","onLeave":false,"expected":{"minutes":480,"display":"8 小時"}}
            """;
        var day = HayakuApi.ParseDay(JsonDocument.Parse(json).RootElement, default);
        Assert.Equal(new DateOnly(2026, 10, 28), day.Date);
        Assert.Equal(2, day.Working.Count);
        Assert.Equal(480, day.ExpectedMinutes);
        Assert.Null(day.Holiday);
    }

    [Fact]
    public void ParseEntries_讀取計時與停止原因()
    {
        const string json = """
            {"entries":[
              {"id":"a","task":"F2P-5","title":"第一階段研究","startedAt":"2026-10-28T00:28:25.871Z","endedAt":null,"duration":null,"running":true,"source":"TIMER","note":null},
              {"id":"b","task":"F2P-25","title":"第二階段研究","startedAt":"2026-10-27T00:30:00.000Z","endedAt":"2026-10-27T04:00:00.000Z","duration":{"minutes":210,"display":"3.5 小時"},"running":false,"source":"TIMER","note":null,"stopReason":"BREAK"}
            ]}
            """;
        var entries = HayakuApi.ParseEntries(JsonDocument.Parse(json).RootElement, "entries");
        Assert.Equal(2, entries.Count);
        Assert.True(entries[0].Running);
        Assert.Null(entries[0].EndedAt);
        Assert.Equal(210, entries[1].DurationMinutes);
        Assert.Equal("BREAK", entries[1].StopReason);
    }
}
