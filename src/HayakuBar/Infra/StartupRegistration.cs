using Microsoft.Win32;

namespace HayakuBar.Infra;

/// <summary>開機自動啟動：寫在 HKCU\...\Run，不需要系統管理員權限。</summary>
public static class StartupRegistration
{
    const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    const string Name = "HayakuBar";

    public static bool IsEnabled()
    {
        using var key = Registry.CurrentUser.OpenSubKey(RunKey);
        return key?.GetValue(Name) is string;
    }

    public static void Set(bool enabled)
    {
        using var key = Registry.CurrentUser.CreateSubKey(RunKey);
        if (enabled)
            key.SetValue(Name, $"\"{Environment.ProcessPath}\"");
        else
            key.DeleteValue(Name, throwOnMissingValue: false);
    }
}
