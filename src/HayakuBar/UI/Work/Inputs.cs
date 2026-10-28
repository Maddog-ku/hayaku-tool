using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using HayakuBar.Infra;

namespace HayakuBar.UI.Work;

/// <summary>工作台各頁：切到這頁時呼叫 ActivateAsync（arg 例如任務編號、日期）。</summary>
public interface IWorkPage
{
    Task ActivateAsync(object? arg);
}

/// <summary>日期、時間輸入的寬鬆解析與顯示。</summary>
public static class Inputs
{
    public static readonly CultureInfo Zh = new("zh-TW");

    static readonly string[] DateFormats = ["yyyy-MM-dd", "yyyy/M/d", "yyyy-M-d", "M/d", "M-d", "MMdd", "yyyyMMdd"];
    static readonly string[] TimeFormats = ["H:mm", "HH:mm", "Hmm", "HHmm", "%H"];

    /// <summary>今天、明天、後天、+3、11/5、2026-11-05 …；空白回 null。</summary>
    public static DateOnly? ParseDate(string? text, DateOnly? today = null)
    {
        var t = text?.Trim() ?? "";
        var baseDay = today ?? DateOnly.FromDateTime(DateTime.Today);
        if (t.Length == 0)
            return null;
        switch (t)
        {
            case "今天" or "今日":
                return baseDay;
            case "明天" or "明日":
                return baseDay.AddDays(1);
            case "後天":
                return baseDay.AddDays(2);
            case "昨天":
                return baseDay.AddDays(-1);
        }
        if ((t.StartsWith('+') || t.StartsWith('-')) && int.TryParse(t, out var offset))
            return baseDay.AddDays(offset);
        if (DateOnly.TryParseExact(t, DateFormats, CultureInfo.InvariantCulture, DateTimeStyles.None, out var d))
        {
            // 只打月日：年份用今年；已經過了很久（超過半年）就當明年
            if (!t.Contains(baseDay.Year.ToString()) && t.Length <= 5 && d.Year == DateTime.Today.Year)
            {
                d = new DateOnly(baseDay.Year, d.Month, d.Day);
                if (d < baseDay.AddMonths(-6))
                    d = d.AddYears(1);
            }
            return d;
        }
        return null;
    }

    public static TimeOnly? ParseTime(string? text) =>
        TimeOnly.TryParseExact(text?.Trim() ?? "", TimeFormats, CultureInfo.InvariantCulture, DateTimeStyles.None, out var t) ? t : null;

    public static string DateText(DateOnly? d) => d is { } day ? day.ToString("yyyy-MM-dd") : "";

    public static string ShortDate(DateOnly d) =>
        d == DateOnly.FromDateTime(DateTime.Today) ? "今天" :
        d == DateOnly.FromDateTime(DateTime.Today).AddDays(1) ? "明天" :
        d.Year == DateTime.Today.Year ? d.ToString("M/d（ddd）", Zh) : d.ToString("yyyy/M/d", Zh);

    public static DateTimeOffset Local(DateOnly day, TimeOnly time)
    {
        var dt = day.ToDateTime(time);
        return new DateTimeOffset(dt, TimeZoneInfo.Local.GetUtcOffset(dt));
    }

    public static string Iso(DateTimeOffset t) => t.ToString("yyyy-MM-dd'T'HH:mm:sszzz");

    public static string Ago(DateTimeOffset time)
    {
        var span = DateTimeOffset.Now - time;
        return span.TotalMinutes < 1 ? "剛剛" : span.TotalHours < 1 ? $"{(int)span.TotalMinutes} 分鐘前" :
            span.TotalDays < 1 ? $"{(int)span.TotalHours} 小時前" :
            span.TotalDays < 7 ? $"{(int)span.TotalDays} 天前" : time.ToLocalTime().ToString("M/d HH:mm");
    }

    /// <summary>顯示錯誤或成功訊息（TextBlock 的 Tag 不用）。</summary>
    public static void Show(TextBlock target, string? message, bool error = true)
    {
        target.Text = message ?? "";
        target.Foreground = Brushes2.Get(error ? "ErrorBrush" : "RunBrush");
        target.Visibility = string.IsNullOrEmpty(message) ? Visibility.Collapsed : Visibility.Visible;
    }

    /// <summary>執行一個動作；按鈕先變成處理中，失敗時把訊息顯示在 status。</summary>
    public static async Task<bool> Run(Button? button, TextBlock status, Func<Task> action, string? success = null)
    {
        var content = button?.Content;
        if (button is not null)
        {
            button.IsEnabled = false;
            button.Content = "處理中…";
        }
        Show(status, null);
        try
        {
            await action();
            if (success is not null)
                Show(status, success, error: false);
            return true;
        }
        catch (Exception ex)
        {
            Log.Error("工作台動作失敗", ex);
            Show(status, ex is Mcp.McpException or HttpRequestException or TaskCanceledException ? ex.Message : $"發生錯誤：{ex.Message}");
            return false;
        }
        finally
        {
            if (button is not null)
            {
                button.Content = content;
                button.IsEnabled = true;
            }
        }
    }
}
