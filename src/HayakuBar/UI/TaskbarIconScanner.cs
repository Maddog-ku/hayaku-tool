using System.Windows.Automation;
using HayakuBar.Infra;

namespace HayakuBar.UI;

/// <summary>工作列上「開始＋搜尋＋app 圖示」那一整群的範圍（實體像素，螢幕座標）。</summary>
internal sealed record IconRange(int Left, int Right);

/// <summary>
/// 用 UI Automation 讀 Win11 工作列圖示的位置，讓工時避開圖示。
/// 在背景執行緒查詢：工時視窗是工作列的子視窗，UI 執行緒跨程序等 explorer 有卡住工作列的風險。
/// </summary>
internal sealed class TaskbarIconScanner
{
    volatile IconRange? _range;
    int _busy;

    public IconRange? Range => _range;

    public void ScanInBackground()
    {
        if (Interlocked.Exchange(ref _busy, 1) == 1)
            return;
        Task.Run(() =>
        {
            try
            {
                _range = Scan();
            }
            catch (Exception ex)
            {
                Log.Error("讀取工作列圖示位置失敗", ex);
            }
            finally
            {
                Volatile.Write(ref _busy, 0);
            }
        });
    }

    static IconRange? Scan()
    {
        var tray = AutomationElement.RootElement.FindFirst(TreeScope.Children,
            new PropertyCondition(AutomationElement.ClassNameProperty, "Shell_TrayWnd"));
        if (tray is null)
            return null;
        var frame = tray.FindFirst(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.ClassNameProperty, "Taskbar.TaskbarFrameAutomationPeer"));
        if (frame is null)
            return null; // 不是 Win11 的新工作列

        var rects = new List<(double Left, double Right)>();
        foreach (AutomationElement child in frame.FindAll(TreeScope.Children, Condition.TrueCondition))
        {
            var r = child.Current.BoundingRectangle;
            if (!r.IsEmpty && r.Width > 0 && r.Height > 0)
                rects.Add((r.Left, r.Right));
        }
        if (rects.Count == 0)
            return null;

        // 依間距分群，取最寬的一群（圖示群）；左邊的小工具（天氣）之類會被分開
        rects.Sort((a, b) => a.Left.CompareTo(b.Left));
        var best = (Left: rects[0].Left, Right: rects[0].Right);
        var current = best;
        foreach (var (left, right) in rects.Skip(1))
        {
            if (left - current.Right <= 24)
            {
                current.Right = Math.Max(current.Right, right);
            }
            else
            {
                current = (left, right);
            }
            if (current.Right - current.Left > best.Right - best.Left)
                best = current;
        }
        return new IconRange((int)Math.Floor(best.Left), (int)Math.Ceiling(best.Right));
    }
}
