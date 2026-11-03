using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Slack;

/// <summary>Slack 上按了按鈕或送出表單（Socket Mode 收到的 interactive）。</summary>
public sealed record SlackInteraction(
    string Type,
    string? UserId,
    string? ActionId,
    string? Value,
    string? TriggerId,
    string? Channel,
    string? Ts,
    string? CallbackId,
    string? Metadata,
    string? Reason);

/// <summary>
/// 要傳到 Slack 的訊息怎麼排（Block Kit）、按鈕代表什麼、哪些通知要傳。
/// 按鈕的 action_id 是動作，value 是訊息代號（SlackPost.Key），重開程式後也找得回對應的通知或申請。
/// </summary>
public static class SlackMessages
{
    public const string Read = "read", Archive = "archive", Approve = "approve", ApproveWatch = "approve_watch",
        Reject = "reject", Pass = "pass", Reopen = "reopen", Fix = "fix", Open = "open";

    /// <summary>表單的 callback_id；private_metadata 是「動作|訊息代號」。</summary>
    public const string ReasonCallback = "reason";

    /// <summary>一次超過這麼多則一般通知就合成一則（跟本機提示一樣）。</summary>
    public const int MaxSingle = 3;

    // ───────────── 哪些要傳 ─────────────

    /// <summary>要動手處理的通知：測試結果被退回（要修）、進待復驗（要驗）。</summary>
    public static bool IsActionable(NotificationItem n) => n.IsTestResult && n.TaskKey is not null && (n.IsReopened || n.IsReadyToRetest);

    /// <summary>
    /// 這次同步新出現（開程式後才收到、還沒看過）的未讀通知裡，要傳到 Slack 的：要動手處理的一律傳，其他的只在螢幕鎖定時傳。
    /// 看過的記在 seen，所以沒鎖定時收到的一般通知，之後鎖定也不會補傳。
    /// 能審核時「有新的變更申請」不傳，由待我審核的訊息（可以直接核准、駁回）取代。
    /// </summary>
    public static List<NotificationItem> SelectNotifications(IEnumerable<NotificationItem> notifications, ISet<string> seen,
        DateTimeOffset startedAt, bool locked, bool canReview)
    {
        var send = new List<NotificationItem>();
        foreach (var n in notifications)
        {
            if (!n.Unread || n.CreatedAt <= startedAt || !seen.Add($"n:{n.Id}"))
                continue;
            if (n.Type == "CHANGE_REQUESTED" && canReview)
                continue;
            if (IsActionable(n) || locked)
                send.Add(n);
        }
        return send;
    }

    /// <summary>開程式後才送出、還沒傳過的待審申請（一律傳）。</summary>
    public static List<ChangeRequest> SelectReviews(IEnumerable<ChangeRequest> pending, ISet<string> seen, DateTimeOffset startedAt) =>
        pending.Where(c => c.CreatedAt > startedAt && seen.Add($"cr:{c.Id}")).ToList();

    // ───────────── 訊息內容 ─────────────

    public static SlackPost FromNotification(NotificationItem n, string url, DateTime now)
    {
        var who = n.Actor is { } actor ? $"{actor}：" : "";
        var kind = !IsActionable(n) ? "notice" : n.IsReopened ? "fix" : "retest";
        return new SlackPost
        {
            Key = $"n:{n.Id}",
            Kind = kind,
            Title = kind == "notice" && n.TaskKey is null ? n.Label : $"{n.Label}・{n.TaskKey}",
            Body = who + n.Text,
            Url = url,
            NotificationIds = [n.Id],
            TaskKey = n.TaskKey,
            ToastTag = $"n:{n.Id}",
            PostedAt = now,
        };
    }

    public static SlackPost FromNotices(IReadOnlyList<NotificationItem> list, string url, DateTime now) => new()
    {
        Key = $"ns:{now:yyyyMMddHHmmss}:{list[0].Id}",
        Kind = "notices",
        Title = $"有 {list.Count} 則新通知",
        Body = string.Join("\n", list.Take(10).Select(n => $"• {n.Label}：{n.Text}")) + (list.Count > 10 ? "\n…" : ""),
        Url = url,
        NotificationIds = list.Select(n => n.Id).ToList(),
        PostedAt = now,
    };

    public static SlackPost FromReview(ChangeRequest c, string url, DateTime now)
    {
        var lines = new List<string> { $"{c.Requester?.DisplayName ?? "有人"}：{c.TaskKey} {c.TaskTitle}".TrimEnd() };
        lines.AddRange(c.Changes.Take(5).Select(change => "• " + change));
        if (c.Changes.Count > 5)
            lines.Add("…");
        if (!string.IsNullOrWhiteSpace(c.Reason))
            lines.Add($"理由：{c.Reason}");
        var logged = c.LoggedMinutes > 0 ? $"（已經記錄 {WorkMath.HoursMinutes(c.LoggedMinutes)} 工時）" : "";
        return new SlackPost
        {
            Key = $"cr:{c.Id}",
            Kind = "review",
            Title = $"待審核・{c.TypeText}",
            Body = string.Join("\n", lines),
            Url = url,
            ChangeRequestId = c.Id,
            CanWatch = c.Type != "DELETE",
            RejectWarning = c.Type == "CREATE" ? $"駁回會刪除 {c.TaskKey} {c.TaskTitle}{logged}".TrimEnd() : null,
            TaskKey = c.TaskKey,
            ToastTag = $"cr:{c.Id}",
            PostedAt = now,
        };
    }

