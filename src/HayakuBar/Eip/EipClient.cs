using System.Globalization;
using System.Net;
using System.Text.RegularExpressions;
using HayakuBar.Infra;

namespace HayakuBar.Eip;

public sealed record EipLoginResult(bool Ok, string Message);

/// <summary>EIP「假單」清單的一列。Kind 是性質（請假、銷假、補假、黃卡），Type 是假別（特休、事假…），Subtotal 是小計（1天0小時）。</summary>
public sealed record EipLeave(string No, string PersonNo, string Kind, string Type, string Reason,
    DateTime Start, DateTime End, string Status, string? Id, string Subtotal = "");

/// <summary>
/// 公司 EIP（eip.yanray.com.tw，PHP 網站）。沒有 API，用帳密登入表單拿 PHPSESSID，之後帶 cookie 抓頁面。
/// 登入分兩段：先送 account、password（加上登入頁的隱藏欄位 csrf_token、state），
/// 再送「登入身分選擇」表單（頁面本來會自動送出，帶 person_no＝工號）。
/// </summary>
public sealed partial class EipClient : IDisposable
{
    public static readonly Uri BaseUri = new("https://eip.yanray.com.tw/");
    const string LoginPath = "plug/home/php/login.php?get_url=";
    const string LeaveListPath = "plug/absentNext/php/leave_list.php";

    /// <summary>假單清單的狀態篩選（absent_state）。</summary>
    public const int StatePending = 0, StateApproved = 1, StateRejected = 2;

    readonly HttpClient _http;

    public EipClient()
    {
        var handler = new SocketsHttpHandler
        {
            CookieContainer = new CookieContainer(),
            AutomaticDecompression = DecompressionMethods.All,
            PooledConnectionLifetime = TimeSpan.FromMinutes(10),
        };
        _http = new HttpClient(handler) { BaseAddress = BaseUri, Timeout = TimeSpan.FromSeconds(20) };
        _http.DefaultRequestHeaders.UserAgent.ParseAdd("Mozilla/5.0 (Windows NT 10.0; Win64; x64) HayakuBar");
    }

    public bool LoggedIn { get; private set; }

    /// <summary>登入後的工號（例 Y2798），用來只挑自己的假單。</summary>
    public string? PersonNo { get; private set; }

    /// <summary>用帳密登入；帳密錯誤回 Ok=false，連不上 EIP 會丟例外。</summary>
    public async Task<EipLoginResult> LoginAsync(EipCredentials cred, CancellationToken ct)
    {
        LoggedIn = false;
        var form = await _http.GetStringAsync(LoginPath, ct);
        var fields = HiddenFields(form);
        if (!fields.ContainsKey("csrf_token"))
            return new EipLoginResult(false, "EIP 登入頁的格式跟預期不同，可能改版了");
        fields["account"] = cred.Account;
        fields["password"] = cred.Password;

        var html = await PostFormAsync(LoginPath, fields, ct);
        if (IsLoginForm(html))
        {
            var alert = AlertText(html);
            Log.Info($"EIP 登入失敗{(alert is null ? "" : $"：{alert}")}");
            return new EipLoginResult(false, alert ?? "帳號或密碼錯誤");
        }

        // 第二段：登入身分選擇（瀏覽器會自動送出）
        var identity = IdentityFields(html);
        if (identity.TryGetValue("person_no", out var personNo))
        {
            PersonNo = personNo;
            html = await PostFormAsync(LoginPath, identity, ct);
            if (IsLoginForm(html))
                return new EipLoginResult(false, AlertText(html) ?? "EIP 登入身分選擇失敗");
        }
        LoggedIn = true;
        Log.Info($"EIP 登入成功（工號 {PersonNo ?? "未知"}）");
        return new EipLoginResult(true, "登入成功");
    }

    /// <summary>某一年、某個狀態的假單（自己的，主管會連部屬的一起列出，用 PersonNo 過濾）。</summary>
    public async Task<List<EipLeave>> ListLeavesAsync(int year, int state, EipCredentials cred, CancellationToken ct)
    {
        var html = await WithLoginAsync(() => PostFormAsync(LeaveListPath, new Dictionary<string, string>
        {
            ["years"] = year.ToString(),
            ["selectDep"] = "",
            ["selectPerson"] = "",
            ["absent_state"] = state.ToString(),
            ["btnSearch"] = "",
        }, ct), cred, ct);
        if (!html.Contains("請假時間(起)"))
            throw new InvalidOperationException("EIP 假單頁的格式跟預期不同，可能改版了");
        var leaves = ParseLeaveList(html);
        return PersonNo is { Length: > 0 } me ? leaves.Where(l => l.PersonNo.Equals(me, StringComparison.OrdinalIgnoreCase)).ToList() : leaves;
    }

    /// <summary>還沒登入就先登入；session 過期（被導回登入頁）時重新登入一次再試。</summary>
    async Task<string> WithLoginAsync(Func<Task<string>> fetch, EipCredentials cred, CancellationToken ct)
    {
        if (!LoggedIn && !(await LoginAsync(cred, ct)).Ok)
            throw new EipLoginException("EIP 帳號或密碼錯誤");
        var html = await fetch();
        if (!IsLoginForm(html))
            return html;
        if (!(await LoginAsync(cred, ct)).Ok)
            throw new EipLoginException("EIP 帳號或密碼錯誤");
        return await fetch();
    }

