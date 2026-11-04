using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// 測試結果批次推到下一步的小視窗（例〔已更新到測試機〕：待更新測試機 → 待復驗，通知回報的人復驗）。
/// 可以填更新說明（例版本號），會變成每筆的留言、也放進通知。tasks 為 null 是專案裡那個子狀態的全部。
/// </summary>
public sealed class AdvanceTestsWindow : SmallDialog
{
    /// <summary>做完了（成功）：參數是伺服器的說明。</summary>
    public event Action<string>? Done;

    public AdvanceTestsWindow(TimerEngine engine, string project, TestSubStatus from, IReadOnlyList<string>? tasks)
        : base(from.BatchText ?? "推到下一步")
    {
        var what = tasks switch
        {
            null => $"{project} 所有「{from.Name}」的測試結果",
            { Count: 1 } => $"{tasks[0]}（{from.Name}）",
            _ => $"{project}「{from.Name}」的 {tasks.Count} 筆測試結果",
        };
        Body.Children.Add(new TextBlock
        {
            Text = $"{from.BatchText}：{what}", FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 6),
        });
        if (tasks is { Count: > 1 and <= 12 })
            Body.Children.Add(new TextBlock
            {
                Text = string.Join("、", tasks), Foreground = Brushes2.Get("MutedBrush"), TextWrapping = TextWrapping.Wrap,
                Margin = new Thickness(0, 0, 0, 6),
            });
        Body.Children.Add(new TextBlock
        {
            Text = "推到下一步（回到待復驗的會通知回報的人復驗）。", Foreground = Brushes2.Get("MutedBrush"), TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
        });
        var comment = new TextBox { Style = (Style)FindResource("HintBox"), Tag = "例 測試機 1.3.0-rc2（可以不填）", MaxLength = 200, Margin = new Thickness(0, 0, 0, 12) };
        Body.Children.Add(Label("更新說明（會變成每筆的留言）"));
        Body.Children.Add(comment);
        AddButton(from.BatchText ?? "確定", primary: true,
            () => RunAsync(async () => Done?.Invoke(await engine.AdvanceTestResultsAsync(project, from, tasks, comment.Text))));
        AddButton("取消", primary: false, cancel: true);
        Loaded += (_, _) => comment.Focus();
    }
}