    /// <summary>通知列與不能顯示 blocks 時看到的文字。</summary>
    public static string Fallback(SlackPost p) =>
        p.Body.Length > 0 ? $"{p.Title}：{p.Body.Split('\n')[0]}" : p.Title;

    /// <summary>訊息的 blocks：標題與內容；還沒處理放按鈕（error：上次按了失敗的原因），處理了換成結果。</summary>
    public static List<object> Blocks(SlackPost p, string? error = null)
    {
        var blocks = new List<object> { Section($"*{Escape(p.Title)}*" + (p.Body.Length > 0 ? "\n" + Escape(p.Body) : "")) };
        if (p.Done is { } done)
        {
            blocks.Add(Context(done));
            return blocks;
        }
        blocks.Add(new { type = "actions", block_id = "act", elements = Buttons(p) });
        if (error is not null)
            blocks.Add(Context("⚠ " + error));
        return blocks;
    }

    static List<object> Buttons(SlackPost p)
    {
        var list = new List<object>();
        switch (p.Kind)
        {
            case "review":
                list.Add(Button("✓ 核准", Approve, p.Key, "primary"));
                if (p.CanWatch)
                    list.Add(Button("核准並關注", ApproveWatch, p.Key));
                list.Add(Button("✕ 駁回", Reject, p.Key, "danger"));
                break;
            case "retest":
                list.Add(Button("✓ 通過", Pass, p.Key, "primary"));
                list.Add(Button("↩ 退回", Reopen, p.Key, "danger"));
                break;
            case "fix":
                list.Add(Button("▶ 開始修並計時", Fix, p.Key, "primary"));
                list.Add(Button("標為已讀", Read, p.Key));
                break;
            case "notices":
                list.Add(Button("全部標為已讀", Read, p.Key, "primary"));
                break;
            default:
                list.Add(Button("標為已讀", Read, p.Key, "primary"));
                list.Add(Button("封存", Archive, p.Key));
                break;
        }
        if (p.Url is { Length: > 0 } url)
            list.Add(new { type = "button", text = Plain("開啟"), action_id = Open, url });
        return list;
    }

    /// <summary>駁回申請、退回測試結果要填原因的表單；駁回的說明可以不填，退回一定要填。</summary>
    public static object ReasonModal(SlackPost p, string action)
    {
        var reject = action == Reject;
        var blocks = new List<object> { Section($"*{Escape(p.Title)}*\n{Escape(p.Body.Split('\n')[0])}") };
        if (reject && p.RejectWarning is { } warning)
            blocks.Add(Context("⚠ " + warning));
        blocks.Add(new
        {
            type = "input",
            block_id = "reason",
            optional = reject,
            label = Plain(reject ? "給申請人的說明" : "退回的原因"),
            element = new { type = "plain_text_input", action_id = "text", multiline = true },
        });
        return new
        {
            type = "modal",
            callback_id = ReasonCallback,
            private_metadata = $"{action}|{p.Key}",
            title = Plain(reject ? "駁回申請" : "退回測試結果"),
            submit = Plain(reject ? "駁回" : "退回"),
            close = Plain("取消"),
            blocks,
        };
    }

    static object Section(string mrkdwn) =>
        new { type = "section", text = new { type = "mrkdwn", text = mrkdwn.Length > 2900 ? mrkdwn[..2900] + "…" : mrkdwn } };

    static object Context(string text) => new { type = "context", elements = new object[] { new { type = "mrkdwn", text = Escape(text) } } };

    static object Button(string text, string action, string value, string? style = null) =>
        new { type = "button", text = Plain(text), action_id = action, value, style };

    static object Plain(string text) => new { type = "plain_text", text, emoji = true };

    /// <summary>Slack 文字裡 &amp; &lt; &gt; 要跳脫。</summary>
    public static string Escape(string s) => s.Replace("&", "&amp;").Replace("<", "&lt;").Replace(">", "&gt;");

    // ───────────── 收到的互動 ─────────────

    public static SlackInteraction ParseInteraction(JsonElement p)
    {
        string? S(JsonElement e, params string[] path)
        {
            foreach (var name in path)
            {
                if (e.ValueKind != JsonValueKind.Object || !e.TryGetProperty(name, out e))
                    return null;
            }
            return e.ValueKind == JsonValueKind.String ? e.GetString() : null;
        }

        var type = S(p, "type") ?? "";
        string? actionId = null, value = null, reason = null;
        if (p.TryGetProperty("actions", out var actions) && actions.ValueKind == JsonValueKind.Array && actions.GetArrayLength() > 0)
        {
            actionId = S(actions[0], "action_id");
            value = S(actions[0], "value");
        }
        if (p.TryGetProperty("view", out var view))
            reason = S(view, "state", "values", "reason", "text", "value");
        return new SlackInteraction(type, S(p, "user", "id"), actionId, value, S(p, "trigger_id"),
            S(p, "container", "channel_id") ?? S(p, "channel", "id"), S(p, "container", "message_ts") ?? S(p, "message", "ts"),
            S(p, "view", "callback_id"), S(p, "view", "private_metadata"), reason);
    }
}
