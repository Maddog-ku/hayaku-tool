using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using HayakuBar.Infra;
using Microsoft.Win32;

namespace HayakuBar.UI;

/// <summary>
/// 淺色／深色主題：跟隨 Windows 的「應用程式模式」，或使用者指定。切換時換掉 App.xaml 合併的主題檔，
/// 用 DynamicResource 的畫面會立即更新，也會把視窗標題列切成對應的深淺色。
/// </summary>
public static class ThemeManager
{
    const int DWMWA_USE_IMMERSIVE_DARK_MODE = 20;
    static bool _applied;

    /// <summary>System（跟隨 Windows）、Light、Dark。</summary>
    public static string Mode { get; private set; } = "System";

    public static bool IsDark { get; private set; }

    public static event Action? Changed;

    public static void Initialize(string mode)
    {
        Mode = Normalize(mode);
        SystemEvents.UserPreferenceChanged += (_, e) =>
        {
            if (Mode == "System" && e.Category is UserPreferenceCategory.General or UserPreferenceCategory.Color or UserPreferenceCategory.VisualStyle)
                Application.Current?.Dispatcher.InvokeAsync(Refresh);
        };
        Refresh();
    }

    public static void SetMode(string mode)
    {
        Mode = Normalize(mode);
        Refresh();
    }

    static string Normalize(string? mode) => mode is "Light" or "Dark" ? mode : "System";

    static bool SystemAppsDark()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            return key?.GetValue("AppsUseLightTheme") is int light && light == 0;
        }
        catch
        {
            return false;
        }
    }

    static void Refresh()
    {
        var dark = Mode switch
        {
            "Dark" => true,
            "Light" => false,
            _ => SystemAppsDark(),
        };
        if (_applied && dark == IsDark)
            return;
        IsDark = dark;
        _applied = true;

        var dictionaries = Application.Current.Resources.MergedDictionaries;
        var next = new ResourceDictionary
        {
            Source = new Uri($"pack://application:,,,/HayakuBar;component/Themes/{(dark ? "Dark" : "Light")}.xaml"),
        };
        var current = dictionaries.FirstOrDefault(d => d.Source?.OriginalString.Contains("Themes/") == true);
        if (current is null)
            dictionaries.Insert(0, next);
        else
            dictionaries[dictionaries.IndexOf(current)] = next;

        foreach (Window window in Application.Current.Windows)
            ApplyTitleBar(window);
        Log.Info($"主題：{(dark ? "深色" : "淺色")}（{Mode}）");
        Changed?.Invoke();
    }

    /// <summary>標題列跟著深淺色（Windows 11／10 20H1 以上）。在視窗 SourceInitialized 之後呼叫。</summary>
    public static void ApplyTitleBar(Window window)
    {
        var hwnd = new WindowInteropHelper(window).Handle;
        if (hwnd == IntPtr.Zero)
            return;
        var value = IsDark ? 1 : 0;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, ref value, sizeof(int));
    }

    [DllImport("dwmapi.dll")]
    static extern int DwmSetWindowAttribute(IntPtr hwnd, int attribute, ref int value, int size);
}
