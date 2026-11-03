using System.Text.Json;
using HayakuBar.Mcp;
using HayakuBar.Slack;

namespace HayakuBar.Tests;

public class SlackTests
{
    static JsonElement Json(string s) => JsonDocument.Parse(s).RootElement.Clone();

    [Fact]
    public void 回應ok_直接回傳() =>
        Assert.Equal("T1", SlackClient.Check("auth.test", Json("""{"ok":true,"team_id":"T1"}""")).GetProperty("team_id").GetString());

    [Fact]
    public void 回應不ok_丟出錯誤代碼()
    {
        var ex = Assert.Throws<SlackException>(() => SlackClient.Check("auth.test", Json("""{"ok":false,"error":"invalid_auth"}""")));
        Assert.Equal("invalid_auth", ex.Error);
        Assert.Contains("金鑰不對", ex.Message);
    }

    [Fact]
    public void 沒有ok欄位_當成失敗() =>
        Assert.Equal("unknown", Assert.Throws<SlackException>(() => SlackClient.Check("x", Json("{}"))).Error);

    [Fact]
    public void 不認得的錯誤代碼_直接顯示() => Assert.Equal("some_new_error", SlackClient.Describe("some_new_error"));

    [Fact]
    public void App設定_開了SocketMode與按鈕互動_權限只要私訊()
    {
        var m = Json(SlackClient.Manifest);
        Assert.True(m.GetProperty("settings").GetProperty("socket_mode_enabled").GetBoolean());
        Assert.True(m.GetProperty("settings").GetProperty("interactivity").GetProperty("is_enabled").GetBoolean());
        var scopes = m.GetProperty("oauth_config").GetProperty("scopes").GetProperty("bot").EnumerateArray().Select(s => s.GetString()).ToList();
        Assert.Equal(["chat:write", "im:write"], scopes);
        Assert.True(m.GetProperty("features").GetProperty("app_home").GetProperty("messages_tab_enabled").GetBoolean());
    }
}

public class SlackMessagesTests
{
    static readonly DateTimeOffset Started = new(2026, 11, 3, 9, 0, 0, TimeSpan.FromHours(8));
    static readonly DateTime Now = new(2026, 11, 3, 10, 0, 0);

    static NotificationItem N(string id, string type = "COMMENT", bool unread = true, int minutesAfterStart = 10, string? task = "F2P-1") =>
        new(id, type, "標籤", "內容", "小明", task, "F2P", null, Started.AddMinutes(minutesAfterStart), unread, 1);

    static ChangeRequest Cr(string id, string type = "UPDATE", int minutesAfterStart = 10) =>
        new(id, "F2P", type, "PENDING", "F2P-2", "做登入", new PersonRef("amy", "Amy"), null, Started.AddMinutes(minutesAfterStart),
            ["狀態：待處理 → 進行中"], "趕時程", null, 90, 0);

    static readonly JsonSerializerOptions Relaxed = new() { Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping };

    static string Json(object o) => JsonSerializer.Serialize(o, Relaxed);

    [Fact]
    public void 沒鎖定_只傳要動手處理的()
    {
        var seen = new HashSet<string>();
        var list = new[] { N("1"), N("2", "TEST_RESULT_REOPENED"), N("3", "TEST_RESULT_READY_TO_RETEST") };
        var send = SlackMessages.SelectNotifications(list, seen, Started, locked: false, canReview: false);
        Assert.Equal(["2", "3"], send.Select(n => n.Id));
    }

    [Fact]
    public void 鎖定_一般通知也傳()
    {
        var send = SlackMessages.SelectNotifications([N("1")], new HashSet<string>(), Started, locked: true, canReview: false);
        Assert.Single(send);
    }

    [Fact]
    public void 沒鎖定時看過的一般通知_之後鎖定也不補傳()
    {
        var seen = new HashSet<string>();
        SlackMessages.SelectNotifications([N("1")], seen, Started, locked: false, canReview: false);
        Assert.Empty(SlackMessages.SelectNotifications([N("1")], seen, Started, locked: true, canReview: false));
    }

    [Fact]
    public void 已讀_開程式前收到的_不傳()
    {
        var list = new[] { N("1", unread: false), N("2", minutesAfterStart: -5) };
        Assert.Empty(SlackMessages.SelectNotifications(list, new HashSet<string>(), Started, locked: true, canReview: false));
    }

    [Fact]
    public void 能審核時_有新的變更申請通知不傳_由審核訊息取代()
    {
        var list = new[] { N("1", "CHANGE_REQUESTED") };
        Assert.Empty(SlackMessages.SelectNotifications(list, new HashSet<string>(), Started, locked: true, canReview: true));
        Assert.Single(SlackMessages.SelectNotifications(list, new HashSet<string>(), Started, locked: true, canReview: false));
    }

    [Fact]
    public void 待審申請_開程式後的才傳_同一筆只傳一次()
    {
        var seen = new HashSet<string>();
        Assert.Equal(["a"], SlackMessages.SelectReviews([Cr("a"), Cr("b", minutesAfterStart: -1)], seen, Started).Select(c => c.Id));
        Assert.Empty(SlackMessages.SelectReviews([Cr("a")], seen, Started));
    }

