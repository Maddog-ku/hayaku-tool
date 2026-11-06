using System.Windows;
using System.Windows.Documents;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>清單上任務名稱的寫法：測試任務、測試結果前面加類型圖示，退回過的測試結果標第幾輪（第 3 輪起加 🔥）。</summary>
public static class TaskText
{
    public static string Title(string title, TestInfo test)
    {
        var icon = TaskKinds.Icon(test.Kind);
        var text = icon.Length > 0 ? $"{icon} {title}" : title;
        if (test.Kind == TaskKind.TestResult && test.Round >= 2)
            text += test.Hot ? $"（第 {test.Round} 輪 🔥）" : $"（第 {test.Round} 輪）";
        return text;
    }

    public static string Title(TaskSummary t) => Title(t.Title, t.Test);
}

/// <summary>
/// 點任務編號是複製編號（跟網頁一樣），點名稱才打開任務：編號的 Run／TextBlock 設 Tag="copykey"，
/// 清單的點擊處理先問 IsTarget。
/// </summary>
public static class KeyCopy
{
    public const string Tag = "copykey";

    public static bool IsTarget(object? source) => source switch
    {
        FrameworkContentElement { Tag: Tag } => true,
        FrameworkElement { Tag: Tag } => true,
        _ => false,
    };

    public static void Copy(TimerEngine engine, string key) => Copy(engine, key, $"已複製 {key}");

    /// <summary>複製任務在網頁上的連結（跟〔在網頁開啟〕同一個網址）。</summary>
    public static void CopyLink(TimerEngine engine, string key) => Copy(engine, engine.TaskUrl(key), $"已複製 {key} 的連結");

    static void Copy(TimerEngine engine, string text, string notice)
    {
        try
        {
            Clipboard.SetText(text);
            engine.ShowNotice(notice);
        }
        catch (Exception ex) when (ex is System.Runtime.InteropServices.COMException or System.Runtime.InteropServices.ExternalException)
        {
            engine.ShowNotice("複製失敗，剪貼簿被別的程式占用，再試一次");
        }
    }

    /// <summary>點到的是編號就複製並回 true（呼叫端就不要再打開任務）。</summary>
    public static bool TryCopy(object? source, TimerEngine engine, string key)
    {
        if (!IsTarget(source))
            return false;
        Copy(engine, key);
        return true;
    }

    /// <summary>做一個可以點的編號 Run（滑過變手指，提示「點一下複製」）。</summary>
    public static Run Run(string key) => new(key) { Tag = Tag, Cursor = System.Windows.Input.Cursors.Hand, ToolTip = "點一下複製編號" };
}
