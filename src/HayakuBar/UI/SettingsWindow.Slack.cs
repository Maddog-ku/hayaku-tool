using System.Windows;
using System.Windows.Input;
using HayakuBar.Infra;
using HayakuBar.Slack;

namespace HayakuBar.UI;

/// <summary>設定的 Slack 頁：建立 App 的步驟、貼金鑰、送測試訊息、中斷連線。</summary>
public partial class SettingsWindow
{
    void LoadSlack()
    {
        var cred = SlackCredentialStore.Load();
        ShowSlackAccount(cred.IsEmpty ? null : cred);
        SlackForwardBox.IsChecked = _engine.Config.SlackForward;
        RefreshSlackStatus();
        if (_engine.Slack is { } slack)
        {
            slack.StatusChanged += RefreshSlackStatus;
            Closed += (_, _) => slack.StatusChanged -= RefreshSlackStatus;
        }
    }

    void RefreshSlackStatus()
    {
        var slack = _engine.Slack;
        SlackLiveStatus.Text = slack?.StatusText ?? "";
        SlackLiveStatus.Foreground = Brushes2.Get(slack is { Configured: true, Healthy: false } && !slack.StatusText.StartsWith("連線中")
            ? "ErrorBrush" : "MutedBrush");
    }

    void SlackForward_Click(object sender, RoutedEventArgs e)
    {
        _engine.Config.SlackForward = SlackForwardBox.IsChecked == true;
        Apply();
    }

    /// <summary>已連線只顯示工作區、送測試訊息與中斷連線；沒有金鑰才顯示建立 App 的步驟與輸入欄位。</summary>
    void ShowSlackAccount(SlackCredentials? cred)
    {
        var connected = cred is not null;
        SlackConnectedPanel.Visibility = SlackForwardPanel.Visibility = connected ? Visibility.Visible : Visibility.Collapsed;
        SlackSetupPanel.Visibility = connected ? Visibility.Collapsed : Visibility.Visible;
        SlackAccountText.Text = connected
            ? $"{(cred!.Team.Length > 0 ? cred.Team + " · " : "")}私訊給 {cred.UserId} · 金鑰用 Windows 加密存在這台電腦"
            : "";
        if (!connected)
            SlackBotBox.Password = SlackAppBox.Password = "";
    }

    void ShowSlackStatus(string? text, string brush = "MutedBrush")
    {
        SlackStatus.Text = text ?? "";
        SlackStatus.Foreground = Brushes2.Get(brush);
        SlackStatus.Visibility = string.IsNullOrEmpty(text) ? Visibility.Collapsed : Visibility.Visible;
    }

    void SlackCopyManifest_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            Clipboard.SetText(SlackClient.Manifest);
            ShowSlackStatus("已複製 App 設定，到 Slack 貼上");
        }
        catch (Exception ex) when (ex is System.Runtime.InteropServices.COMException or System.Runtime.InteropServices.ExternalException)
        {
            ShowSlackStatus("複製失敗，剪貼簿被別的程式占用，再試一次", "ErrorBrush");
        }
    }

    void SlackOpenApps_Click(object sender, RoutedEventArgs e) => Web.Open(SlackClient.AppsUrl);

    void SlackBox_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        SlackConnect_Click(sender, e);
    }

    async void SlackConnect_Click(object sender, RoutedEventArgs e)
    {
        var cred = new SlackCredentials
        {
            BotToken = SlackBotBox.Password.Trim(),
            AppToken = SlackAppBox.Password.Trim(),
            UserId = SlackUserBox.Text.Trim(),
        };
        if (cred.IsEmpty)
        {
            ShowSlackStatus("請填 Bot Token、App-Level Token 和成員 ID", "ErrorBrush");
            return;
        }
        if (!cred.BotToken.StartsWith("xoxb-") || !cred.AppToken.StartsWith("xapp-"))
        {
            ShowSlackStatus(SlackClient.Describe("not_allowed_token_type"), "ErrorBrush");
            return;
        }
        SlackConnectButton.IsEnabled = false;
        ShowSlackStatus("連線中…");
        try
        {
            using var client = new SlackClient();
            cred.Team = await client.AuthTestAsync(cred.BotToken, CancellationToken.None);
            await client.OpenSocketUrlAsync(cred.AppToken, CancellationToken.None);
            await SendTestAsync(client, cred);
            SlackCredentialStore.Save(cred);
            _engine.Slack?.Restart();
            Log.Info($"Slack 已連線（{cred.Team}，私訊給 {cred.UserId}）");
            ShowSlackAccount(cred);
            ShowSlackStatus("已連線，到 Slack 左邊「應用程式」的 HayakuBar 看看有沒有收到測試訊息", "RunBrush");
        }
        catch (Exception ex)
        {
            Log.Error("Slack 連線失敗", ex);
            ShowSlackStatus($"連線失敗：{ex.Message}", "ErrorBrush");
        }
        finally
        {
            SlackConnectButton.IsEnabled = true;
        }
    }

    async void SlackTest_Click(object sender, RoutedEventArgs e)
    {
        var cred = SlackCredentialStore.Load();
        if (cred.IsEmpty)
        {
            ShowSlackAccount(null);
            return;
        }
        SlackTestButton.IsEnabled = false;
        ShowSlackStatus("傳送中…");
        try
        {
            using var client = new SlackClient();
            await SendTestAsync(client, cred);
            ShowSlackStatus("已送出測試訊息", "RunBrush");
        }
        catch (Exception ex)
        {
            Log.Error("Slack 測試訊息失敗", ex);
            ShowSlackStatus($"傳送失敗：{ex.Message}", "ErrorBrush");
        }
        finally
        {
            SlackTestButton.IsEnabled = true;
        }
    }

    static async Task SendTestAsync(SlackClient client, SlackCredentials cred)
    {
        var channel = await client.OpenDmAsync(cred.BotToken, cred.UserId, CancellationToken.None);
        await client.PostMessageAsync(cred.BotToken, channel, $"✓ HayakuBar 已連上 Slack（測試訊息，{DateTime.Now:M/d HH:mm}）", null,
            CancellationToken.None);
    }

    void SlackLogout_Click(object sender, RoutedEventArgs e)
    {
        var ok = MessageBox.Show(this, "中斷 Slack 連線？會刪掉存在這台電腦的金鑰，之後不再傳到 Slack。Slack 上的 App 不會被刪除。", "Slack",
            MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok != MessageBoxResult.OK)
            return;
        SlackCredentialStore.Clear();
        _engine.Slack?.Restart();
        Log.Info("Slack 已中斷連線");
        SlackUserBox.Text = "";
        ShowSlackAccount(null);
        ShowSlackStatus(null);
    }
}
