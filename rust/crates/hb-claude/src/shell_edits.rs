//! 對應 C# `Claude/ShellEdits.cs`。
//!
//! 判斷 Claude 跑的 Bash／PowerShell 指令有沒有在專案裡寫檔：Claude 常用 sed -i、python 腳本改程式，不一定走 Edit 工具。
//! 只認常見的寫法：sed／perl -i、重導向或 tee 到專案裡的檔案、Set-Content 類、git apply／patch、
//! 交給 python／node 執行的腳本（heredoc 或 -c）裡開檔寫入。
//! 純讀取（sed -n、grep、cat、git status）、寫到專案外（暫存資料夾、/dev/null）、只是把腳本寫成檔案的都不算。
//! 所在資料夾照指令先後順序追 cd（變數也會展開）；cd 到看不出來的地方，之後的動作就不算。
//!
//! Rust 的 regex 不支援反向參考與前後環視：Heredoc、Redirect 照 .NET 的比對規則手寫掃描，
//! 其他有環視的（Interpreter、InPlace）把環視改成擷取，結果（是否相符、相符的起點）跟 C# 一樣。

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Captures, Regex};

use crate::claude_monitor::{is_under, normalize};
use crate::util::fold;

/// 標記「這裡有一段會寫檔的腳本」，取代掉腳本內容後留在指令裡，才知道它在第幾個 cd 之後。
const SCRIPT_MARK: char = '\u{2}';

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("固定的 regex")
}

static QUOTED: LazyLock<Regex> = LazyLock::new(|| re(r#"'[^']*'|"(?:[^"\\]|\\.)*""#));

static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| re("\u{1}(\\d+)\u{1}"));

/// C# `(?:^|[\s;&|(])(?:python3?|py|node|perl|ruby|php)(?:\.exe)?(?=\s|$)`（只用 IsMatch，環視改成擷取不影響結果）。
static INTERPRETER: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:^|[\s;&|(])(?:python3?|py|node|perl|ruby|php)(?:\.exe)?(?:\s|$)"));

static INLINE_SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:^|[\s;&|(])(?:python3?|py|node|perl|ruby|php)(?:\.exe)?\s+(?:-\S+\s+)*?-[ce]\s*$"));

/// 變數設定：bash 的 S="..."、PowerShell 的 $S = "..."。
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?:^|[;&|(\n]|\s)(?:export\s+)?(?:\$(?P<ps>\w+)\s*=\s*|(?P<sh>[A-Za-z_]\w*)=)(?P<value>[^\s;&|)]*)")
});

static VARIABLE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\$\{(\w+)\}|\$env:(\w+)|\$(\w+)"));

static CHANGE_DIR: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)(?:^|[;&|(\n])\s*(?:cd|Set-Location|pushd)\s+([^\s;&|)]+)"));

static TEE: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[\s;&|(])tee\s+(?:-\S+\s+)*([^\s;&|<>()]+)"));

/// C# 結尾是 `(?=[\s.=]|$)`：這裡改成擷取 `la`，下一次從 `la` 的位置繼續找（跟環視不吃字一樣）。
static IN_PLACE: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?:^|[\s;&|(])(?:sed|perl)\s(?:[^;&|\n]*?\s)?-(?:[a-zA-Z]*i[a-zA-Z]*|-in-place)(?P<la>[\s.=]|$)")
});

static WRITERS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)\b(?:Set-Content|Add-Content|Out-File|Clear-Content)\b|\[(?:System\.)?IO\.File\]::(?:Write|Append)|(?:^|[\s;&|(])(?:git\s+(?:apply|am|mv|rm)|patch)\s",
    )
});

/// 腳本裡寫檔：open(..., 'w')、write_text、writeFileSync、File.WriteAllText、os.replace…
static SCRIPT_WRITE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"\bopen\([^)\n]*(?:,\s*|mode\s*=\s*)['"][wax]|\.write_(?:text|bytes)\(|\bwriteFileSync\(|\bFile\.(?:WriteAll|AppendAll)\w*\(|\bos\.(?:replace|rename)\(|\bshutil\.(?:copy\w*|move)\("#,
    )
});

static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"^\w$"));

/// regex 的 `\w`（Unicode）。
fn is_word(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphanumeric() || c == '_';
    }
    let mut buf = [0u8; 4];
    WORD.is_match(c.encode_utf8(&mut buf))
}

