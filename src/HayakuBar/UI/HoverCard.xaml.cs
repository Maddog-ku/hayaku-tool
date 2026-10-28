using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using HayakuBar.Core;

namespace HayakuBar.UI;

/// <summary>滑鼠停在工作列的工時上時，顯示今天各任務的明細（不接收滑鼠、不搶焦點）。</summary>
public partial class HoverCard : Window
{
    Rect _anchor;
    bool _alignLeft;

    public HoverCard()
    {
        InitializeComponent();
        SourceInitialized += (_, _) =>
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            SetWindowLong(hwnd, -20, GetWindowLong(hwnd, -20) | 0x08000000 | 0x00000080 | 0x00000020); // NOACTIVATE | TOOLWINDOW | TRANSPARENT
            ThemeManager.ApplyTitleBar(this);
        };
        SizeChanged += (_, _) => Reposition();
    }

    public void ShowFor(TimerEngine engine, Rect anchor, bool alignLeft)
    {
        _anchor = anchor;
        _alignLeft = alignLeft;
        var now = DateTime.Now;
        var worked = WorkMath.UnionMinutes(engine.Entries, now.Date, now);
        var expected = engine.Schedule?.ExpectedMinutes ?? 0;
        HeaderText.Text = $"今天 {WorkMath.HoursMinutes(worked)}" + (expected > 0 ? $" / {WorkMath.HoursMinutes(expected)}" : "");
        var phase = WorkMath.Phase(engine.Schedule, now, engine.State.ClockedOut).Text;
        var parts = new List<string>();
        if (phase.Length > 0)
            parts.Add(phase);
        if (expected > 0 && worked < expected)
            parts.Add($"還差 {WorkMath.HoursMinutes(expected - worked)}");
        if (engine.UnreadCount > 0)
            parts.Add($"{engine.UnreadCount} 則未讀通知");
        SubText.Text = string.Join(" · ", parts);
        Timeline.SetData(now.Date, engine.Schedule?.Working, engine.Entries, now);

        var running = engine.RunningKeys();
        Rows.ItemsSource = engine.Entries
            .GroupBy(e => e.Task, StringComparer.OrdinalIgnoreCase)
            .Select(g => (Group: g, Minutes: WorkMath.TaskMinutes(g, g.Key, now.Date, now), Running: running.Contains(g.Key)))
            .OrderByDescending(x => x.Running)
            .ThenByDescending(x => x.Minutes)
            .Take(8)
            .Select(x => new TaskRowVm
            {
                Key = x.Group.Key,
                Title = x.Group.Last().Title,
                Running = x.Running,
                IsClaude = engine.IsClaudeOwned(x.Group.Key),
                Elapsed = WorkMath.Clock(x.Minutes),
            })
            .ToList();
        Show();
        Reposition();
    }

    void Reposition()
    {
        var work = SystemParameters.WorkArea;
        var left = _alignLeft ? _anchor.Left - 6 : _anchor.Right - ActualWidth + 6;
        Left = Math.Clamp(left, work.Left, Math.Max(work.Left, work.Right - ActualWidth));
        Top = Math.Max(work.Top, Math.Min(_anchor.Top, work.Bottom) - ActualHeight - 2);
    }

    [DllImport("user32.dll")]
    static extern int GetWindowLong(IntPtr hWnd, int index);

    [DllImport("user32.dll")]
    static extern int SetWindowLong(IntPtr hWnd, int index, int value);
}
