using HayakuBar.Core;

namespace HayakuBar.Tests;

public class KeyFinderTests
{
    static readonly HashSet<string> Projects = new(StringComparer.OrdinalIgnoreCase) { "F2P", "EG", "REVIVAL" };
    static bool Known(string key) => Projects.Contains(KeyFinder.ProjectOf(key));

    [Fact]
    public void Prompt_取最後提到的已知任務()
    {
        Assert.Equal("EG-7", KeyFinder.FindInPrompt("先看 F2P-97，再處理 EG-7 的 UTF-8 問題", Known));
    }

    [Fact]
    public void Prompt_不認小寫與未知專案()
    {
        Assert.Null(KeyFinder.FindInPrompt("f2p-97 跟 UTF-8、ISO-8859", Known));
    }

    [Theory]
    [InlineData("feature/f2p-97-reset-button", "F2P-97")]
    [InlineData("F2P-14", "F2P-14")]
    [InlineData("bugfix/EG-18_sync", "EG-18")]
    [InlineData("main", null)]
    [InlineData("release-2026-10", null)]
    public void Branch_大小寫都認(string branch, string? expected) =>
        Assert.Equal(expected, KeyFinder.FindInBranch(branch, Known));

    [Theory]
    [InlineData("F2P-97  報表頁面新增歸0按鈕", "F2P-97")]
    [InlineData(" f2p-5 ", "F2P-5")]
    [InlineData("沒有編號", null)]
    public void Parse_使用者輸入(string text, string? expected) => Assert.Equal(expected, KeyFinder.Parse(text));
}
