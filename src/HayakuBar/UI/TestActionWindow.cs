using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;

namespace HayakuBar.UI;

/// <summary>
/// 測試結果的小視窗：
/// fix＝開始修（可以順手填預估工時，也可以跳過；預設一起開始計時）；
/// reopen＝復驗沒過、退回（說明會變成一則留言，輪數 +1）。
/// 從提示、通知、任務詳細都開這個，不用開網頁。
/// </summary>
public sealed class TestActionWindow : SmallDialog
{
    readonly TimerEngine _engine;
    readonly TestActionRequest _request;

    /// <summary>做完了（成功）：給呼叫端更新畫面；參數是伺服器的說明。</summary>
    public event Action<string>? Done;

    public TestActionWindow(TimerEngine engine, TestActionRequest request)
        : base(request.Action == "fix" ? "開始修" : "退回測試結果")
    {
        _engine = engine;
        _request = request;
        Body.Children.Add(new TextBlock
        {
            Text = $"{request.Key} {request.Title}".TrimEnd(), FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(0, 0, 0, 12),
        });
        if (request.Action == "fix")
            BuildFix();
        else
            BuildReopen();
    }

    void BuildFix()
    {
        var hours = new TextBox { Style = (Style)FindResource("HintBox"), Tag = "例 2、0.5（可以不填）", Margin = new Thickness(0, 0, 0, 10) };
        var timer = new CheckBox { Content = "同時開始計時", IsChecked = true, Margin = new Thickness(0, 0, 0, 12) };
        Body.Children.Add(Label("預估要修幾小時"));
        Body.Children.Add(hours);
        Body.Children.Add(timer);
        AddButton("開始修", primary: true, () =>
        {
            double? h = null;
            var text = hours.Text.Trim();
            if (text.Length > 0)
            {
                if (!double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var parsed) || parsed < 0 || parsed > 1666)
                {
                    ShowError("預估工時請填小時數，例 2 或 0.5；不想填就留空");
                    return Task.CompletedTask;
                }
                h = parsed;
            }
            return RunAsync(async () => Done?.Invoke(await _engine.StartFixAsync(_request.Key, h, timer.IsChecked == true)));
        });
        AddButton("跳過預估，開始修", primary: false,
            () => RunAsync(async () => Done?.Invoke(await _engine.StartFixAsync(_request.Key, null, timer.IsChecked == true))));
        AddButton("取消", primary: false, cancel: true);
        Loaded += (_, _) => hours.Focus();
    }

    void BuildReopen()
    {
        var comment = MultiLine(80);
        Body.Children.Add(Label("哪裡還有問題（會變成一則留言給工程師）"));
        Body.Children.Add(comment);
        AddButton("退回", primary: true, () =>
        {
            if (comment.Text.Trim().Length == 0)
            {
                ShowError("寫一下哪裡還有問題，工程師才知道要修什麼");
                return Task.CompletedTask;
            }
            return RunAsync(async () => Done?.Invoke(await _engine.RetestAsync(_request.Key, pass: false, comment.Text.Trim())));
        });
        AddButton("取消", primary: false, cancel: true);
        Loaded += (_, _) => comment.Focus();
    }
}
