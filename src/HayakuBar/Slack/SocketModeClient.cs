using System.Net.WebSockets;
using System.Text;
using System.Text.Json;
using HayakuBar.Infra;

namespace HayakuBar.Slack;

/// <summary>
/// Slack Socket Mode：HayakuBar 主動用 WebSocket 連到 Slack，Slack 把按鈕、表單的互動從這條連線送過來，不需要對外開放的網址。
/// 每則訊息都要在 3 秒內回 envelope_id（先回再處理）；Slack 會不定時要求重連（disconnect），斷線就等一下再連（5 秒起跳，最多 5 分鐘）。
/// </summary>
public sealed class SocketModeClient : IDisposable
{
    static readonly TimeSpan MinDelay = TimeSpan.FromSeconds(5), MaxDelay = TimeSpan.FromMinutes(5);

    readonly Func<CancellationToken, Task<string>> _openUrl;
    readonly Func<JsonElement, Task> _onInteractive;
    readonly SemaphoreSlim _send = new(1, 1);
    CancellationTokenSource? _cts;
    Task? _loop;

    public SocketModeClient(Func<CancellationToken, Task<string>> openUrl, Func<JsonElement, Task> onInteractive)
    {
        _openUrl = openUrl;
        _onInteractive = onInteractive;
    }

    /// <summary>連線狀態：null＝已連上；其他是給人看的說明（連線中、斷線原因）。</summary>
    public string? Problem { get; private set; } = "連線中…";

    public event Action? StatusChanged;

    public void Start()
    {
        if (_loop is not null)
            return;
        _cts = new CancellationTokenSource();
        _loop = Task.Run(() => RunAsync(_cts.Token));
    }

    public void Stop()
    {
        _cts?.Cancel();
        _cts = null;
        _loop = null;
        SetProblem("未連線");
    }

    void SetProblem(string? problem)
    {
        if (Problem == problem)
            return;
        Problem = problem;
        StatusChanged?.Invoke();
    }

    async Task RunAsync(CancellationToken ct)
    {
        var delay = MinDelay;
        while (!ct.IsCancellationRequested)
        {
            try
            {
                SetProblem("連線中…");
                var url = await _openUrl(ct);
                using var ws = new ClientWebSocket();
                ws.Options.KeepAliveInterval = TimeSpan.FromSeconds(30);
                await ws.ConnectAsync(new Uri(url), ct);
                await ReceiveAsync(ws, () => delay = MinDelay, ct);
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            {
                return;
            }
            catch (Exception ex)
            {
                if (ct.IsCancellationRequested)
                    return;
                Log.Error("Slack 連線中斷", ex);
                SetProblem(ex is SlackException se ? $"連不上：{SlackClient.Describe(se.Error)}" : $"連不上：{ex.Message}");
            }
            if (ct.IsCancellationRequested)
                return;
            try
            {
                await Task.Delay(delay, ct);
            }
            catch (OperationCanceledException)
            {
                return;
            }
            delay = TimeSpan.FromTicks(Math.Min(delay.Ticks * 2, MaxDelay.Ticks));
        }
    }

    /// <summary>一直收到 Slack 要求重連或連線斷掉為止。</summary>
    async Task ReceiveAsync(ClientWebSocket ws, Action connected, CancellationToken ct)
    {
        var buffer = new byte[16 * 1024];
        using var message = new MemoryStream();
        while (ws.State == WebSocketState.Open && !ct.IsCancellationRequested)
        {
            message.SetLength(0);
            WebSocketReceiveResult result;
            do
            {
                result = await ws.ReceiveAsync(buffer, ct);
                if (result.MessageType == WebSocketMessageType.Close)
                    return;
                message.Write(buffer, 0, result.Count);
            } while (!result.EndOfMessage);

            using var doc = JsonDocument.Parse(message.ToArray());
            var root = doc.RootElement;
            var type = root.TryGetProperty("type", out var t) ? t.GetString() : null;
            if (type == "hello")
            {
                connected();
                SetProblem(null);
                Log.Info("Slack Socket Mode 已連線");
                continue;
            }
            if (type == "disconnect")
            {
                Log.Info($"Slack 要求重連（{(root.TryGetProperty("reason", out var r) ? r.GetString() : "")}）");
                return;
            }
            if (root.TryGetProperty("envelope_id", out var envelope) && envelope.GetString() is { } id)
                await SendAsync(ws, JsonSerializer.Serialize(new { envelope_id = id }), ct);
            if (type == "interactive" && root.TryGetProperty("payload", out var payload))
            {
                var copy = payload.Clone();
                _ = Task.Run(async () =>
                {
                    try
                    {
                        await _onInteractive(copy);
                    }
                    catch (Exception ex)
                    {
                        Log.Error("處理 Slack 按鈕失敗", ex);
                    }
                });
            }
        }
    }

    async Task SendAsync(ClientWebSocket ws, string json, CancellationToken ct)
    {
        await _send.WaitAsync(ct);
        try
        {
            await ws.SendAsync(Encoding.UTF8.GetBytes(json), WebSocketMessageType.Text, true, ct);
        }
        finally
        {
            _send.Release();
        }
    }

    public void Dispose() => Stop();
}
