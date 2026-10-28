using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Threading;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.UI;

/// <summary>右下角跳出的提示：從右邊滑入，滑鼠移上去時不會自動關閉。</summary>
public partial class ToastWindow : Window
{
    readonly DispatcherTimer _timer = new();
    readonly bool _fromBottom;
    bool _closing;

    public ToastWindow(ToastRequest request)
    {
        InitializeComponent();
        TitleText.Text = request.Title;
        MessageText.Text = request.Message;
        MessageText.Visibility = string.IsNullOrWhiteSpace(request.Message) ? Visibility.Collapsed : Visibility.Visible;
        foreach (var action in request.Actions ?? [])
        {
            var button = new Button
            {
                Content = action.Label,
                Style = (Style)FindResource(action.Primary ? "PrimaryButton" : "FlatButton"),
                Margin = new Thickness(0, 0, 6, 0),
                Padding = new Thickness(12, 4, 12, 4),
            };
            button.Click += async (_, _) =>
            {
                button.IsEnabled = false;
                try
                {
                    await action.Run();
                }
                catch (Exception ex)
                {
                    Log.Error($"提示按鈕「{action.Label}」失敗", ex);
                }
                Dismiss();
            };
            ActionsPanel.Children.Add(button);
        }
        ActionsPanel.Visibility = ActionsPanel.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;

        _fromBottom = request.FromBottom;
        _timer.Interval = request.Sticky ? TimeSpan.FromMinutes(10)
            : request.Duration ?? TimeSpan.FromSeconds(request.Actions is { Count: > 0 } ? 15 : 7);
        _timer.Tick += (_, _) => Dismiss();
        MouseEnter += (_, _) => _timer.Stop();
        MouseLeave += (_, _) => _timer.Start();
        SourceInitialized += (_, _) => MakeNoActivate();
        Loaded += (_, _) =>
        {
            AnimateIn();
            _timer.Start();
        };
    }

    public event Action<ToastWindow>? Dismissed;

    /// <summary>從下面滑上來的（新通知）：擺在工作列工時的正上方。</summary>
    public bool FromBottom => _fromBottom;

    void Close_Click(object sender, RoutedEventArgs e) => Dismiss();

    public void Dismiss()
    {
        if (_closing)
            return;
        _closing = true;
        _timer.Stop();
        if (!SystemParameters.ClientAreaAnimation)
        {
            Close();
            Dismissed?.Invoke(this);
            return;
        }
        if (_fromBottom)
        {
            // 滑回下緣
            var down = new DoubleAnimation(Slide.Y, ActualHeight, TimeSpan.FromMilliseconds(240))
            {
                EasingFunction = new CubicEase { EasingMode = EasingMode.EaseIn },
            };
            down.Completed += (_, _) =>
            {
                Close();
                Dismissed?.Invoke(this);
            };
            Slide.BeginAnimation(TranslateTransform.YProperty, down);
            return;
        }
        var fade = new DoubleAnimation(1, 0, TimeSpan.FromMilliseconds(160));
        fade.Completed += (_, _) =>
        {
            Close();
            Dismissed?.Invoke(this);
        };
        Slide.BeginAnimation(TranslateTransform.XProperty, new DoubleAnimation(0, 30, TimeSpan.FromMilliseconds(160)));
        Root.BeginAnimation(OpacityProperty, fade);
    }

    void AnimateIn()
    {
        if (!SystemParameters.ClientAreaAnimation)
        {
            Root.Opacity = 1;
            return;
        }
        var ease = new CubicEase { EasingMode = EasingMode.EaseOut };
        if (_fromBottom)
        {
            // 從下緣滑上來（不淡入，看起來像從工作列冒出來）
            Root.Opacity = 1;
            Slide.BeginAnimation(TranslateTransform.YProperty,
                new DoubleAnimation(ActualHeight, 0, TimeSpan.FromMilliseconds(300)) { EasingFunction = ease });
            return;
        }
        Slide.BeginAnimation(TranslateTransform.XProperty, new DoubleAnimation(40, 0, TimeSpan.FromMilliseconds(260)) { EasingFunction = ease });
        Root.BeginAnimation(OpacityProperty, new DoubleAnimation(0, 1, TimeSpan.FromMilliseconds(200)));
    }

    /// <summary>點提示不搶走目前視窗的焦點。</summary>
    void MakeNoActivate()
    {
        var hwnd = new WindowInteropHelper(this).Handle;
        SetWindowLong(hwnd, GWL_EXSTYLE, GetWindowLong(hwnd, GWL_EXSTYLE) | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW);
    }

