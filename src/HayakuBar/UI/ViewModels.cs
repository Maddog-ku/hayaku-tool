using System.ComponentModel;
using System.Windows;
using System.Windows.Media;
using HayakuBar.Claude;
using HayakuBar.Core;

namespace HayakuBar.UI;

static class Brushes2
{
    public static Brush Get(string key) => (Brush)Application.Current.Resources[key];
}

public sealed class TaskRowVm : INotifyPropertyChanged
{
    public required string Key { get; init; }
    public required string Title { get; init; }
    public bool Running { get; init; }
    public bool IsClaude { get; init; }

    public string Icon => IsClaude ? "✦" : Running ? "●" : "○";
    public Brush IconBrush => Brushes2.Get(IsClaude ? "ClaudeBrush" : Running ? "RunBrush" : "MutedBrush");
    public string IconTip => IsClaude ? "Claude 自動計時" : Running ? "計時中" : "已暫停";
    public string ActionGlyph => Busy ? "…" : Running ? "❚❚" : "▶";
    public string ActionTip => Busy ? "處理中" : Running ? "暫停" : "開始計時";

    bool _busy;

    /// <summary>按了開始／暫停、還在等 Hayaku 回應。</summary>
    public bool Busy
    {
        get => _busy;
        set
        {
            if (_busy == value)
                return;
            _busy = value;
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(Busy)));
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(ActionGlyph)));
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(ActionTip)));
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(NotBusy)));
        }
    }

    public bool NotBusy => !Busy;

    string _elapsed = "";
    public string Elapsed
    {
        get => _elapsed;
        set
        {
            if (_elapsed == value)
                return;
            _elapsed = value;
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(Elapsed)));
        }
    }

    public event PropertyChangedEventHandler? PropertyChanged;
}

public sealed class RepoRowVm
{
    public required string Name { get; init; }
    public required string Path { get; init; }
    public required string StatusText { get; init; }
    public required Brush StatusBrush { get; init; }
    public required string TaskText { get; init; }
    public required string TaskTip { get; init; }
    public Visibility PickVisibility { get; init; }

    public static RepoRowVm From(RepoActivity a, DateTime now, int idleMinutes)
    {
        string status;
        Brush brush;
        if (!a.Enabled)
            (status, brush) = ("已停用", Brushes2.Get("MutedBrush"));
        else if (a.LastActivity == DateTime.MinValue)
            (status, brush) = ("找不到 session", Brushes2.Get("MutedBrush"));
        else if (a.Active)
            (status, brush) = (a.SourceText.Length > 0 ? $"活動中（{a.SourceText}）" : "活動中", Brushes2.Get("ClaudeBrush"));
        else
            (status, brush) = ($"閒置 {Ago(now - a.LastActivity)}", Brushes2.Get("MutedBrush"));

        return new RepoRowVm
        {
            Name = a.Name,
            Path = a.Path,
            StatusText = status,
            StatusBrush = brush,
            TaskText = a.ResolvedTask ?? "未對應任務",
            TaskTip = a.ResolvedTask is null
                ? $"偵測不到任務編號（分支：{a.Branch ?? "無"}），點「選擇」指定"
                : $"來源：{a.TaskSource}；Claude 有動作就開始計時，之後持續計時",
            PickVisibility = Visibility.Visible,
        };
    }

    static string Ago(TimeSpan span) =>
        span.TotalMinutes < 1 ? "不到 1 分" :
        span.TotalHours < 1 ? $"{(int)span.TotalMinutes} 分" :
        span.TotalDays < 1 ? $"{(int)span.TotalHours} 小時" : $"{(int)span.TotalDays} 天";
}

public sealed class MeetingRowVm
{
    public required Mcp.MeetingItem Item { get; init; }
    public required string Time { get; init; }
    public required string Title { get; init; }
    public required string Detail { get; init; }
    public required Brush TimeBrush { get; init; }
    public Visibility JoinVisibility { get; init; }

    public static MeetingRowVm From(Mcp.MeetingItem m, DateTimeOffset now)
    {
        var ongoing = now >= m.StartAt && now < m.EndAt;
        var canJoin = !m.InMeeting && now >= m.StartAt.AddMinutes(-15) && now < m.EndAt;
        var parts = new List<string>();
        if (m.Project.Length > 0)
            parts.Add(m.Project);
        if (!string.IsNullOrWhiteSpace(m.Location))
            parts.Add(m.Location!);
        if (!string.IsNullOrWhiteSpace(m.MyStatus))
            parts.Add(m.MyStatus!);
        else if (now >= m.EndAt)
            parts.Add("已結束");
        return new MeetingRowVm
        {
            Item = m,
            Time = $"{m.StartAt.ToLocalTime():HH:mm}–{m.EndAt.ToLocalTime():HH:mm}",
            Title = m.Title,
            Detail = string.Join(" · ", parts),
            TimeBrush = Brushes2.Get(ongoing ? "AccentBrush" : now >= m.EndAt ? "MutedBrush" : "TextBrush"),
            JoinVisibility = canJoin ? Visibility.Visible : Visibility.Collapsed,
        };
    }
}

public sealed class TodoRowVm : INotifyPropertyChanged
{
    bool _busy;

    public required Mcp.TodoItem Item { get; init; }
    public string Text => Item.Text;

    public string Detail => string.Join(" · ", new[]
    {
        Item.TaskCode,
        Item.RemindAt is { } at ? $"提醒 {at.ToLocalTime():M/d HH:mm}" : null,
        Item.AssignedBy is { } by ? $"{by} 指派" : null,
        Item.Source,
    }.Where(s => !string.IsNullOrWhiteSpace(s)));

    public Visibility DetailVisibility => Detail.Length > 0 ? Visibility.Visible : Visibility.Collapsed;

    public bool NotBusy => !_busy;

    public bool Busy
    {
        get => _busy;
        set
        {
            _busy = value;
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(NotBusy)));
        }
    }

    public event PropertyChangedEventHandler? PropertyChanged;
}

public sealed class ReminderRowVm
{
    public required Mcp.ReminderItem Item { get; init; }
    public string Text => Item.Task is { } task ? $"{Item.Text}（{task}）" : Item.Text;

    public string Time => Item.At is not { } at ? "" :
        at.ToLocalTime().Date == DateTime.Today ? at.ToLocalTime().ToString("HH:mm") : at.ToLocalTime().ToString("M/d HH:mm");
}

public sealed class NotificationRowVm
{
    public required Mcp.NotificationItem Item { get; init; }
    public string Label => Item.Label;
    public string CountText => Item.Count > 1 ? $"（{Item.Count}）" : "";
    public string Text => Item.Text;
    public Visibility UnreadVisibility => Item.Unread ? Visibility.Visible : Visibility.Collapsed;
    public Brush TextBrush => Brushes2.Get(Item.Unread ? "TextBrush" : "MutedBrush");

    public string Meta
    {
        get
        {
            var span = DateTimeOffset.Now - Item.CreatedAt;
            var ago = span.TotalMinutes < 1 ? "剛剛" : span.TotalHours < 1 ? $"{(int)span.TotalMinutes} 分鐘前" :
                span.TotalDays < 1 ? $"{(int)span.TotalHours} 小時前" : $"{(int)span.TotalDays} 天前";
            return Item.Actor is { } actor ? $"{actor} · {ago}" : ago;
        }
    }
}
