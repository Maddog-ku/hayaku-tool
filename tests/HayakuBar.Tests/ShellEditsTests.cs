using HayakuBar.Claude;

namespace HayakuBar.Tests;

public class ShellEditsTests
{
    const string Root = @"D:\repo\Print-App";

    [Theory]
    [InlineData("sed -i 's/a/b/' src/a.java")]
    [InlineData("sed -n 270,320p KmWsdl.java; sed -i 's|// old|// new|' KmWsdl.java")]
    [InlineData("perl -pi -e 's/a/b/' a.pl")]
    [InlineData("echo x >> notes.md")]
    [InlineData("printf x | tee src/a.txt")]
    [InlineData("git apply fix.patch")]
    [InlineData("Set-Content -Path a.cs -Value x")]
    [InlineData("cat > src/a.txt <<'EOF'\nhello > world\nEOF")]
    [InlineData("python - <<'EOF'\nimport io\np='KmWsdl.java'\ns=io.open(p,encoding='utf-8',newline='').read()\nio.open(p,'w',encoding='utf-8',newline='').write(s)\nEOF")]
    [InlineData("python -c \"from pathlib import Path; Path('a.txt').write_text('x')\"")]
    public void 在專案裡寫檔(string command) => Assert.True(ShellEdits.Writes(command, Root, Root));

    [Theory]
    [InlineData("sed -n 40,420p src/a.java")]
    [InlineData("grep -rn \"a->b\" src | head")]
    [InlineData("git status 2>&1; ls > /dev/null")]
    [InlineData("dotnet build 2>&1 | tail -5")]
    [InlineData("Get-Content a.cs | Select-String x > $null")]
    [InlineData("python - <<'EOF'\nprint(1 > 0)\ns=open(p).read()\nEOF")]
    [InlineData("dotnet test > \"C:\\Users\\me\\AppData\\Local\\Temp\\claude\\out.txt\"")]
    [InlineData("git diff > /c/Users/me/scratchpad/diff.txt")]
    public void 只是讀取或寫到專案外_不算(string command) => Assert.False(ShellEdits.Writes(command, Root, Root));

    [Fact]
    public void 先cd進專案再改檔_算()
    {
        Assert.True(ShellEdits.Writes("cd /d/repo/Print-App/src && sed -i 's/a/b/' a.java", @"D:\sj", Root));
        Assert.True(ShellEdits.Writes("cd \"/d/repo/Print-App/1_中文 App\" && sed -i 's/a/b/' a.java", @"D:\sj", Root));
    }

    [Fact]
    public void 在別的資料夾改檔_不算()
    {
        Assert.False(ShellEdits.Writes("sed -i 's/a/b/' a.java", @"D:\repo\Other", Root));
        Assert.False(ShellEdits.Writes("cd /d/repo/Other && sed -i 's/a/b/' a.java", Root, Root));
        Assert.False(ShellEdits.Writes("sed -i 's/a/b/' a.java", @"D:\repo\Print-App2", Root));
    }

    [Fact]
    public void 用變數cd到暫存資料夾再改檔_不算()
    {
        Assert.False(ShellEdits.Writes("S=\"C:/Users/me/AppData/Local/Temp/claude/x/scratchpad\"; cd \"$S\"; sed -i 's/a/b/' a.sh", Root, Root));
        Assert.False(ShellEdits.Writes("SP=/c/Users/me/scratchpad && cd $SP && python - <<'EOF'\nopen('a.txt','w').write('x')\nEOF", Root, Root));
        Assert.False(ShellEdits.Writes("cd \"$NO_SUCH_VAR_FOR_TEST\" && sed -i 's/a/b/' a.sh", Root, Root));
        Assert.True(ShellEdits.Writes("S=/d/repo/Print-App/src; cd \"$S\" && sed -i 's/a/b/' a.java", @"D:\sj", Root));
    }

    [Fact]
    public void 只是把腳本寫成暫存檔_不算() =>
        Assert.False(ShellEdits.Writes("cat > \"C:/Users/me/scratchpad/edit.py\" <<'EOF'\nopen(p,'w').write(s)\nEOF", Root, Root));

    [Fact]
    public void 照指令先後順序看所在資料夾() =>
        Assert.True(ShellEdits.Writes("cd /d/repo/Print-App/src && sed -i 's/a/b/' a.java; cd /c/Users/me && ls", @"D:\sj", Root));

    [Fact]
    public void 從專案外重導向到專案裡的檔案_算() =>
        Assert.True(ShellEdits.Writes("echo x > /d/repo/Print-App/a.txt", @"D:\sj", Root));
}
