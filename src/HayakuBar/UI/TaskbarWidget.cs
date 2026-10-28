using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Drawing.Text;
using System.Windows.Threading;
using HayakuBar.Infra;
using Microsoft.Win32;
using static HayakuBar.Infra.Native;
using Drawing = System.Drawing;
using WinForms = System.Windows.Forms;

namespace HayakuBar.UI;

public enum TaskbarAnchor { Left, BeforeStart, AfterIcons, BeforeTray }

public enum WidgetStatus { Idle, Running, Error }

/// <summary>工作列上的顯示方式：一排（寬）或兩排（窄）。</summary>
public enum WidgetLayout { OneLine, TwoLine }

public sealed record WidgetContent(string Total, string Expected, double Progress, int Running, bool Claude, WidgetStatus Status);

/// <summary>
/// 嵌進 Windows 工作列（Shell_TrayWnd 的子視窗）的工時顯示。用 UpdateLayeredWindow 畫逐像素透明的內容，
/// 背景跟著工作列的顏色與透明效果。可選基準位置、拖曳左右微調，會自動避開工作列圖示。
/// explorer 重啟或工作列重建時會自動重新嵌入。
/// </summary>
internal sealed class TaskbarWidget : IDisposable
{
    readonly HostWindow _host;
    readonly TaskbarIconScanner _icons = new();
    readonly DispatcherTimer _watch = new() { Interval = TimeSpan.FromSeconds(2) };
    IntPtr _tray;
    WidgetContent _content = new("—", "", 0, 0, false, WidgetStatus.Idle);
    double _scale = 1;
    int _width;
    int _height;
    bool _hover;
    bool _down;
    bool _dragging;
    int _downX;
    int _downOffsetPx;
    int _dragOffsetPx;
    string? _lastLayout;

    public TaskbarWidget(TaskbarAnchor anchor, double offset, WidgetLayout layout)
    {
        Anchor = anchor;
        Layout = layout;
        Offset = offset;
        _host = new HostWindow(this);
        _watch.Tick += (_, _) => Watch();
    }

    public TaskbarAnchor Anchor { get; set; }

    public WidgetLayout Layout { get; set; }

    /// <summary>拖曳微調的偏移量（DIP，正數往右）。</summary>
    public double Offset { get; set; }

    public bool IsEmbedded => _host.Handle != IntPtr.Zero;

    /// <summary>目前在螢幕上的位置（實體像素）。</summary>
    public RECT ScreenRect { get; private set; }

    /// <summary>工作列的位置（實體像素）。</summary>
    public RECT TaskbarRect { get; private set; }

    public double Scale => _scale;

    public event Action? Clicked;
    public event Action<Drawing.Point>? MenuRequested;
    public event Action<double>? OffsetCommitted;

    public void Start()
    {
        _icons.ScanInBackground();
        Embed();
        _watch.Start();
    }

    public void Update(WidgetContent content)
    {
        if (content == _content)
            return;
        _content = content;
        if (IsEmbedded)
        {
            Render();
            Place();
        }
    }

    /// <summary>位置或顯示方式改了之後呼叫。</summary>
    public void Relayout()
    {
        if (!IsEmbedded)
            return;
        Render();
        Place();
    }

    void Watch()
    {
        try
        {
            var tray = FindWindow("Shell_TrayWnd", null);
            if (tray == IntPtr.Zero)
                return;
            if (tray != _tray || !IsEmbedded || !IsWindow(_host.Handle))
            {
                Destroy();
                Embed();
                return;
            }
            _icons.ScanInBackground();
            var dpiScale = GetDpiForWindow(_tray) / 96.0;
            GetWindowRect(_tray, out var rect);
            if (Math.Abs(dpiScale - _scale) > 0.001 || rect.Height != _height)
                Render();
            Place();
        }
        catch (Exception ex)
        {
            Log.Error("工作列小工具更新失敗", ex);
        }
    }