/// 在 cwd 跑 command 會不會改到 root 底下的檔案。cwd 是那筆紀錄的工作目錄（沒有就當成 root）。
pub fn writes(command: &str, cwd: Option<&str>, root: &str) -> bool {
    // heredoc：本文換成標記（會被 python／node 執行、裡面又有寫檔才留 ScriptMark）
    let mut shell = String::with_capacity(command.len());
    let mut last = 0;
    for m in heredocs(command) {
        shell.push_str(&command[last..m.start]);
        let line_start = command[..m.start].rfind('\n').map_or(0, |p| p + 1);
        let runs =
            INTERPRETER.is_match(&command[line_start..m.start]) && SCRIPT_WRITE.is_match(&command[m.body.0..m.body.1]);
        shell.push_str(&command[m.rest.0..m.rest.1]);
        if runs {
            shell.push(' ');
            shell.push(SCRIPT_MARK);
        }
        last = m.end;
    }
    shell.push_str(&command[last..]);

    // 引號內容換成佔位符（-c 執行的腳本裡有寫檔就加上 ScriptMark）
    let mut quoted: Vec<String> = Vec::new();
    let original = shell.clone();
    let shell = QUOTED
        .replace_all(&original, |m: &Captures| {
            let all = &m[0];
            quoted.push(all[1..all.len() - 1].to_owned());
            let start = m.get(0).map_or(0, |g| g.start());
            let runs = INLINE_SCRIPT.is_match(&original[..start]) && SCRIPT_WRITE.is_match(all);
            format!("\u{1}{}\u{1}{}", quoted.len() - 1, if runs { " \u{2}" } else { "" })
        })
        .into_owned();

    let mut vars: HashMap<String, Option<String>> = HashMap::new();
    for m in ASSIGNMENT.captures_iter(&shell) {
        let name = m.name("ps").or_else(|| m.name("sh")).map_or("", |g| g.as_str());
        let value = expand(m.name("value").map_or("", |g| g.as_str()), &quoted, &vars);
        vars.insert(fold(name), value);
    }

    // 每個 cd 之後的所在資料夾；None 表示看不出來
    let start_dir = resolve(Some(root), cwd.unwrap_or(root)).unwrap_or_else(|| root.to_owned());
    let mut dirs: Vec<(usize, Option<String>)> = vec![(0, Some(start_dir))];
    for m in CHANGE_DIR.captures_iter(&shell) {
        let at = m.get(0).map_or(0, |g| g.start());
        let target = expand(m.get(1).map_or("", |g| g.as_str()), &quoted, &vars);
        let dir = match target {
            Some(t) => resolve(dirs.last().and_then(|d| d.1.as_deref()), &t),
            None => None,
        };
        dirs.push((at, dir));
    }
    let dir_at =
        |index: usize| -> Option<&str> { dirs.iter().rev().find(|d| d.0 <= index).and_then(|d| d.1.as_deref()) };

    let tees = TEE.captures_iter(&shell).filter_map(|m| {
        let at = m.get(0)?.start();
        Some((at, m.get(1)?.as_str().to_owned()))
    });
    for (at, path) in redirects(&shell).into_iter().chain(tees) {
        if let Some(path) = expand(&path, &quoted, &vars)
            && let Some(target) = resolve(dir_at(at), &path)
            && is_under(&target, root)
        {
            return true;
        }
    }

    let mut writes = in_place_starts(&shell);
    writes.extend(WRITERS.find_iter(&shell).map(|m| m.start()));
    writes.extend(shell.char_indices().filter(|&(_, c)| c == SCRIPT_MARK).map(|(i, _)| i));
    writes.into_iter().any(|i| dir_at(i).is_some_and(|dir| is_under(dir, root)))
}

/// 展開佔位符與變數；有不知道的變數回 None。
fn expand(s: &str, quoted: &[String], vars: &HashMap<String, Option<String>>) -> Option<String> {
    let s = PLACEHOLDER.replace_all(s, |m: &Captures| {
        m[1].parse::<usize>().ok().and_then(|i| quoted.get(i).cloned()).unwrap_or_default()
    });
    let mut unknown = false;
    let s = VARIABLE.replace_all(&s, |m: &Captures| {
        let name = m.get(1).or_else(|| m.get(2)).or_else(|| m.get(3)).map_or("", |g| g.as_str());
        let value = match vars.get(&fold(name)) {
            Some(v) => v.clone(),
            None => std::env::var(name).ok(),
        };
        unknown |= value.is_none();
        value.unwrap_or_default()
    });
    if unknown { None } else { Some(s.into_owned()) }
}

/// 把指令裡的路徑轉成完整路徑；~、/dev/null 這類看不出來或不是檔案的回 None。
fn resolve(base_dir: Option<&str>, path: &str) -> Option<String> {
    if path.is_empty()
        || path.starts_with(['~', '-', '%'])
        || path.contains('*')
        || path.contains('$')
        || path.starts_with("/dev/")
        || path.eq_ignore_ascii_case("nul")
    {
        return None;
    }
    let rooted = path.starts_with('/') || is_path_rooted(path);
    let full = if rooted { path.to_owned() } else { path_combine(base_dir?, path) };
    normalize(&full).ok()
}

