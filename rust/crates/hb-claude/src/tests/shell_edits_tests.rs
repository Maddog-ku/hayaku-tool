//! 移植自 ShellEditsTests.cs。

use crate::shell_edits::writes;

const ROOT: &str = r"D:\repo\Print-App";

#[test]
fn 在專案裡寫檔() {
    for command in [
        "sed -i 's/a/b/' src/a.java",
        "sed -n 270,320p KmWsdl.java; sed -i 's|// old|// new|' KmWsdl.java",
        "perl -pi -e 's/a/b/' a.pl",
        "echo x >> notes.md",
        "printf x | tee src/a.txt",
        "git apply fix.patch",
        "Set-Content -Path a.cs -Value x",
        "cat > src/a.txt <<'EOF'\nhello > world\nEOF",
        "python - <<'EOF'\nimport io\np='KmWsdl.java'\ns=io.open(p,encoding='utf-8',newline='').read()\nio.open(p,'w',encoding='utf-8',newline='').write(s)\nEOF",
        "python -c \"from pathlib import Path; Path('a.txt').write_text('x')\"",
    ] {
        assert!(writes(command, Some(ROOT), ROOT), "{command}");
    }
}

#[test]
fn 只是讀取或寫到專案外_不算() {
    for command in [
        "sed -n 40,420p src/a.java",
        "grep -rn \"a->b\" src | head",
        "git status 2>&1; ls > /dev/null",
        "dotnet build 2>&1 | tail -5",
        "Get-Content a.cs | Select-String x > $null",
        "python - <<'EOF'\nprint(1 > 0)\ns=open(p).read()\nEOF",
        "dotnet test > \"C:\\Users\\me\\AppData\\Local\\Temp\\claude\\out.txt\"",
        "git diff > /c/Users/me/scratchpad/diff.txt",
    ] {
        assert!(!writes(command, Some(ROOT), ROOT), "{command}");
    }
}

#[test]
fn 先cd進專案再改檔_算() {
    assert!(writes("cd /d/repo/Print-App/src && sed -i 's/a/b/' a.java", Some(r"D:\sj"), ROOT));
    assert!(writes("cd \"/d/repo/Print-App/1_中文 App\" && sed -i 's/a/b/' a.java", Some(r"D:\sj"), ROOT));
}

#[test]
fn 在別的資料夾改檔_不算() {
    assert!(!writes("sed -i 's/a/b/' a.java", Some(r"D:\repo\Other"), ROOT));
    assert!(!writes("cd /d/repo/Other && sed -i 's/a/b/' a.java", Some(ROOT), ROOT));
    assert!(!writes("sed -i 's/a/b/' a.java", Some(r"D:\repo\Print-App2"), ROOT));
}

#[test]
fn 用變數cd到暫存資料夾再改檔_不算() {
    assert!(!writes(
        "S=\"C:/Users/me/AppData/Local/Temp/claude/x/scratchpad\"; cd \"$S\"; sed -i 's/a/b/' a.sh",
        Some(ROOT),
        ROOT
    ));
    assert!(!writes(
        "SP=/c/Users/me/scratchpad && cd $SP && python - <<'EOF'\nopen('a.txt','w').write('x')\nEOF",
        Some(ROOT),
        ROOT
    ));
    assert!(!writes("cd \"$NO_SUCH_VAR_FOR_TEST\" && sed -i 's/a/b/' a.sh", Some(ROOT), ROOT));
    assert!(writes("S=/d/repo/Print-App/src; cd \"$S\" && sed -i 's/a/b/' a.java", Some(r"D:\sj"), ROOT));
}

#[test]
fn 只是把腳本寫成暫存檔_不算() {
    assert!(!writes("cat > \"C:/Users/me/scratchpad/edit.py\" <<'EOF'\nopen(p,'w').write(s)\nEOF", Some(ROOT), ROOT));
}

#[test]
fn 照指令先後順序看所在資料夾() {
    assert!(writes("cd /d/repo/Print-App/src && sed -i 's/a/b/' a.java; cd /c/Users/me && ls", Some(r"D:\sj"), ROOT));
}

#[test]
fn 從專案外重導向到專案裡的檔案_算() {
    assert!(writes("echo x > /d/repo/Print-App/a.txt", Some(r"D:\sj"), ROOT));
}
