using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>
/// 讀檔案內容（read_file）的快取與限流：同一個檔案（同一段）只讀一次；Hayaku 限制每人 10 分鐘最多 30 次（跟 Claude 共用），
/// 這裡自己最多用 24 次，留餘裕給 Claude，超過就請使用者等一下，不會一直打到被擋。
/// </summary>
public sealed class FileReader(HayakuApi api)
{
    const int MaxCached = 40;
    const int Budget = 24;
    static readonly TimeSpan Window = TimeSpan.FromMinutes(10);

    readonly Dictionary<(string Id, int Offset), FileContent> _cache = [];
    readonly LinkedList<(string Id, int Offset)> _order = [];
    readonly Queue<DateTime> _calls = new();

    public bool IsCached(string id, int offset = 0) => _cache.ContainsKey((id, offset));

    public async Task<FileContent> ReadAsync(string id, int offset = 0)
    {
        if (_cache.TryGetValue((id, offset), out var cached))
            return cached;
        var now = DateTime.Now;
        while (_calls.Count > 0 && now - _calls.Peek() > Window)
            _calls.Dequeue();
        if (_calls.Count >= Budget)
        {
            var wait = Math.Max(1, (int)Math.Ceiling((Window - (now - _calls.Peek())).TotalMinutes));
            throw new McpException($"10 分鐘內讀了太多檔案（Hayaku 限制每人 30 次，跟 Claude 共用），約 {wait} 分鐘後再試");
        }
        _calls.Enqueue(now);
        var content = await api.ReadFileAsync(id, offset, CancellationToken.None);
        _cache[(id, offset)] = content;
        _order.AddLast((id, offset));
        while (_order.Count > MaxCached)
        {
            _cache.Remove(_order.First!.Value);
            _order.RemoveFirst();
        }
        return content;
    }

    /// <summary>定期清暫存時呼叫：圖片很佔記憶體，整個清掉（限流的紀錄留著）。</summary>
    public void Clear()
    {
        _cache.Clear();
        _order.Clear();
    }
}