    void Embed()
    {
        _tray = FindWindow("Shell_TrayWnd", null);
        if (_tray == IntPtr.Zero)
            return;
        GetWindowRect(_tray, out var trayRect);
        var cp = new WinForms.CreateParams
        {
            Caption = "HayakuBar",
            Parent = _tray,
            Style = WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            ExStyle = WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            X = 0,
            Y = 0,
            Width = 1,
            Height = Math.Max(1, trayRect.Height),
        };
        try
        {
            _host.CreateHandle(cp);
            Render();
            Place();
            Log.Info("已嵌入工作列");
        }
        catch (Exception ex)
        {
            Log.Error("嵌入工作列失敗", ex);
            Destroy();
        }
    }

    void Destroy()
    {
        try
        {
            if (_host.Handle != IntPtr.Zero)
                _host.DestroyHandle();
        }
        catch (Exception ex)
        {
            Log.Error("移除工作列小工具失敗", ex);
        }
    }

    int OffsetPx => _dragging ? _dragOffsetPx : (int)Math.Round(Offset * _scale);

    void Place()
    {
        GetWindowRect(_tray, out var tray);
        TaskbarRect = tray;
        var margin = (int)(8 * _scale);
        var trayLeft = ChildLeft("TrayNotifyWnd") is { } notify ? notify - tray.Left : tray.Width;
        int? iconsLeft = null, iconsRight = null;
        if (_icons.Range is { } icons)
            (iconsLeft, iconsRight) = (icons.Left - tray.Left, icons.Right - tray.Left);
        else if (ChildLeft("Start") is { } start)
            iconsLeft = start - tray.Left;

        var beforeIcons = (iconsLeft ?? 2 * margin + _width) - _width - margin;
        var afterIcons = iconsRight is { } ir ? ir + margin : trayLeft - _width - margin;
        var x = Anchor switch
        {
            TaskbarAnchor.Left => margin,
            TaskbarAnchor.BeforeStart => beforeIcons,
            TaskbarAnchor.AfterIcons => afterIcons,
            _ => trayLeft - _width - margin,
        } + OffsetPx;

        // 疊到圖示或通知區域時（例如開了很多 app），改放到最近一個放得下的位置
        bool Overlaps(int pos) =>
            (iconsLeft is { } l && iconsRight is { } r && pos < r && pos + _width > l) || pos + _width > trayLeft;
        if (Overlaps(x))
        {
            var candidates = new[] { beforeIcons, afterIcons, margin }
                .Where(c => c >= 0 && c + _width <= tray.Width && !Overlaps(c))
                .OrderBy(c => Math.Abs(c - x))
                .ToList();
            if (candidates.Count > 0)
                x = candidates[0];
        }
        x = Math.Clamp(x, 0, Math.Max(0, tray.Width - _width));
        var layoutKey = $"{Anchor} x={x} w={_width} icons={iconsLeft}..{iconsRight} tray={trayLeft} offset={OffsetPx}";
        if (layoutKey != _lastLayout && !_dragging)
        {
            _lastLayout = layoutKey;
            Log.Info($"工作列位置 {layoutKey}");
        }
        SetWindowPos(_host.Handle, HWND_TOP, x, 0, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        ScreenRect = new RECT { Left = tray.Left + x, Top = tray.Top, Right = tray.Left + x + _width, Bottom = tray.Top + _height };
    }

    int? ChildLeft(string className)
    {
        var child = FindWindowEx(_tray, IntPtr.Zero, className, null);
        return child != IntPtr.Zero && GetWindowRect(child, out var r) && !r.IsEmpty ? r.Left : null;
    }

    // ───────────── 繪製 ─────────────

    static bool DarkTaskbar()
    {
        using var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        return key?.GetValue("SystemUsesLightTheme") is not int light || light == 0;
    }

    void Render()
    {
        _scale = GetDpiForWindow(_tray) / 96.0;
        if (_scale <= 0)
            _scale = 1;
        GetWindowRect(_tray, out var tray);
        _height = Math.Max(1, tray.Height);
        var s = _scale;
        var dark = DarkTaskbar();
        var text = dark ? Drawing.Color.FromArgb(245, 255, 255, 255) : Drawing.Color.FromArgb(235, 0, 0, 0);
        var muted = dark ? Drawing.Color.FromArgb(150, 255, 255, 255) : Drawing.Color.FromArgb(140, 0, 0, 0);
        var track = dark ? Drawing.Color.FromArgb(70, 255, 255, 255) : Drawing.Color.FromArgb(50, 0, 0, 0);
        var accent = Drawing.Color.FromArgb(255, 232, 118, 64);
        var run = Drawing.Color.FromArgb(255, 63, 185, 132);
        var claude = Drawing.Color.FromArgb(255, 217, 119, 87);
        var error = Drawing.Color.FromArgb(255, 232, 84, 72);

        var twoLine = Layout == WidgetLayout.TwoLine;
        using var bold = new Drawing.Font("Microsoft JhengHei UI", (float)((twoLine ? 12.5 : 13) * s), Drawing.FontStyle.Bold, Drawing.GraphicsUnit.Pixel);
        using var regular = new Drawing.Font("Microsoft JhengHei UI", (float)((twoLine ? 12 : 13) * s), Drawing.FontStyle.Regular, Drawing.GraphicsUnit.Pixel);
        using var small = new Drawing.Font("Segoe UI Symbol", (float)((twoLine ? 10 : 11.5) * s), Drawing.FontStyle.Regular, Drawing.GraphicsUnit.Pixel);
        var fmt = Drawing.StringFormat.GenericTypographic;

        float pad = (float)(8 * s), dot = (float)((twoLine ? 6 : 8) * s), gap = (float)((twoLine ? 5 : 7) * s);
        float barH = (float)((twoLine ? 3 : 4) * s), minBar = (float)(36 * s), oneLineBar = (float)(44 * s);
        var runText = _content.Running > 0 ? $"●{_content.Running}" : "";
        var claudeText = _content.Claude ? "✦" : "";
        Drawing.SizeF totalSize, expectedSize, runSize, claudeSize;
        using (var probe = new Drawing.Bitmap(1, 1))
        using (var pg = Drawing.Graphics.FromImage(probe))
        {
            pg.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
            totalSize = pg.MeasureString(_content.Total, bold, int.MaxValue, fmt);
            expectedSize = _content.Expected.Length > 0 ? pg.MeasureString(" " + _content.Expected, regular, int.MaxValue, fmt) : Drawing.SizeF.Empty;
            runSize = runText.Length > 0 ? pg.MeasureString(runText, small, int.MaxValue, fmt) : Drawing.SizeF.Empty;
            claudeSize = claudeText.Length > 0 ? pg.MeasureString(claudeText, small, int.MaxValue, fmt) : Drawing.SizeF.Empty;
        }
        var line1 = dot + gap + totalSize.Width + expectedSize.Width;
        var extras = (runSize.Width > 0 ? gap + runSize.Width : 0) + (claudeSize.Width > 0 ? gap * 0.6f + claudeSize.Width : 0);
        float barW, contentWidth;
        if (twoLine)
        {
            // 兩排：上面「3.4h / 8h」，下面進度條＋計時數＋Claude
            contentWidth = Math.Max(line1, minBar + extras);
            barW = contentWidth - extras;
        }
        else
        {
            // 一排：● 3.4h / 8h ▬▬▬ ●6 ✦
            barW = oneLineBar;
            contentWidth = line1 + gap + barW + extras;
        }
        _width = (int)Math.Ceiling(pad + contentWidth + pad);

        using var bmp = new Drawing.Bitmap(_width, _height, PixelFormat.Format32bppArgb);
        using (var g = Drawing.Graphics.FromImage(bmp))
        {
            g.SmoothingMode = SmoothingMode.AntiAlias;
            g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
            // alpha 1 的底色：看不見，但讓整塊都點得到（alpha 0 的像素會穿透滑鼠）
            g.Clear(Drawing.Color.FromArgb(1, 0, 0, 0));
            if (_hover || _down)
            {
                var inset = (float)(4 * s);
                using var hoverBrush = new Drawing.SolidBrush(dark ? Drawing.Color.FromArgb(_down ? 22 : 34, 255, 255, 255) : Drawing.Color.FromArgb(_down ? 18 : 26, 0, 0, 0));
                using var hoverPath = RoundRect(new Drawing.RectangleF(1, inset, _width - 2, _height - inset * 2), (float)(4 * s));
                g.FillPath(hoverBrush, hoverPath);
            }

            float cy1, cy2;
            var line2H = Math.Max(barH, Math.Max(runSize.Height, claudeSize.Height));
            if (twoLine)
            {
                var lineGap = (float)(5 * s);
                var top = (_height - (totalSize.Height + lineGap + line2H)) / 2;
                cy1 = top + totalSize.Height / 2;
                cy2 = top + totalSize.Height + lineGap + line2H / 2;
            }
            else
            {
                cy1 = cy2 = _height / 2f;
            }

            // 狀態點＋工時
            var x = pad;
            var dotColor = _content.Status switch
            {
                WidgetStatus.Error => error,
                WidgetStatus.Running => run,
                _ => muted,
            };
            using (var b = new Drawing.SolidBrush(dotColor))
                g.FillEllipse(b, x, cy1 - dot / 2, dot, dot);
            x += dot + gap;
            using (var b = new Drawing.SolidBrush(text))
                g.DrawString(_content.Total, bold, b, x, cy1 - totalSize.Height / 2, fmt);
            x += totalSize.Width;
            if (expectedSize.Width > 0)
            {
                using var b = new Drawing.SolidBrush(muted);
                g.DrawString(" " + _content.Expected, regular, b, x, cy1 - expectedSize.Height / 2, fmt);
                x += expectedSize.Width;
            }

            // 進度條＋計時數＋Claude（兩排時換到第二行）
            x = twoLine ? pad : x + gap;
            using (var b = new Drawing.SolidBrush(track))
            using (var path = RoundRect(new Drawing.RectangleF(x, cy2 - barH / 2, barW, barH), barH / 2))
                g.FillPath(b, path);
            var fill = (float)(barW * Math.Clamp(_content.Progress, 0, 1));
            if (fill >= barH)
            {
                using var b = new Drawing.SolidBrush(accent);
                using var path = RoundRect(new Drawing.RectangleF(x, cy2 - barH / 2, fill, barH), barH / 2);
                g.FillPath(b, path);
            }
            x += barW;
            if (runSize.Width > 0)
            {
                x += gap;
                using var b = new Drawing.SolidBrush(run);
                g.DrawString(runText, small, b, x, cy2 - runSize.Height / 2, fmt);
                x += runSize.Width;
            }
            if (claudeSize.Width > 0)
            {
                x += gap * 0.6f;
                using var b = new Drawing.SolidBrush(claude);
                g.DrawString(claudeText, small, b, x, cy2 - claudeSize.Height / 2, fmt);
            }
        }
        Present(bmp);
    }

    void Present(Drawing.Bitmap bmp)
    {
        var screenDc = GetDC(IntPtr.Zero);
        var memDc = CreateCompatibleDC(screenDc);
        var hBitmap = bmp.GetHbitmap(Drawing.Color.FromArgb(0));
        var old = SelectObject(memDc, hBitmap);
        try
        {
            var size = new SIZE { Cx = bmp.Width, Cy = bmp.Height };
            var src = new POINT();
            var blend = new BLENDFUNCTION { BlendOp = AC_SRC_OVER, SourceConstantAlpha = 255, AlphaFormat = AC_SRC_ALPHA };
            if (!UpdateLayeredWindow(_host.Handle, screenDc, IntPtr.Zero, ref size, memDc, ref src, 0, ref blend, ULW_ALPHA))
                Log.Error($"UpdateLayeredWindow 失敗：{System.Runtime.InteropServices.Marshal.GetLastWin32Error()}");
        }
        finally
        {
            SelectObject(memDc, old);
            DeleteObject(hBitmap);
            DeleteDC(memDc);
            ReleaseDC(IntPtr.Zero, screenDc);
        }
    }

    static GraphicsPath RoundRect(Drawing.RectangleF r, float radius)
    {
        var path = new GraphicsPath();
        var d = Math.Min(radius * 2, Math.Min(r.Width, r.Height));
        if (d <= 0)
        {
            path.AddRectangle(r);
            return path;
        }
        path.AddArc(r.Left, r.Top, d, d, 180, 90);
        path.AddArc(r.Right - d, r.Top, d, d, 270, 90);
        path.AddArc(r.Right - d, r.Bottom - d, d, d, 0, 90);
        path.AddArc(r.Left, r.Bottom - d, d, d, 90, 90);
        path.CloseFigure();
        return path;
    }

    // ───────────── 滑鼠 ─────────────

    void OnMessage(ref WinForms.Message m)
    {
        switch (m.Msg)
        {
            case WM_MOUSEMOVE:
                if (!_hover)
                {
                    _hover = true;
                    var tme = new TRACKMOUSEEVENT
                    {
                        cbSize = System.Runtime.InteropServices.Marshal.SizeOf<TRACKMOUSEEVENT>(),
                        dwFlags = TME_LEAVE,
                        hwndTrack = _host.Handle,
                    };
                    TrackMouseEvent(ref tme);
                    Render();
                }
                if (_down && GetCursorPos(out var p))
                {
                    if (!_dragging && Math.Abs(p.X - _downX) > 4 * _scale)
                        _dragging = true;
                    if (_dragging)
                    {
                        _dragOffsetPx = _downOffsetPx + (p.X - _downX);
                        Place();
                    }
                }
                break;

            case WM_LBUTTONDOWN:
                if (GetCursorPos(out var down))
                {
                    _down = true;
                    _dragging = false;
                    _downX = down.X;
                    _downOffsetPx = (int)Math.Round(Offset * _scale);
                    SetCapture(_host.Handle);
                    Render();
                }
                break;

            case WM_LBUTTONUP:
                if (!_down)
                    break;
                var wasDragging = _dragging;
                if (wasDragging)
                {
                    Offset = _dragOffsetPx / _scale;
                    _dragging = false;
                }
                _down = false;
                ReleaseCapture();
                Render();
                Place();
                if (wasDragging)
                    OffsetCommitted?.Invoke(Offset);
                else
                    Clicked?.Invoke();
                break;

            case WM_CAPTURECHANGED:
                if (_down && m.LParam != _host.Handle)
                {
                    _down = false;
                    _dragging = false;
                    Render();
                    Place();
                }
                break;

            case WM_MOUSELEAVE:
                _hover = false;
                Render();
                break;

            case WM_RBUTTONUP:
                if (GetCursorPos(out var at))
                    MenuRequested?.Invoke(new Drawing.Point(at.X, at.Y));
                break;
        }
    }

    public void Dispose()
    {
        _watch.Stop();
        Destroy();
    }

    sealed class HostWindow(TaskbarWidget owner) : WinForms.NativeWindow
    {
        protected override void WndProc(ref WinForms.Message m)
        {
            try
            {
                owner.OnMessage(ref m);
            }
            catch (Exception ex)
            {
                Log.Error("工作列小工具訊息處理失敗", ex);
            }
            base.WndProc(ref m);
        }
    }
}
