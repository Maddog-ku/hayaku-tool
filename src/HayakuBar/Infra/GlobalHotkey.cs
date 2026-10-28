using System.Runtime.InteropServices;
using System.Windows.Input;
using System.Windows.Interop;

namespace HayakuBar.Infra;

/// <summary>全域快捷鍵（RegisterHotKey），格式例「Win+Shift+H」「Ctrl+Alt+F9」。</summary>
public sealed class GlobalHotkey : IDisposable
{
    const int WM_HOTKEY = 0x0312;
    const int HotkeyId = 0x4859;
    const uint MOD_ALT = 0x1, MOD_CONTROL = 0x2, MOD_SHIFT = 0x4, MOD_WIN = 0x8, MOD_NOREPEAT = 0x4000;

    readonly HwndSource _source;
    bool _registered;

    public GlobalHotkey()
    {
        _source = new HwndSource(new HwndSourceParameters("HayakuBarHotkey") { Width = 0, Height = 0, WindowStyle = 0 });
        _source.AddHook(WndProc);
    }

    public event Action? Pressed;

    /// <summary>註冊快捷鍵；空字串表示取消。被其他程式占用或格式不對時回傳 false。</summary>
    public bool Register(string? text)
    {
        Unregister();
        if (string.IsNullOrWhiteSpace(text))
            return true;
        if (!TryParse(text, out var modifiers, out var vk))
            return false;
        _registered = RegisterHotKey(_source.Handle, HotkeyId, modifiers | MOD_NOREPEAT, vk);
        if (!_registered)
            Log.Error($"快捷鍵 {text} 註冊失敗（可能被其他程式占用）");
        return _registered;
    }

    void Unregister()
    {
        if (_registered)
            UnregisterHotKey(_source.Handle, HotkeyId);
        _registered = false;
    }

    public static bool TryParse(string text, out uint modifiers, out uint vk)
    {
        modifiers = 0;
        vk = 0;
        Key? key = null;
        foreach (var raw in text.Split('+', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            switch (raw.ToLowerInvariant())
            {
                case "win": modifiers |= MOD_WIN; break;
                case "ctrl": case "control": modifiers |= MOD_CONTROL; break;
                case "shift": modifiers |= MOD_SHIFT; break;
                case "alt": modifiers |= MOD_ALT; break;
                default:
                    if (!Enum.TryParse<Key>(raw.Length == 1 && char.IsDigit(raw[0]) ? "D" + raw : raw, ignoreCase: true, out var parsed))
                        return false;
                    key = parsed;
                    break;
            }
        }
        if (key is null || modifiers == 0)
            return false;
        vk = (uint)KeyInterop.VirtualKeyFromKey(key.Value);
        return vk != 0;
    }

    /// <summary>把按下的組合鍵轉成文字；只有修飾鍵、或沒有修飾鍵時回傳 null。</summary>
    public static string? Format(ModifierKeys modifiers, bool win, Key key)
    {
        if (key is Key.LeftCtrl or Key.RightCtrl or Key.LeftShift or Key.RightShift or Key.LeftAlt or Key.RightAlt
            or Key.LWin or Key.RWin or Key.System)
            return null;
        var parts = new List<string>();
        if (win) parts.Add("Win");
        if (modifiers.HasFlag(ModifierKeys.Control)) parts.Add("Ctrl");
        if (modifiers.HasFlag(ModifierKeys.Alt)) parts.Add("Alt");
        if (modifiers.HasFlag(ModifierKeys.Shift)) parts.Add("Shift");
        if (parts.Count == 0)
            return null;
        var name = key is >= Key.D0 and <= Key.D9 ? ((int)(key - Key.D0)).ToString() : key.ToString();
        parts.Add(name);
        return string.Join("+", parts);
    }

    IntPtr WndProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == WM_HOTKEY && wParam.ToInt32() == HotkeyId)
        {
            handled = true;
            Pressed?.Invoke();
        }
        return IntPtr.Zero;
    }

    public void Dispose()
    {
        Unregister();
        _source.Dispose();
    }

    [DllImport("user32.dll", SetLastError = true)]
    static extern bool RegisterHotKey(IntPtr hWnd, int id, uint modifiers, uint vk);

    [DllImport("user32.dll")]
    static extern bool UnregisterHotKey(IntPtr hWnd, int id);
}
