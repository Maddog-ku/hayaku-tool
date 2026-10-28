using System.Text.Json;
using System.Windows.Input;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

/// <summary>樣本取自實際 Hayaku MCP 回應。</summary>
public class ExtrasParseTests
{
    [Fact]
    public void ParseMeetings_讀取會議()
    {
        const string json = """
            {"meetings":[{"meeting":"REVIVAL-9","id":"4aa94b66","title":"HAYAKU專案管理系統介紹會議-2","project":"REVIVAL",
              "startAt":"2026-10-27T07:35:00.000Z","endAt":"2026-10-27T09:00:00.000Z","location":null,"recurring":false,
              "attendees":8,"inMeeting":0,"me":"已離開"}],"more":false}
            """;
        var m = Assert.Single(HayakuApi.ParseMeetings(JsonDocument.Parse(json).RootElement));
        Assert.Equal("REVIVAL-9", m.Key);
        Assert.Equal("已離開", m.MyStatus);
        Assert.False(m.InMeeting);
        Assert.Equal(85, (m.EndAt - m.StartAt).TotalMinutes);
    }

    [Fact]
    public void ParseNotifications_組出任務編號與文字()
    {
        const string json = """
            {"items":[
              {"count":1,"unreadCount":1,"latest":{"id":"n1","type":"TASK_ASSIGNED",
                "actor":{"username":"hayaku-admin","displayName":"系統管理員"},
                "project":{"key":"F2P","name":"F2P-臉部辨識結合印表機"},
                "task":{"id":"t1","number":96,"title":"優化上傳","deleted":false,"kind":"TASK"},
                "payload":{"task":"F2P-96 優化上傳","count":1},"readAt":null,"createdAt":"2026-10-27T07:34:51.828Z"}},
              {"count":1,"unreadCount":0,"latest":{"id":"n2","type":"MEETING_INVITED","actor":{"displayName":"羅瑞騰"},
                "project":{"key":"REVIVAL","name":"Revival"},
                "task":{"id":"m1","number":9,"title":"介紹會議","kind":"MEETING"},
                "payload":{"task":"REVIVAL-9 介紹會議"},"readAt":"2026-10-27T08:08:29.851Z","createdAt":"2026-10-27T07:38:01.138Z"}}
            ]}
            """;
        var list = HayakuApi.ParseNotifications(JsonDocument.Parse(json).RootElement);
        Assert.Equal(2, list.Count);
        Assert.Equal("被指派任務", list[0].Label);
        Assert.Equal("F2P-96", list[0].TaskKey);
        Assert.Equal("F2P-96 優化上傳", list[0].Text);
        Assert.Equal("系統管理員", list[0].Actor);
        Assert.True(list[0].Unread);
        Assert.Equal("m1", list[1].MeetingId);
        Assert.False(list[1].Unread);
    }

    [Theory]
    [InlineData("Win+Shift+H", true)]
    [InlineData("Ctrl+Alt+F9", true)]
    [InlineData("Ctrl+1", true)]
    [InlineData("H", false)]          // 沒有修飾鍵
    [InlineData("Win+Shift+", false)] // 沒有主鍵
    [InlineData("Ctrl+NotAKey", false)]
    public void Hotkey_解析(string text, bool ok) => Assert.Equal(ok, GlobalHotkey.TryParse(text, out _, out _));

    [Fact]
    public void Hotkey_格式化()
    {
        Assert.Equal("Win+Shift+H", GlobalHotkey.Format(ModifierKeys.Shift, win: true, Key.H));
        Assert.Equal("Ctrl+Alt+1", GlobalHotkey.Format(ModifierKeys.Control | ModifierKeys.Alt, win: false, Key.D1));
        Assert.Null(GlobalHotkey.Format(ModifierKeys.Shift, win: false, Key.LeftShift)); // 只有修飾鍵
        Assert.Null(GlobalHotkey.Format(ModifierKeys.None, win: false, Key.H));           // 沒有修飾鍵
    }
}
