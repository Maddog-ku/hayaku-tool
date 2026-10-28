using System.Runtime.InteropServices;

namespace HayakuBar.Infra;

internal static class Native
{
    public const int WS_CHILD = 0x40000000;
    public const int WS_VISIBLE = 0x10000000;
    public const int WS_CLIPSIBLINGS = 0x04000000;
    public const int WS_EX_LAYERED = 0x00080000;
    public const int WS_EX_TOOLWINDOW = 0x00000080;

    public const int WM_MOUSEMOVE = 0x0200;
    public const int WM_LBUTTONDOWN = 0x0201;
    public const int WM_LBUTTONUP = 0x0202;
    public const int WM_RBUTTONUP = 0x0205;
    public const int WM_MOUSELEAVE = 0x02A3;
    public const int WM_CAPTURECHANGED = 0x0215;

    public const uint SWP_NOSIZE = 0x0001;
    public const uint SWP_NOMOVE = 0x0002;
    public const uint SWP_NOACTIVATE = 0x0010;
    public const uint SWP_SHOWWINDOW = 0x0040;
    public static readonly IntPtr HWND_TOP = IntPtr.Zero;
    public static readonly IntPtr HWND_TOPMOST = new(-1);

    public const int ULW_ALPHA = 0x02;
    public const byte AC_SRC_OVER = 0x00;
    public const byte AC_SRC_ALPHA = 0x01;
    public const uint TME_LEAVE = 0x02;

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT
    {
        public int Left, Top, Right, Bottom;
        public readonly int Width => Right - Left;
        public readonly int Height => Bottom - Top;
        public readonly bool IsEmpty => Width <= 0 || Height <= 0;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X, Y; }

    [StructLayout(LayoutKind.Sequential)]
    public struct SIZE { public int Cx, Cy; }

    [StructLayout(LayoutKind.Sequential, Pack = 1)]
    public struct BLENDFUNCTION
    {
        public byte BlendOp, BlendFlags, SourceConstantAlpha, AlphaFormat;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct TRACKMOUSEEVENT
    {
        public int cbSize;
        public uint dwFlags;
        public IntPtr hwndTrack;
        public uint dwHoverTime;
    }

    /// <summary>
    /// 使用者現在是否不方便被打擾：全螢幕程式、簡報模式、Windows 的安靜時段、螢幕鎖定。
    /// （SHQueryUserNotificationState：1 不在電腦前、2 全螢幕、3 全螢幕 D3D、4 簡報、5 可以通知、6 安靜時段、7 全螢幕的市集 app）
    /// </summary>
    public static bool UserBusy()
    {
        try
        {
            return SHQueryUserNotificationState(out var state) == 0 && state is 1 or 2 or 3 or 4 or 6 or 7;
        }
        catch
        {
            return false;
        }
    }

    [DllImport("shell32.dll")]
    static extern int SHQueryUserNotificationState(out int state);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string? lpClassName, string? lpWindowName);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr childAfter, string? lpClassName, string? lpWindowName);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr insertAfter, int x, int y, int cx, int cy, uint flags);

    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool GetCursorPos(out POINT point);

    [DllImport("user32.dll")]
    public static extern IntPtr SetCapture(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool ReleaseCapture();

    [DllImport("user32.dll")]
    public static extern bool TrackMouseEvent(ref TRACKMOUSEEVENT tme);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool UpdateLayeredWindow(IntPtr hwnd, IntPtr hdcDst, IntPtr pptDst, ref SIZE psize,
        IntPtr hdcSrc, ref POINT pptSrc, int crKey, ref BLENDFUNCTION pblend, int dwFlags);

    [DllImport("user32.dll")]
    public static extern IntPtr GetDC(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);

    [DllImport("gdi32.dll")]
    public static extern IntPtr CreateCompatibleDC(IntPtr hDC);

    [DllImport("gdi32.dll")]
    public static extern bool DeleteDC(IntPtr hdc);

    [DllImport("gdi32.dll")]
    public static extern IntPtr SelectObject(IntPtr hDC, IntPtr hObject);

    [DllImport("gdi32.dll")]
    public static extern bool DeleteObject(IntPtr hObject);
}