/// `Path.IsPathRooted`（Windows）：\ 或 / 開頭，或「磁碟機代號:」。
pub(crate) fn is_path_rooted(path: &str) -> bool {
    let b = path.as_bytes();
    (!b.is_empty() && (b[0] == b'\\' || b[0] == b'/')) || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// `Path.Combine(a, b)`（b 不是根路徑）。
pub(crate) fn path_combine(a: &str, b: &str) -> String {
    if a.is_empty() {
        return b.to_owned();
    }
    if b.is_empty() {
        return a.to_owned();
    }
    if a.ends_with(['\\', '/']) || b.starts_with(['\\', '/']) { format!("{a}{b}") } else { format!("{a}\\{b}") }
}

/// 一段 heredoc（位元組位置）。
struct Heredoc {
    start: usize,
    end: usize,
    /// 第 3 組：`<<'EOF'` 之後同一行剩下的內容
    rest: (usize, usize),
    /// 第 4 組：本文
    body: (usize, usize),
}

/// C# `<<-?\s*(['"]?)(\w+)\1([^\n]*)\n(.*?)(?:\n\2[ \t]*(?=\n|$)|$)`（Singleline）的 Matches。
fn heredocs(s: &str) -> Vec<Heredoc> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match heredoc_at(s, &chars, i) {
            Some((m, end)) => {
                out.push(m);
                i = if end > i { end } else { i + 1 };
            }
            None => i += 1,
        }
    }
    out
}

fn heredoc_at(s: &str, chars: &[(usize, char)], i: usize) -> Option<(Heredoc, usize)> {
    let n = chars.len();
    let byte = |k: usize| if k < n { chars[k].0 } else { s.len() };
    let at = |k: usize| if k < n { Some(chars[k].1) } else { None };
    if at(i) != Some('<') || at(i + 1) != Some('<') {
        return None;
    }
    let mut j = i + 2;
    if at(j) == Some('-') {
        j += 1;
    }
    while at(j).is_some_and(char::is_whitespace) {
        j += 1;
    }
    let quote = match at(j) {
        Some(q @ ('\'' | '"')) => {
            j += 1;
            Some(q)
        }
        _ => None,
    };
    let delim_start = j;
    while at(j).is_some_and(is_word) {
        j += 1;
    }
    if j == delim_start {
        return None;
    }
    let delim = &s[byte(delim_start)..byte(j)];
    let delim_chars = j - delim_start;
    if let Some(q) = quote {
        if at(j) != Some(q) {
            return None;
        }
        j += 1;
    }
    let rest_start = j;
    while at(j).is_some_and(|c| c != '\n') {
        j += 1;
    }
    let rest_end = j;
    if at(j) != Some('\n') {
        return None;
    }
    j += 1;
    let body_start = j;
    // 本文 lazy：從最短的開始，先試結尾行，再試字串結尾（$：結尾或最後一個換行之前）
    for e in body_start..=n {
        if at(e) == Some('\n') && s[byte(e + 1)..].starts_with(delim) {
            let mut k = e + 1 + delim_chars;
            while matches!(at(k), Some(' ' | '\t')) {
                k += 1;
            }
            if k == n || at(k) == Some('\n') {
                return Some((
                    Heredoc {
                        start: byte(i),
                        end: byte(k),
                        rest: (byte(rest_start), byte(rest_end)),
                        body: (byte(body_start), byte(e)),
                    },
                    k,
                ));
            }
        }
        if e == n || (e + 1 == n && at(e) == Some('\n')) {
            return Some((
                Heredoc {
                    start: byte(i),
                    end: byte(e),
                    rest: (byte(rest_start), byte(rest_end)),
                    body: (byte(body_start), byte(e)),
                },
                e,
            ));
        }
    }
    None
}

/// C# `(?<![\w<>=\-$])(?:[12*]|&)?>>?\|?(?![&(>=])\s*([^\s;&|<>()]+)` 的 Matches：（起點, 第 1 組）。
fn redirects(s: &str) -> Vec<(usize, String)> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let n = chars.len();
    let byte = |k: usize| if k < n { chars[k].0 } else { s.len() };
    let at = |k: usize| if k < n { Some(chars[k].1) } else { None };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let found = (|| {
            if i > 0 {
                let p = chars[i - 1].1;
                if is_word(p) || matches!(p, '<' | '>' | '=' | '-' | '$') {
                    return None;
                }
            }
            let mut j = i;
            if matches!(at(j), Some('1' | '2' | '*' | '&')) {
                j += 1;
            }
            if at(j) != Some('>') {
                return None;
            }
            j += 1;
            if at(j) == Some('>') {
                j += 1;
            }
            if at(j) == Some('|') {
                j += 1;
            }
            if matches!(at(j), Some('&' | '(' | '>' | '=')) {
                return None;
            }
            while at(j).is_some_and(char::is_whitespace) {
                j += 1;
            }
            let start = j;
            while at(j).is_some_and(|c| !c.is_whitespace() && !matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')')) {
                j += 1;
            }
            if j == start {
                return None;
            }
            Some((j, s[byte(start)..byte(j)].to_owned()))
        })();
        match found {
            Some((end, target)) => {
                out.push((byte(i), target));
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

/// InPlace 的 Matches 起點（結尾的環視不吃字：下一次從環視的位置繼續）。
fn in_place_starts(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos <= s.len() {
        let Some(m) = IN_PLACE.captures_at(s, pos) else { break };
        let (Some(all), Some(la)) = (m.get(0), m.name("la")) else { break };
        out.push(all.start());
        pos = if la.start() > all.start() { la.start() } else { all.end().max(all.start() + 1) };
        // 落在多位元組字中間時往後挪到字的邊界
        while pos < s.len() && !s.is_char_boundary(pos) {
            pos += 1;
        }
    }
    out
}
