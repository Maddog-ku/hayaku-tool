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
    public string ActionGlyph => Running ? "❚❚" : "▶";
    public string ActionTip => Running ? "暫停" : "開始計時";

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
        if (!a.Repo.Enabled)
            (status, brush) = ("已停用", Brushes2.Get("MutedBrush"));
        else if (a.LastActivity == DateTime.MinValue)
            (status, brush) = ("找不到 session", Brushes2.Get("MutedBrush"));
        else if (a.Active)
            (status, brush) = ("活動中", Brushes2.Get("ClaudeBrush"));
        else
            (status, brush) = ($"閒置 {Ago(now - a.LastActivity)}", Brushes2.Get("MutedBrush"));

        return new RepoRowVm
        {
            Name = a.Name,
            Path = a.Repo.Path,
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