    [Fact]
    public void 審核訊息_有核准駁回_新增任務的駁回要警告()
    {
        var p = SlackMessages.FromReview(Cr("a", "CREATE"), "https://x/t", Now);
        Assert.Equal("cr:a", p.Key);
        Assert.Contains("駁回會刪除 F2P-2 做登入", p.RejectWarning);
        var json = Json(SlackMessages.Blocks(p));
        Assert.Contains("\"action_id\":\"approve\"", json);
        Assert.Contains("\"action_id\":\"approve_watch\"", json);
        Assert.Contains("\"action_id\":\"reject\"", json);
        Assert.Contains("\"value\":\"cr:a\"", json);
        Assert.Contains("\"url\":\"https://x/t\"", json);
    }

    [Fact]
    public void 刪除任務的申請_不能核准並關注() =>
        Assert.DoesNotContain("approve_watch", Json(SlackMessages.Blocks(SlackMessages.FromReview(Cr("a", "DELETE"), "u", Now))));

    [Fact]
    public void 測試結果_被退回是開始修_待復驗是通過退回()
    {
        var fix = SlackMessages.FromNotification(N("1", "TEST_RESULT_REOPENED"), "u", Now);
        var retest = SlackMessages.FromNotification(N("2", "TEST_RESULT_READY_TO_RETEST"), "u", Now);
        Assert.Equal(("fix", "retest"), (fix.Kind, retest.Kind));
        Assert.Contains("\"action_id\":\"fix\"", Json(SlackMessages.Blocks(fix)));
        var r = Json(SlackMessages.Blocks(retest));
        Assert.Contains("\"action_id\":\"pass\"", r);
        Assert.Contains("\"action_id\":\"reopen\"", r);
    }

    [Fact]
    public void 處理過了_按鈕換成結果()
    {
        var p = SlackMessages.FromNotification(N("1"), "u", Now);
        p.Done = "✓ 已標為已讀";
        var json = Json(SlackMessages.Blocks(p));
        Assert.DoesNotContain("actions", json);
        Assert.Contains("已標為已讀", json);
    }

    [Fact]
    public void 失敗_保留按鈕並顯示原因()
    {
        var json = Json(SlackMessages.Blocks(SlackMessages.FromNotification(N("1"), "u", Now), "連不上"));
        Assert.Contains("\"action_id\":\"read\"", json);
        Assert.Contains("⚠ 連不上", json);
    }

    [Fact]
    public void 很多則合成一則_全部標為已讀()
    {
        var p = SlackMessages.FromNotices([N("1"), N("2"), N("3"), N("4")], "u", Now);
        Assert.Equal(["1", "2", "3", "4"], p.NotificationIds);
        Assert.Equal("有 4 則新通知", p.Title);
    }

    [Fact]
    public void 文字要跳脫() => Assert.Equal("a &lt;b&gt; &amp; c", SlackMessages.Escape("a <b> & c"));

    [Fact]
    public void 表單_駁回說明可不填_退回一定要填()
    {
        var p = SlackMessages.FromReview(Cr("a", "CREATE"), "u", Now);
        var reject = Json(SlackMessages.ReasonModal(p, SlackMessages.Reject));
        Assert.Contains("\"optional\":true", reject);
        Assert.Contains("\"private_metadata\":\"reject|cr:a\"", reject);
        Assert.Contains("駁回會刪除", reject);
        Assert.Contains("\"optional\":false", Json(SlackMessages.ReasonModal(p, SlackMessages.Reopen)));
    }

    [Fact]
    public void 解析按鈕()
    {
        var payload = JsonDocument.Parse("""
            {"type":"block_actions","user":{"id":"U1"},"trigger_id":"t1",
             "container":{"type":"message","message_ts":"111.2","channel_id":"D1"},
             "actions":[{"action_id":"approve","value":"cr:a","type":"button"}]}
            """).RootElement;
        var i = SlackMessages.ParseInteraction(payload);
        Assert.Equal(("block_actions", "U1", "approve", "cr:a", "t1", "D1", "111.2"),
            (i.Type, i.UserId, i.ActionId, i.Value, i.TriggerId, i.Channel, i.Ts));
    }

    [Fact]
    public void 解析表單送出()
    {
        var payload = JsonDocument.Parse("""
            {"type":"view_submission","user":{"id":"U1"},
             "view":{"callback_id":"reason","private_metadata":"reopen|n:9",
                     "state":{"values":{"reason":{"text":{"type":"plain_text_input","value":"還是會當"}}}}}}
            """).RootElement;
        var i = SlackMessages.ParseInteraction(payload);
        Assert.Equal(("reason", "reopen|n:9", "還是會當"), (i.CallbackId, i.Metadata, i.Reason));
    }

    [Fact]
    public void 超過7天的訊息_不留()
    {
        var s = new SlackState { Posts = [new SlackPost { Key = "a", PostedAt = Now.AddDays(-8) }, new SlackPost { Key = "b", PostedAt = Now.AddDays(-1) }] };
        s.Prune(Now);
        Assert.Equal(["b"], s.Posts.Select(p => p.Key));
    }
}
