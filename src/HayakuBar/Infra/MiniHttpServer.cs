using System.Net;
using System.Net.Sockets;
using System.Text;

namespace HayakuBar.Infra;

public sealed record MiniRequest(string Method, string Path, string Query, string Body);

public sealed record MiniResponse(int Status, string ContentType, string Body);

/// <summary>
/// 只聽 127.0.0.1 的極簡 HTTP 伺服器。不用 HttpListener 是因為它在非系統管理員下需要 URL ACL。
/// 用在 OAuth 回呼與 Claude Code hooks。
/// </summary>
public sealed class MiniHttpServer : IDisposable
{
    readonly TcpListener _listener;
    readonly Func<MiniRequest, Task<MiniResponse>> _handler;
    readonly CancellationTokenSource _cts = new();

    public MiniHttpServer(int port, Func<MiniRequest, Task<MiniResponse>> handler)
    {
        _listener = new TcpListener(IPAddress.Loopback, port);
        _handler = handler;
    }

    public void Start()
    {
        _listener.Start();
        _ = AcceptLoopAsync();
    }

    async Task AcceptLoopAsync()
    {
        while (!_cts.IsCancellationRequested)
        {
            TcpClient client;
            try
            {
                client = await _listener.AcceptTcpClientAsync(_cts.Token);
            }
            catch
            {
                break;
            }
            _ = Task.Run(() => HandleAsync(client));
        }
    }

    async Task HandleAsync(TcpClient client)
    {
        using (client)
        {
            try
            {
                using var cts = CancellationTokenSource.CreateLinkedTokenSource(_cts.Token);
                cts.CancelAfter(TimeSpan.FromSeconds(10));
                var stream = client.GetStream();
                var request = await ReadRequestAsync(stream, cts.Token);
                if (request is null)
                    return;

                MiniResponse response;
                try
                {
                    response = await _handler(request);
                }
                catch (Exception ex)
                {
                    Log.Error("HTTP 處理失敗", ex);
                    response = new MiniResponse(500, "text/plain", "error");
                }

                var body = Encoding.UTF8.GetBytes(response.Body);
                var head = $"HTTP/1.1 {response.Status} {(response.Status == 200 ? "OK" : "Error")}\r\n" +
                           $"Content-Type: {response.ContentType}; charset=utf-8\r\n" +
                           $"Content-Length: {body.Length}\r\nConnection: close\r\n\r\n";
                await stream.WriteAsync(Encoding.ASCII.GetBytes(head), cts.Token);
                await stream.WriteAsync(body, cts.Token);
            }
            catch (Exception ex) when (ex is IOException or OperationCanceledException or SocketException)
            {
                // 對方提早斷線，忽略
            }
            catch (Exception ex)
            {
                Log.Error("HTTP 連線錯誤", ex);
            }
        }
    }

    static async Task<MiniRequest?> ReadRequestAsync(NetworkStream stream, CancellationToken ct)
    {
        var buffer = new byte[8192];
        var head = new MemoryStream();
        int headerEnd;
        while (true)
        {
            var n = await stream.ReadAsync(buffer, ct);
            if (n == 0)
                return null;
            head.Write(buffer, 0, n);
            if (head.Length > 64 * 1024)
                return null;
            headerEnd = FindHeaderEnd(head.GetBuffer(), (int)head.Length);
            if (headerEnd >= 0)
                break;
        }

        var all = head.GetBuffer();
        var total = (int)head.Length;
        var lines = Encoding.ASCII.GetString(all, 0, headerEnd).Split("\r\n");
        var first = lines[0].Split(' ');
        if (first.Length < 2)
            return null;

        var contentLength = 0;
        foreach (var line in lines.Skip(1))
        {
            var colon = line.IndexOf(':');
            if (colon > 0 && line[..colon].Trim().Equals("Content-Length", StringComparison.OrdinalIgnoreCase))
                int.TryParse(line[(colon + 1)..].Trim(), out contentLength);
        }
        if (contentLength is < 0 or > 5_000_000)
            return null;

        var body = new MemoryStream();
        var bodyStart = headerEnd + 4;
        body.Write(all, bodyStart, total - bodyStart);
        while (body.Length < contentLength)
        {
            var n = await stream.ReadAsync(buffer, ct);
            if (n == 0)
                break;
            body.Write(buffer, 0, n);
        }

        var target = first[1];
        var q = target.IndexOf('?');
        var bodyText = Encoding.UTF8.GetString(body.GetBuffer(), 0, (int)Math.Min(body.Length, contentLength));
        return new MiniRequest(first[0], q >= 0 ? target[..q] : target, q >= 0 ? target[(q + 1)..] : "", bodyText);
    }

    static int FindHeaderEnd(byte[] data, int length) => data.AsSpan(0, length).IndexOf("\r\n\r\n"u8);

    public void Dispose()
    {
        _cts.Cancel();
        try { _listener.Stop(); } catch { }
    }
}