    async Task<string> PostFormAsync(string path, Dictionary<string, string> fields, CancellationToken ct)
    {
        using var res = await _http.PostAsync(path, new FormUrlEncodedContent(fields), ct);
        res.EnsureSuccessStatusCode();
        return await res.Content.ReadAsStringAsync(ct);
    }

    public void Dispose() => _http.Dispose();

    // ───────────── 解析（internal 給測試用） ─────────────

    [GeneratedRegex(@"<input\b[^>]*>", RegexOptions.IgnoreCase)]
    private static partial Regex InputTag();

    [GeneratedRegex(@"\b(?<name>[\w-]+)\s*=\s*(?:'(?<v>[^']*)'|""(?<v>[^""]*)""|(?<v>[^\s>]+))", RegexOptions.IgnoreCase)]
    private static partial Regex Attribute();

    [GeneratedRegex(@"\schecked\b", RegexOptions.IgnoreCase)]
    private static partial Regex Checked();

    [GeneratedRegex(@"alert\s*\(\s*(['""])(?<msg>.*?)\1\s*\)", RegexOptions.IgnoreCase | RegexOptions.Singleline)]
    private static partial Regex Alert();

    [GeneratedRegex(@"<tr\b[^>]*>(?<row>.*?)</tr>", RegexOptions.IgnoreCase | RegexOptions.Singleline)]
    private static partial Regex Row();

    [GeneratedRegex(@"<t[dh]\b[^>]*>(?<cell>.*?)</t[dh]>", RegexOptions.IgnoreCase | RegexOptions.Singleline)]
    private static partial Regex Cell();

    [GeneratedRegex(@"<[^>]+>")]
    private static partial Regex Tag();

    [GeneratedRegex(@"[?&]ID=(?<id>\d+)", RegexOptions.IgnoreCase)]
    private static partial Regex RowId();

    static Dictionary<string, string> Attributes(string tag) =>
        Attribute().Matches(tag)
            .GroupBy(m => m.Groups["name"].Value.ToLowerInvariant())
            .ToDictionary(g => g.Key, g => WebUtility.HtmlDecode(g.First().Groups["v"].Value));

    static string Text(string html) => WebUtility.HtmlDecode(Tag().Replace(html, "")).Trim();

    /// <summary>表單裡的隱藏欄位（name → value），value 照原樣送回去，跟瀏覽器一樣。</summary>
    internal static Dictionary<string, string> HiddenFields(string html) => Inputs(html, radios: false);

    /// <summary>「登入身分選擇」表單要送的欄位：隱藏欄位＋已勾選的 radio。</summary>
    internal static Dictionary<string, string> IdentityFields(string html) => Inputs(html, radios: true);

    static Dictionary<string, string> Inputs(string html, bool radios)
    {
        var fields = new Dictionary<string, string>();
        foreach (Match m in InputTag().Matches(html))
        {
            var a = Attributes(m.Value);
            var type = a.GetValueOrDefault("type")?.ToLowerInvariant();
            var wanted = type == "hidden" || (radios && type == "radio" && Checked().IsMatch(m.Value));
            if (wanted && a.TryGetValue("name", out var name) && name.Length > 0)
                fields[name] = a.GetValueOrDefault("value") ?? "";
        }
        return fields;
    }

    /// <summary>頁面上有密碼欄位就是還在登入頁（登入失敗或 session 過期）。</summary>
    internal static bool IsLoginForm(string html) =>
        InputTag().Matches(html).Any(m => Attributes(m.Value).GetValueOrDefault("type")?.Equals("password", StringComparison.OrdinalIgnoreCase) == true);

    /// <summary>PHP 常用 alert('…') 回錯誤訊息。</summary>
    internal static string? AlertText(string html) =>
        Alert().Match(html) is { Success: true } m && m.Groups["msg"].Value.Trim() is { Length: > 0 } msg ? msg : null;

    /// <summary>解析「假單」清單（leave_list.php）的表格；欄位用表頭文字對應，沒有起訖時間的列（黃卡）略過。</summary>
    internal static List<EipLeave> ParseLeaveList(string html)
    {
        var list = new List<EipLeave>();
        List<string>? headers = null;
        foreach (Match row in Row().Matches(html))
        {
            var cells = Cell().Matches(row.Groups["row"].Value).Select(c => c.Groups["cell"].Value).ToList();
            if (headers is null)
            {
                var texts = cells.Select(Text).ToList();
                if (texts.Contains("單號") && texts.Contains("請假時間(起)"))
                    headers = texts;
                continue;
            }
            string Col(string name) => headers.IndexOf(name) is var i and >= 0 && i < cells.Count ? Text(cells[i]) : "";

            if (ParseTime(Col("請假時間(起)")) is not { } start || ParseTime(Col("請假時間(迄)")) is not { } end || end <= start)
                continue;
            var id = RowId().Match(row.Value) is { Success: true } m ? m.Groups["id"].Value : null;
            list.Add(new EipLeave(Col("單號"), Col("工號"), Col("類別"), Col("假別"), Col("請假原因"), start, end, Col("狀態"), id, Col("小計")));
        }
        return list;
    }

    static DateTime? ParseTime(string text) =>
        DateTime.TryParseExact(text, ["yyyy-MM-dd HH:mm", "yyyy-MM-dd HH:mm:ss", "yyyy/MM/dd HH:mm"], CultureInfo.InvariantCulture,
            DateTimeStyles.None, out var t) ? t : null;
}

public sealed class EipLoginException(string message) : Exception(message);
