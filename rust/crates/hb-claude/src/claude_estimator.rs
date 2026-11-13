//! 對應 C# `Claude/ClaudeEstimator.cs`。
//!
//! 建立任務時請 Claude 估預計工時：在背景跑 `claude -p`（用本機 Claude Code 的登入）。
//! 不存 session、不載入使用者設定（不觸發 hook）、不開工具與 MCP，執行目錄在 HayakuBar 自己的資料夾，
//! 所以不會被監看成一段新的 Claude 工作。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;

use hb_backend::infra::AppPaths;
use regex::Regex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TIMEOUT: Duration = Duration::from_secs(90);

const ARGS: &str = "-p --model haiku --no-session-persistence --tools \"\" --setting-sources \"\" --strict-mcp-config";

static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d+(?:\.\d+)?").expect("固定的 regex"));

pub struct ClaudeEstimator;

/// 還沒結束的 claude 程序：被取消（future 被丟棄）或逾時時連子程序一起結束（C# `Kill(entireProcessTree: true)`）。
struct KillTree {
    pid: Option<u32>,
}

impl Drop for KillTree {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(crate::CREATE_NO_WINDOW)
                .spawn();
        }
    }
}

impl ClaudeEstimator {
    /// 估預計工時（小時，0.5 為單位）；找不到 claude、逾時或回覆看不懂時回 None。
    /// 在 hb-backend 的 tokio 執行環境跑，可以在任何執行器 await；丟棄這個 future 就是取消（C# 的 CancellationToken）。
    pub async fn estimate(title: &str, project: Option<&str>, prompts: Option<&str>) -> Option<f64> {
        let Some(exe) = find_claude() else {
            log::info!("估預計工時：找不到 claude 指令");
            return None;
        };
        let title = title.to_owned();
        let input = Self::build_prompt(&title, project, prompts);
        hb_backend::rt::bg(async move {
            match tokio::time::timeout(TIMEOUT, run(&exe, &title, &input)).await {
                Ok(Ok(hours)) => hours,
                Ok(Err(err)) => {
                    log::error!("估預計工時失敗: {err}");
                    None
                }
                Err(_) => {
                    log::error!("估預計工時失敗: 逾時（{} 秒）", TIMEOUT.as_secs());
                    None
                }
            }
        })
        .await
    }

    pub fn build_prompt(title: &str, project: Option<&str>, prompts: Option<&str>) -> String {
        const NL: &str = "\r\n";
        let mut sb = String::new();
        sb.push_str(
            "你是軟體專案的工時估算助手。依下面的工作內容，估計一位熟悉這個專案的工程師（搭配 Claude Code）完成它，",
        );
        sb.push_str(NL);
        sb.push_str("包含實作、自我測試與修正，需要多少工時。");
        sb.push_str(NL);
        sb.push_str("只回一個數字，單位是小時，以 0.5 為單位，最少 0.5；不要任何其他文字。");
        sb.push_str(NL);
        sb.push_str(NL);
        if let Some(project) = project.filter(|p| !p.trim().is_empty()) {
            sb.push_str(&format!("專案：{project}{NL}"));
        }
        sb.push_str(&format!("任務名稱：{title}{NL}"));
        if let Some(prompts) = prompts.filter(|p| !p.trim().is_empty()) {
            sb.push_str(NL);
            sb.push_str(&format!("使用者給 Claude Code 的指令：{NL}"));
            sb.push_str(&format!("{prompts}{NL}"));
        }
        sb
    }

    /// 回覆裡的第一個數字，四捨五入到 0.5 小時（最少 0.5）；沒有數字或大得離譜回 None。
    pub fn parse_hours(text: Option<&str>) -> Option<f64> {
        let text = text.filter(|t| !t.trim().is_empty())?;
        let m = NUMBER.find(text)?;
        let hours: f64 = m.as_str().parse().ok()?;
        if hours <= 0.0 || hours > 1000.0 {
            return None;
        }
        // Math.Round(…, MidpointRounding.AwayFromZero)
        Some(((hours * 2.0).round() / 2.0).max(0.5))
    }
}

async fn run(exe: &Path, title: &str, input: &str) -> std::io::Result<Option<f64>> {
    let exe_text = exe.to_string_lossy();
    let lower = exe_text.to_lowercase();
    let mut cmd = if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        let mut c = tokio::process::Command::new("cmd.exe");
        c.raw_arg(format!("/d /s /c \"\"{exe_text}\" {ARGS}\""));
        c
    } else {
        let mut c = tokio::process::Command::new(exe);
        c.raw_arg(ARGS);
        c
    };
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(AppPaths::dir())
        .creation_flags(crate::CREATE_NO_WINDOW);
    let mut child = cmd.spawn()?;
    let mut guard = KillTree { pid: child.id() };

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).await?;
        stdin.shutdown().await?;
        drop(stdin);
    }
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let read_out = async {
        let mut buf = Vec::new();
        if let Some(s) = stdout.as_mut() {
            s.read_to_end(&mut buf).await?;
        }
        Ok::<_, std::io::Error>(buf)
    };
    let read_err = async {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            s.read_to_end(&mut buf).await?;
        }
        Ok::<_, std::io::Error>(buf)
    };
    let (out, err, status) = tokio::join!(read_out, read_err, child.wait());
    let status = status?;
    guard.pid = None; // 已經結束，不用再殺
    let text = String::from_utf8_lossy(&out?).into_owned();
    let error = String::from_utf8_lossy(&err.unwrap_or_default()).into_owned();
    if !status.success() {
        let code = status.code().map_or_else(|| "?".to_owned(), |c| c.to_string());
        log::error!("估預計工時：claude 結束碼 {code}：{}", error.trim());
        return Ok(None);
    }
    let hours = ClaudeEstimator::parse_hours(Some(&text));
    let described = match hours {
        Some(h) => format!("{h} 小時"),
        None => format!("看不懂的回覆「{}」", text.trim()),
    };
    log::info!("估預計工時：「{title}」→ {described}");
    Ok(hours)
}

/// PATH 裡的 claude（原生安裝的 claude.exe 優先，其次 npm 的 claude.cmd），再看預設安裝位置。
fn find_claude() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .filter(|d| !d.is_empty())
        .map(|d| PathBuf::from(d.trim_matches('"')))
        .collect();
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(profile).join(".local").join("bin"));
    }
    if let Some(app_data) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(app_data).join("npm"));
    }
    for name in ["claude.exe", "claude.cmd"] {
        for dir in &dirs {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}
