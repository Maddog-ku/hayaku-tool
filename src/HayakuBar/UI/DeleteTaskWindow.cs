using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.UI;

/// <summary>
/// 刪除任務的確認小視窗：任務詳細、工作台清單／樹狀／看板與專案頁的右鍵、面板的右鍵都用這個。
/// 刪除成功時跳提示，並馬上同步一次（清單跟著更新）。
/// </summary>
public sealed class DeleteTaskWindow : SmallDialog
{
    /// <summary>刪除成功了（呼叫端據此關掉詳細、重畫清單）。</summary>
    public bool Deleted { get; private set; }

    /// <param name="children">子任務數；不知道時傳 null。</param>
    public DeleteTaskWindow(TimerEngine engine, string key, string title, int? children)
        : base("刪除任務")
    {
        Body.Children.Add(new TextBlock
        {
            Text = $"刪除 {key} {title}？".Replace("  ", " "), FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 8),
        });
        var sub = children switch
        {
            > 0 => $"連同 {children} 個子任務一起刪除。",
            null => "有子任務的話會一起刪除。",
            _ => "",
        };
        Body.Children.Add(new TextBlock
        {
            Text = sub + "可以從網頁的垃圾桶還原；如果你是執行者，會送出刪除申請給管理者審核。",
            Foreground = Brushes2.Get("MutedBrush"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 14),
        });
        AddButton("刪除", primary: true, () => RunAsync(async () =>
        {
            var message = await engine.Api.DeleteTaskAsync(key, null, CancellationToken.None);
            Log.Info($"刪除任務 {key}：{message ?? "完成"}");
            engine.ShowNotice(message ?? $"已刪除 {key}（可以從網頁的垃圾桶還原）");
            Deleted = true;
        }));
        AddButton("取消", primary: false, cancel: true);
    }

    /// <summary>問過再刪；刪了就同步一次，回傳有沒有刪。</summary>
    public static async Task<bool> ConfirmAsync(Window? owner, TimerEngine engine, string key, string title, int? children)
    {
        var dialog = new DeleteTaskWindow(engine, key, title, children) { Owner = owner };
        dialog.ShowDialog();
        if (!dialog.Deleted)
            return false;
        await engine.AfterExternalChangeAsync();
        return true;
    }
}