    const int GWL_EXSTYLE = -20;
    const int WS_EX_NOACTIVATE = 0x08000000;
    const int WS_EX_TOOLWINDOW = 0x00000080;

    [DllImport("user32.dll")]
    static extern int GetWindowLong(IntPtr hWnd, int index);

    [DllImport("user32.dll")]
    static extern int SetWindowLong(IntPtr hWnd, int index, int value);
}

/// <summary>管理同時顯示的提示：一般提示從右下角往上疊；新通知從工作列工時的位置往上疊；最多 4 則。</summary>
public sealed class ToastManager
{
    const int MaxVisible = 4;
    const double CardMargin = 8; // ToastWindow 外圈留給陰影的邊
    readonly List<ToastWindow> _open = [];

    /// <summary>工作列上工時的位置（DIP）與面板是否靠左展開；新通知從這裡冒出來。</summary>
    public Func<(Rect Anchor, bool AlignLeft)>? Anchor { get; set; }

    /// <summary>一般提示停留多久（設定的秒數）；null 表示不自動收回。</summary>
    public Func<TimeSpan?>? DefaultDuration { get; set; }

    /// <summary>現在要不要先別跳提示（全螢幕、簡報…）；回 true 時先存起來，之後再顯示。</summary>
    public Func<bool>? IsQuiet { get; set; }

    readonly List<ToastRequest> _held = [];
    DispatcherTimer? _heldTimer;

    public void Show(ToastRequest request)
    {
        if (!request.Important && IsQuiet?.Invoke() == true)
        {
            Hold(request);
            return;
        }
        ShowNow(request);
    }

    /// <summary>勿擾中：先存起來，每 5 秒看一次，結束後依序顯示（太多則只留最新的幾則）。</summary>
    void Hold(ToastRequest request)
    {
        _held.Add(request);
        if (_held.Count > MaxVisible)
            _held.RemoveAt(0);
        if (_heldTimer is not null)
            return;
        _heldTimer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(5) };
        _heldTimer.Tick += (_, _) =>
        {
            if (IsQuiet?.Invoke() == true)
                return;
            _heldTimer.Stop();
            _heldTimer = null;
            var pending = _held.ToList();
            _held.Clear();
            foreach (var r in pending)
                ShowNow(r);
        };
        _heldTimer.Start();
    }

    void ShowNow(ToastRequest request)
    {
        // 所有提示都一樣：從工作列工時的位置滑上來，停留設定的秒數後滑回去；
        // 要你處理的（會議快開始、提醒到了…）會停到你按按鈕或關掉
        if (!request.Sticky && request.Duration is null)
        {
            var duration = DefaultDuration?.Invoke();
            request = duration is null ? request with { Sticky = true } : request with { Duration = duration };
        }
        request = request with { FromBottom = true };
        var toast = new ToastWindow(request);
        toast.Dismissed += t =>
        {
            _open.Remove(t);
            Layout();
        };
        toast.SizeChanged += (_, _) => Layout();
        _open.Add(toast);
        while (_open.Count > MaxVisible)
        {
            var oldest = _open[0];
            _open.RemoveAt(0); // 先移出清單：Dismiss 有動畫，不會馬上觸發 Dismissed
            oldest.Dismiss();
        }
        toast.Show();
        Layout();
    }

    void Layout()
    {
        var work = SystemParameters.WorkArea;
        var anchor = Anchor?.Invoke();

        // 一般提示：右下角往上疊
        var bottom = work.Bottom - 4;
        foreach (var t in Enumerable.Reverse(_open).Where(t => !t.FromBottom || anchor is null))
        {
            var height = t.ActualHeight > 0 ? t.ActualHeight : 120;
            t.Left = work.Right - t.Width - 4;
            t.Top = bottom - height;
            bottom -= height - 4;
        }

        // 新通知：工作列工時的正上方往上疊，跟面板對齊同一邊
        if (anchor is not { } a)
            return;
        bottom = Math.Min(a.Anchor.Top, work.Bottom);
        foreach (var t in Enumerable.Reverse(_open).Where(t => t.FromBottom))
        {
            var height = t.ActualHeight > 0 ? t.ActualHeight : 120;
            var left = a.AlignLeft ? a.Anchor.Left - CardMargin : a.Anchor.Right - t.Width + CardMargin;
            t.Left = Math.Clamp(left, work.Left, Math.Max(work.Left, work.Right - t.Width));
            t.Top = bottom - height;
            bottom -= height - CardMargin;
        }
    }
}
