//! 移植自 `tests/HayakuBar.Tests`：ClaudeTests、AutoCreateTests（Claude 部分）、EstimateTests、SessionDigestTests、ShellEditsTests。

mod auto_create_tests;
mod claude_tests;
mod session_digest_tests;
mod shell_edits_tests;

use std::ffi::OsString;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// C# 測試的 `[Collection("env")]`：會改 CLAUDE_CONFIG_DIR 的測試不要平行跑。
pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// C# 的 TempDir。
pub(crate) fn temp_dir() -> TempDir {
    tempfile::Builder::new().prefix("hayakubar-test-").tempdir().unwrap()
}

pub(crate) fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// 暫時把 CLAUDE_CONFIG_DIR 指到暫存資料夾（離開時還原），測試不碰真正的 ~/.claude。
pub(crate) struct ClaudeDir {
    pub dir: TempDir,
    previous: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl ClaudeDir {
    pub fn new() -> Self {
        let guard = env_lock();
        let dir = temp_dir();
        let previous = std::env::var_os("CLAUDE_CONFIG_DIR");
        // SAFETY: 會改環境變數的測試都先拿 env_lock，同一時間只有一個在改
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir.path()) };
        Self { dir, previous, _guard: guard }
    }

    pub fn path(&self) -> String {
        path_str(self.dir.path())
    }

    pub fn projects(&self) -> std::path::PathBuf {
        self.dir.path().join("projects")
    }
}

impl Drop for ClaudeDir {
    fn drop(&mut self) {
        // SAFETY: 同上，還拿著 env_lock
        unsafe {
            match &self.previous {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
    }
}

/// C# `JsonSerializer.Serialize(string)`。
pub(crate) fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}
