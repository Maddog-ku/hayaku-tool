using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>聊天裡的投票：題目、各選項票數與誰投了、狀態與截止時間（唯讀；MCP 不能投票，投票請到網頁）。</summary>
public sealed class PollWindow : SmallDialog
{
    PollWindow() : base("投票", 460)
    {
    }

    public static void Show(TimerEngine engine, string? pollId, Window? owner)
    {
        if (pollId is null)
            return;
        var window = new PollWindow { Owner = owner };
        window.Body.Children.Add(window.Label("讀取中…"));
        window.AddButton("關閉", primary: true, () =>
        {
            window.Close();
            return Task.CompletedTask;
        });
        window.Loaded += async (_, _) => await window.LoadAsync(engine, pollId);
        window.Show();
    }

    async Task LoadAsync(TimerEngine engine, string pollId)
    {
        PollInfo poll;
        try
        {
            poll = await engine.Api.GetPollAsync(pollId, CancellationToken.None);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取投票 {pollId} 失敗", ex);
            Body.Children.Clear();
            ShowError($"讀不到投票：{ex.Message}");
            return;
        }
        Body.Children.Clear();
        Body.Children.Add(new TextBlock { Text = poll.Question, FontWeight = FontWeights.SemiBold, FontSize = 15, TextWrapping = TextWrapping.Wrap });
        var meta = new[]
        {
            poll.Type, poll.Anonymous ? "匿名" : null, poll.Status, poll.ClosesAt is { } at ? $"{at.ToLocalTime():M/d HH:mm} 截止" : null,
            poll.CreatedBy is { } by ? $"{by} 發起" : null,
            poll.Voted is { } v ? $"{v} 人投了" + (poll.Pending is { } p && p > 0 ? $"、{p} 人還沒投" : "") : null,
        };
        Body.Children.Add(new TextBlock
        {
            Text = string.Join(" · ", meta.Where(m => !string.IsNullOrWhiteSpace(m))), Foreground = Brushes2.Get("MutedBrush"),
            TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 2, 0, 10),
        });
        var max = Math.Max(1, poll.Options.Select(o => o.Votes ?? 0).DefaultIfEmpty().Max());
        foreach (var o in poll.Options)
        {
            var head = new DockPanel { Margin = new Thickness(0, 4, 0, 2) };
            var count = new TextBlock { Text = o.Votes is { } n ? $"{n} 票" : "", Foreground = Brushes2.Get("MutedBrush") };
            DockPanel.SetDock(count, Dock.Right);
            head.Children.Add(count);
            head.Children.Add(new TextBlock
            {
                Text = (o.Mine ? "✓ " : "") + o.Label, TextWrapping = TextWrapping.Wrap, FontWeight = o.Mine ? FontWeights.SemiBold : FontWeights.Normal,
            });
            Body.Children.Add(head);
            if (o.Votes is { } votes)
                Body.Children.Add(new ProgressBar { Maximum = max, Value = votes, Height = 6, Margin = new Thickness(0, 0, 0, 2) });
            if (o.Voters.Count > 0)
                Body.Children.Add(new TextBlock
                {
                    Text = string.Join("、", o.Voters), Foreground = Brushes2.Get("MutedBrush"), FontSize = 12, TextWrapping = TextWrapping.Wrap,
                });
        }
        if (poll.Options.All(o => o.Votes is null))
            Body.Children.Add(new TextBlock { Text = "這個投票設定投了才看得到結果。", Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 8, 0, 0) });
        Body.Children.Add(new TextBlock
        {
            Text = "要投票請到網頁（Hayaku 的 MCP 還不能投票）。", Foreground = Brushes2.Get("MutedBrush"), FontSize = 12, Margin = new Thickness(0, 12, 0, 8),
        });
    }
}
