//! 對應 C# `Claude/HookInstaller.cs`。
//!
//! 在 repo 的 .claude/settings.local.json 加上 Claude Code hooks，事件發生時把 hook JSON 轉送給本機 HayakuBar。
//! 工具沒開時 curl 連不上，會直接 exit 0，不影響 Claude Code。

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};

const EVENTS: [&str; 3] = ["UserPromptSubmit", "Stop", "SessionEnd"];

pub struct HookInstaller;

impl HookInstaller {
    pub fn settings_path(repo: &str) -> PathBuf {
        Path::new(repo).join(".claude").join("settings.local.json")
    }

    fn marker(port: i32) -> String {
        format!("127.0.0.1:{port}/hook")
    }

    pub fn command(port: i32) -> String {
        format!(
            "curl -s -m 2 -X POST -H \"Content-Type: application/json\" --data-binary @- http://{} || exit 0",
            Self::marker(port)
        )
    }

    /// 三個事件都裝了這個連接埠的 hook。檔案讀不懂時回錯誤（C# 丟例外）。
    pub fn is_installed(repo: &str, port: i32) -> anyhow::Result<bool> {
        let root = load(&Self::settings_path(repo))?;
        Ok(root.as_ref().and_then(|r| r.get("hooks")).and_then(Value::as_object).is_some_and(|hooks| {
            EVENTS.iter().all(|ev| hooks.get(*ev).and_then(Value::as_array).is_some_and(|arr| contains_ours(arr, port)))
        }))
    }

    pub fn install(repo: &str, port: i32) -> anyhow::Result<()> {
        let path = Self::settings_path(repo);
        let mut root = load(&path)?.unwrap_or_default();
        if !root.get("hooks").is_some_and(Value::is_object) {
            root.insert("hooks".into(), Value::Object(Map::new()));
        }
        let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
            bail!("hooks 不是 JSON 物件");
        };
        for ev in EVENTS {
            if !hooks.get(ev).is_some_and(Value::is_array) {
                hooks.insert(ev.into(), Value::Array(Vec::new()));
            }
            let Some(arr) = hooks.get_mut(ev).and_then(Value::as_array_mut) else { continue };
            remove_ours(arr, port);
            arr.push(json!({
                "hooks": [{
                    "type": "command",
                    "command": Self::command(port),
                    "timeout": 5,
                }],
            }));
        }
        save(&path, &root)
    }

    pub fn uninstall(repo: &str, port: i32) -> anyhow::Result<()> {
        let path = Self::settings_path(repo);
        let Some(mut root) = load(&path)? else { return Ok(()) };
        let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else { return Ok(()) };
        for ev in EVENTS {
            let Some(arr) = hooks.get_mut(ev).and_then(Value::as_array_mut) else { continue };
            remove_ours(arr, port);
            if arr.is_empty() {
                hooks.shift_remove(ev);
            }
        }
        if hooks.is_empty() {
            root.shift_remove("hooks");
        }
        save(&path, &root)
    }
}

fn contains_ours(groups: &[Value], port: i32) -> bool {
    groups.iter().filter_map(Value::as_object).any(|g| {
        g.get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hs| hs.iter().filter_map(Value::as_object).any(|h| is_ours(h, port)))
    })
}

fn remove_ours(groups: &mut Vec<Value>, port: i32) {
    groups.retain_mut(|group| {
        let Some(hs) = group.as_object_mut().and_then(|g| g.get_mut("hooks")).and_then(Value::as_array_mut) else {
            return true;
        };
        hs.retain(|h| !h.as_object().is_some_and(|h| is_ours(h, port)));
        !hs.is_empty()
    });
}

fn is_ours(hook: &Map<String, Value>, port: i32) -> bool {
    hook.get("command").and_then(Value::as_str).is_some_and(|c| c.contains(&HookInstaller::marker(port)))
}

/// 讀設定檔；沒有或是空的回 None，不是 JSON 物件回錯誤（允許註解與結尾逗號，跟 C# 的讀法一樣）。
fn load(path: &Path) -> anyhow::Result<Option<Map<String, Value>>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).with_context(|| format!("讀取 {} 失敗", path.display()))?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let text = String::from_utf8_lossy(bytes);
    if text.trim().is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(&strip_comments_and_trailing_commas(&text))?;
    match value {
        Value::Object(map) => Ok(Some(map)),
        _ => bail!("{} 不是 JSON 物件", path.display()),
    }
}

fn save(path: &Path, root: &Map<String, Value>) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut backup = path.as_os_str().to_owned();
    backup.push(".hayakubar.bak");
    let backup = PathBuf::from(backup);
    if path.exists() && !backup.exists() {
        std::fs::copy(path, &backup)?;
    }
    let text = hb_backend::infra::json_file::to_string(root)?;
    std::fs::write(path, format!("{text}\r\n"))?;
    Ok(())
}

/// JsonCommentHandling.Skip ＋ AllowTrailingCommas：去掉字串外的 // 與 /* */ 註解，以及 } 或 ] 前多的逗號。
fn strip_comments_and_trailing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
                out.push(' ');
            }
            '}' | ']' => {
                // 往回找最後一個不是空白的字，是逗號就拿掉
                if let Some(pos) = out.iter().rposition(|ch| !ch.is_whitespace())
                    && out[pos] == ','
                {
                    out.remove(pos);
                }
                out.push(c);
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 註解與結尾逗號() {
        let text = "{ // 註解\n \"a\": [1, 2,], /* x */ \"b\": \"http://x//y\",\n}";
        let v: Value = serde_json::from_str(&strip_comments_and_trailing_commas(text)).unwrap();
        assert_eq!(json!({"a": [1, 2], "b": "http://x//y"}), v);
    }
}
