using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.UI;

/// <summary>
/// 批次新增任務：貼上清單（一行一個，用縮排表示子任務），一次建立整棵樹（create_tasks，全部成功或全部不建）。
/// 專案頁的「＋ 批次新增」開這個。
/// </summary>
public sealed class BatchCreateWindow : SmallDialog
{
    const int Max = 100;
    readonly TextBox _parent = new() { Margin = new Thickness(0, 0, 0, 12) };
    readonly TextBox _list = MultiLine(200);
    readonly CheckBox _assignMe = new() { Content = "全部指派給我", Margin = new Thickness(0, 0, 0, 12) };
    readonly TextBlock _preview = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, -6, 0, 12) };

    /// <summary>建好了（呼叫端據此重新整理）。</summary>
    public bool Created { get; private set; }

    public BatchCreateWindow(TimerEngine engine, string project, string? parent) : base($"批次新增任務 · {project}", 520)
    {
        _list.MaxHeight = 360;
        _list.AcceptsTab = true;
        _parent.Text = parent ?? "";
        _preview.Foreground = Brushes2.Get("MutedBrush");
        _list.TextChanged += (_, _) => Preview();
        Body.Children.Add(Label("放在哪個任務底下（選填，填編號；空著放在專案最上層）"));
        Body.Children.Add(_parent);
        Body.Children.Add(Label("任務清單：一行一個，用 Tab 或空白縮排表示是上一行的子任務（項目符號、編號會自動拿掉）"));
        Body.Children.Add(_list);
        Body.Children.Add(_preview);
        Body.Children.Add(_assignMe);
        AddButton("建立", primary: true, () => CreateAsync(engine, project));
        AddButton("取消", primary: false, cancel: true);
        Preview();
        Loaded += (_, _) => _list.Focus();
    }

    void Preview()
    {
        var nodes = TaskOutline.Parse(_list.Text);
        var count = TaskOutline.Count(nodes);
        int Depth(OutlineNode n) => n.Children.Count == 0 ? 1 : 1 + n.Children.Max(Depth);
        _preview.Text = count == 0 ? "還沒有任務" :
            $"會建立 {count} 個任務（最上層 {nodes.Count} 個、最深 {nodes.Max(Depth)} 層）" + (count > Max ? $"，一次最多 {Max} 個" : "");
        _preview.Foreground = Brushes2.Get(count > Max ? "ErrorBrush" : "MutedBrush");
    }

    async Task CreateAsync(TimerEngine engine, string project)
    {
        var nodes = TaskOutline.Parse(_list.Text);
        var count = TaskOutline.Count(nodes);
        if (count == 0)
        {
            ShowError("請先貼上任務清單");
            return;
        }
        if (count > Max)
        {
            ShowError($"一次最多 {Max} 個任務，請分批建立");
            return;
        }
        string? parent = null;
        if (_parent.Text.Trim().Length > 0 && (parent = KeyFinder.Parse(_parent.Text)) is null)
        {
            ShowError("上層請填任務編號，例 F2P-5");
            return;
        }
        var assignMe = _assignMe.IsChecked == true;
        await RunAsync(async () =>
        {
            var message = await engine.Api.CreateTasksAsync(project, parent, nodes, assignMe, CancellationToken.None);
            Log.Info($"批次新增 {project} {count} 個任務{(parent is null ? "" : $"（在 {parent} 底下）")}：{message ?? "完成"}");
            engine.ShowNotice(message ?? $"已建立 {count} 個任務");
            Created = true;
        });
        if (Created)
            await engine.AfterExternalChangeAsync();
    }
}
