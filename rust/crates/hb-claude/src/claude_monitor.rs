//! 對應 C# `Claude/ClaudeMonitor.cs`。
//!
//! 監看設定的主資料夾（與裡面的專案）的 Claude Code 活動。CLI 和桌面版都寫在
//! ~/.claude/projects/<編碼路徑>/*.jsonl（每行都有 cwd、gitBranch、timestamp、entrypoint），hooks 只是讓反應更即時。
//! 資料夾名稱是「開 Claude 時所在路徑」把非英數字元換成 '-'，中文路徑會撞名，所以一律用檔案裡的 cwd 決定歸屬：
//! session 算給「包含它的最深一層」單位，子專案的 session 不會重複算進主資料夾。
//!
//! 跟 C# 一樣只在 UI 執行緒使用；檔案掃描同步進行。

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use chrono::{Local, NaiveDateTime, TimeDelta};
use hb_api::KeyFinder;
use hb_backend::config::{IndexMap, RepoConfig};
use regex::Regex;
use serde_json::Value;

use crate::session_digest;
use crate::shell_edits;
use crate::util::{
    FileEntry, eq_ic, flag, fold, jsonl_files, max_by_first, newest, now, read_head, starts_with_ic, str_prop, subdirs,
    utf8, utf16_len,
};

const INITIAL_TAIL_BYTES: u64 = 512 * 1024;
const MAX_READ_BYTES: u64 = 1024 * 1024;
const HEAD_BYTES: u64 = 256 * 1024;
const QUICK_HEAD_BYTES: u64 = 32 * 1024;
const RECENT_FILES_DAYS: i64 = 7;
const DETECT_CACHE_MINUTES: i64 = 2;
const INDEX_CACHE_SECONDS: i64 = 60;
const SKIP_FOLDERS: [&str; 5] = ["node_modules", "bin", "obj", "dist", "build"];

/// 會改檔的工具。
const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// 跑指令的工具：多半是看東西，指令真的會在專案裡寫檔（sed -i、python 改檔…）才算改檔，見 ShellEdits。
const SHELL_TOOLS: [&str; 2] = ["Bash", "PowerShell"];

/// 判斷任務編號是否存在（C# `Func<string, bool>`）。背景整理日誌時也會用到，所以要能跨執行緒。
pub type KnownKey = Arc<dyn Fn(&str) -> bool + Send + Sync>;

static POSIX_DRIVE_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^/(?:mnt/)?([a-zA-Z])(?:/(.*))?$").expect("固定的 regex"));
static CWD_FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""cwd":"((?:[^"\\]|\\.)*)""#).expect("固定的 regex"));
static ENTRYPOINT_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""entrypoint":"([^"]*)""#).expect("固定的 regex"));

/// 一個監看單位：主資料夾本身，或主資料夾裡偵測到的某個專案。
#[derive(Debug, Clone, PartialEq)]
pub struct RepoActivity {
    pub path: String,

    /// 所屬的主資料夾；本身就是主資料夾時為 None。
    pub root_path: Option<String>,

    pub enabled: bool,

    /// Claude 改了這裡的程式、但對應不到任務時，詢問要不要建立任務。
    pub auto_create: bool,

    /// 最後活動時間；沒有活動是 `NaiveDateTime::MIN`（C# `DateTime.MinValue`）。
    pub last_activity: NaiveDateTime,
    pub ended_at: Option<NaiveDateTime>,
    pub session_id: Option<String>,
    pub session_file: Option<String>,
    pub file_offset: u64,
    pub branch: Option<String>,
    pub prompt_key: Option<String>,

    /// 這個 session 最近一次在本單位裡改檔（Edit、Write…）的時間；只是問問題、規劃不會有。
    pub edited_at: Option<NaiveDateTime>,

    /// 這個 session 第一句有內容的指令摘要，當新任務名稱的建議。
    pub first_prompt: Option<String>,

    /// 最近一筆紀錄的來源：cli 或 claude-desktop。
    pub entrypoint: Option<String>,

    pub active: bool,
    pub resolved_task: Option<String>,
    pub task_source: Option<String>,
}

impl RepoActivity {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            root_path: None,
            enabled: true,
            auto_create: false,
            last_activity: NaiveDateTime::MIN,
            ended_at: None,
            session_id: None,
            session_file: None,
            file_offset: 0,
            branch: None,
            prompt_key: None,
            edited_at: None,
            first_prompt: None,
            entrypoint: None,
            active: false,
            resolved_task: None,
            task_source: None,
        }
    }

    /// `Path.GetFileName(Path.TrimEnd('\\', '/'))`。
    pub fn name(&self) -> &str {
        file_name(self.path.trim_end_matches(['\\', '/']))
    }

    pub fn source_text(&self) -> String {
        ClaudeMonitor::source_text(self.entrypoint.as_deref())
    }
}

/// 最近有 Claude Code session 的資料夾（給設定頁快速加入 repo 用）。
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredProject {
    pub path: String,
    pub last_activity: NaiveDateTime,
    pub source: String,
}

/// 主資料夾裡偵測到的專案。
#[derive(Debug, Clone, PartialEq)]
pub struct DetectedProject {
    pub path: String,
    pub name: String,
    pub is_git: bool,
    pub last_session: Option<NaiveDateTime>,
    pub source: String,
}

#[derive(Debug, Clone)]
struct SessionInfo {
    root: String,
    last_write: NaiveDateTime,
    entrypoint: Option<String>,
}

/// session 檔開頭的 cwd 與 entrypoint。
type HeadInfo = (String, Option<String>);

pub struct ClaudeMonitor {
    /// 鍵：路徑的 OrdinalIgnoreCase 比較鍵
    repos: IndexMap<String, RepoActivity>,

    /// session 檔 → 開頭的 cwd（開 session 時的資料夾）與 entrypoint；確定後不會變，快取起來。（鍵：比較鍵；值含原本的路徑）
    session_heads: HashMap<String, (String, HeadInfo)>,

    detect_cache: HashMap<String, (NaiveDateTime, Vec<DetectedProject>)>,
    index: Option<(NaiveDateTime, Vec<SessionInfo>)>,

    pub is_known_key: KnownKey,
}

impl Default for ClaudeMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// `Path.GetFileName`：最後一個 \、/、: 之後。
fn file_name(path: &str) -> &str {
    match path.rfind(['\\', '/', ':']) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// 路徑轉換失敗（C# `Path.GetFullPath` 丟的 ArgumentException：空字串、含 NUL…）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPath;

impl std::fmt::Display for InvalidPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("路徑格式不正確")
    }
}

impl std::error::Error for InvalidPath {}

/// 統一成 Windows 路徑；Git Bash 格式（/d/repo/x、/mnt/d/repo/x）也轉成 D:\repo\x。
pub fn normalize(path: &str) -> Result<String, InvalidPath> {
    let mut p = path.trim().to_owned();
    if let Some(m) = POSIX_DRIVE_PATH.captures(&p) {
        let drive = m[1].to_ascii_uppercase();
        let rest = m.get(2).map_or("", |g| g.as_str()).replace('/', "\\");
        p = format!("{drive}:\\{rest}");
    }
    // Path.GetFullPath：Windows 上同樣是 GetFullPathNameW
    let full = std::path::absolute(&p).map_err(|_| InvalidPath)?;
    Ok(full.to_string_lossy().trim_end_matches(['\\', '/']).to_owned())
}

/// path 是 root 本身或在 root 底下（不分大小寫）。
pub fn is_under(path: &str, root: &str) -> bool {
    eq_ic(path, root) || starts_with_ic(path, &format!("{root}\\"))
}

/// Claude Code 的專案資料夾命名：路徑中非英數字元都換成 '-'（D:\sj\hayaku tool → D--sj-hayaku-tool）。
/// 以 UTF-16 單位計（BMP 以外的字變成兩個 '-'，跟 Claude Code 一樣）。
pub fn encode(path: &str) -> Result<String, InvalidPath> {
    Ok(normalize(path)?
        .chars()
        .flat_map(|c| {
            let n = if c.is_ascii_alphanumeric() { 1 } else { c.len_utf16() };
            std::iter::repeat_n(if c.is_ascii_alphanumeric() { c } else { '-' }, n)
        })
        .collect())
}

fn is_git_repo(path: &str) -> bool {
    Path::new(path).join(".git").exists()
}

fn is_skip_folder(name: &str) -> bool {
    SKIP_FOLDERS.iter().any(|s| eq_ic(s, name))
}

impl ClaudeMonitor {
    pub fn new() -> Self {
        Self {
            repos: IndexMap::new(),
            session_heads: HashMap::new(),
            detect_cache: HashMap::new(),
            index: None,
            is_known_key: Arc::new(|_| true),
        }
    }

    /// 所有監看單位（主資料夾＋偵測到的專案）。
    pub fn repos(&self) -> impl Iterator<Item = &RepoActivity> {
        self.repos.values()
    }

    pub fn repos_mut(&mut self) -> impl Iterator<Item = &mut RepoActivity> {
        self.repos.values_mut()
    }

    /// 用路徑找監看單位（不分大小寫）。
    pub fn repo(&self, path: &str) -> Option<&RepoActivity> {
        self.repos.get(&fold(path))
    }

    /// Claude Code 的 projects 資料夾（CLAUDE_CONFIG_DIR 或 ~/.claude 底下）。
    pub fn projects_root() -> PathBuf {
        let config_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
        let root = match config_dir {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => user_profile().join(".claude"),
        };
        root.join("projects")
    }

    pub fn source_text(entrypoint: Option<&str>) -> String {
        match entrypoint {
            Some("cli") => "CLI".into(),
            Some("claude-desktop") => "桌面版".into(),
            None | Some("") => String::new(),
            Some(other) => other.into(),
        }
    }

    /// 見 [`normalize`]。
    pub fn normalize(path: &str) -> Result<String, InvalidPath> {
        normalize(path)
    }

    /// 見 [`is_under`]。
    pub fn is_under(path: &str, root: &str) -> bool {
        is_under(path, root)
    }

    /// 見 [`encode`]。
    pub fn encode(path: &str) -> Result<String, InvalidPath> {
        encode(path)
    }

    // ───────────── 監看單位 ─────────────

    /// 依設定建立監看單位：每個主資料夾，加上（開啟自動偵測時）裡面的每個專案。
    pub fn sync(&mut self, repos: &[RepoConfig]) {
        enum Apply {
            Root { enabled: bool, auto_create: bool },
            Project { root: String, enabled: bool, auto_create: bool },
        }
        let configs: Vec<&RepoConfig> = repos.iter().filter(|r| !r.path.trim().is_empty()).collect();
        let mut wanted: IndexMap<String, (String, Apply)> = IndexMap::new();
        // C# `wanted[path] = …`：已有的鍵保留原本的寫法與位置
        fn set(wanted: &mut IndexMap<String, (String, Apply)>, path: String, apply: Apply) {
            match wanted.get_mut(&fold(&path)) {
                Some(slot) => slot.1 = apply,
                None => {
                    wanted.insert(fold(&path), (path, apply));
                }
            }
        }

        for repo in &configs {
            let Ok(path) = normalize(&repo.path) else {
                log::error!("監看資料夾路徑不正確：{}", repo.path);
                continue;
            };
            set(&mut wanted, path, Apply::Root { enabled: repo.enabled, auto_create: repo.auto_create });
        }
        for repo in configs.iter().filter(|r| r.include_subprojects) {
            let Ok(root) = normalize(&repo.path) else { continue };
            for project in self.detect_projects(&root, false) {
                if wanted.contains_key(&fold(&project.path)) {
                    continue; // 使用者另外單獨加的資料夾，以它自己的設定為準
                }
                let custom = repo.projects.iter().find(|p| eq_ic(&p.name, &project.name));
                set(
                    &mut wanted,
                    project.path.clone(),
                    Apply::Project {
                        root: root.clone(),
                        enabled: repo.enabled && custom.is_none_or(|c| c.enabled),
                        auto_create: custom.is_some_and(|c| c.auto_create),
                    },
                );
            }
        }

        for (key, (path, apply)) in &wanted {
            let activity = self.repos.entry(key.clone()).or_insert_with(|| RepoActivity::new(path.clone()));
            match apply {
                Apply::Root { enabled, auto_create } => {
                    activity.root_path = None;
                    activity.enabled = *enabled;
                    activity.auto_create = *auto_create;
                }
                Apply::Project { root, enabled, auto_create } => {
                    activity.root_path = Some(root.clone());
                    activity.enabled = *enabled;
                    activity.auto_create = *auto_create;
                }
            }
        }
        self.repos.retain(|k, _| wanted.contains_key(k));
    }

    /// 主資料夾裡的專案：git repo 的子資料夾，以及 Claude 曾在裡面開過 session 的子資料夾。
    /// 主資料夾本身是 git repo 時，只認巢狀的 git repo（一般子資料夾的 session 算主資料夾的）。
    pub fn detect_projects(&mut self, root: &str, refresh: bool) -> Vec<DetectedProject> {
        let Ok(root) = normalize(root) else { return Vec::new() };
        let key = fold(&root);
        if !refresh
            && let Some((at, projects)) = self.detect_cache.get(&key)
            && now() - *at < TimeDelta::minutes(DETECT_CACHE_MINUTES)
        {
            return projects.clone();
        }

        let mut found: IndexMap<String, DetectedProject> = IndexMap::new();
        if !Path::new(&root).is_dir() {
            return Vec::new();
        }
        let root_is_git = is_git_repo(&root);
        match subdirs(Path::new(&root)) {
            Ok(dirs) => {
                for dir in dirs {
                    let name = file_name(&dir).to_owned();
                    if name.starts_with('.') || is_skip_folder(&name) {
                        continue;
                    }
                    if is_git_repo(&dir) {
                        found.insert(
                            fold(&dir),
                            DetectedProject {
                                path: dir,
                                name,
                                is_git: true,
                                last_session: None,
                                source: String::new(),
                            },
                        );
                    }
                }
            }
            Err(err) => log::error!("列出 {root} 失敗: {err}"),
        }

        for session in self.session_index(refresh) {
            if !is_under(&session.root, &root) || utf16_len(&session.root) == utf16_len(&root) {
                continue;
            }
            let rest: String = session.root.chars().skip(root.chars().count() + 1).collect();
            let name = rest.split('\\').next().unwrap_or("").to_owned();
            let child = format!("{root}\\{name}");
            if name.starts_with('.') || is_skip_folder(&name) {
                continue;
            }
            let mut project = match found.get(&fold(&child)) {
                Some(p) => p.clone(),
                None => {
                    if root_is_git || !Path::new(&child).is_dir() {
                        continue;
                    }
                    DetectedProject {
                        path: child.clone(),
                        is_git: is_git_repo(&child),
                        name,
                        last_session: None,
                        source: String::new(),
                    }
                }
            };
            if project.last_session.is_none_or(|l| session.last_write > l) {
                project.last_session = Some(session.last_write);
                project.source = Self::source_text(session.entrypoint.as_deref());
            }
            found.insert(fold(&child), project);
        }

        let mut list: Vec<DetectedProject> = found.into_values().collect();
        list.sort_by(|a, b| {
            b.last_session
                .unwrap_or(NaiveDateTime::MIN)
                .cmp(&a.last_session.unwrap_or(NaiveDateTime::MIN))
                .then_with(|| fold(&a.name).cmp(&fold(&b.name)))
        });
        self.detect_cache.insert(key, (now(), list.clone()));
        list
    }

    /// 每個 session 資料夾最新的一個檔案：開在哪、最後寫入時間、CLI 或桌面版。
    fn session_index(&mut self, refresh: bool) -> Vec<SessionInfo> {
        if !refresh
            && let Some((at, sessions)) = &self.index
            && now() - *at < TimeDelta::seconds(INDEX_CACHE_SECONDS)
        {
            return sessions.clone();
        }
        let mut sessions = Vec::new();
        let root = Self::projects_root();
        if root.is_dir() {
            for dir in subdirs(&root).unwrap_or_default() {
                let Ok(files) = jsonl_files(Path::new(&dir)) else { continue };
                if let Some(newest) = newest(files)
                    && let Some((cwd, entrypoint)) = self.head(&newest)
                {
                    sessions.push(SessionInfo { root: cwd, last_write: newest.last_write, entrypoint });
                }
            }
        }
        self.index = Some((now(), sessions.clone()));
        sessions
    }

    /// 清暫存：已刪除或超過 7 天沒動的 session 檔的開頭資訊、專案偵測與 session 索引（下次用到再重建）。
    pub fn trim_caches(&mut self) {
        let since = now() - TimeDelta::days(RECENT_FILES_DAYS);
        self.session_heads
            .retain(|_, (file, _)| FileEntry::from_path(Path::new(file)).is_some_and(|info| info.last_write >= since));
        self.detect_cache.clear();
        self.index = None;
    }

    // ───────────── 掃描 session ─────────────

    pub fn scan(&mut self) {
        let root = Self::projects_root();
        if !root.is_dir() || self.repos.is_empty() {
            return;
        }
        let dirs = match subdirs(&root) {
            Ok(d) => d,
            Err(err) => {
                log::error!("列出 {} 失敗: {err}", root.display());
                return;
            }
        };

        let units: Vec<(String, String)> = self.repos.iter().map(|(k, a)| (k.clone(), a.path.clone())).collect();
        let encoded: Vec<(String, String)> =
            units.iter().filter_map(|(k, p)| encode(p).ok().map(|e| (k.clone(), e))).collect();
        let mut newest_files: IndexMap<String, FileEntry> = IndexMap::new();
        let since = now() - TimeDelta::days(RECENT_FILES_DAYS);

        for dir in dirs {
            let dir_name = file_name(&dir).to_owned();
            let candidates: Vec<&(String, String)> = encoded
                .iter()
                .filter(|(_, name)| eq_ic(&dir_name, name) || starts_with_ic(&dir_name, &format!("{name}-")))
                .collect();
            if candidates.is_empty() {
                continue;
            }
            let exact_unit = candidates.iter().find(|(_, name)| eq_ic(&dir_name, name)).map(|(k, _)| k.clone());
            let files = match jsonl_files(Path::new(&dir)) {
                Ok(f) => f,
                Err(err) => {
                    log::error!("列出 {dir} 失敗: {err}");
                    continue;
                }
            };
            for file in files {
                if file.last_write < since {
                    continue;
                }
                // 歸給包含 session 起始資料夾的最深一層單位；剛開的 session 還沒有 cwd 時先看資料夾名稱
                let owner = match self.head(&file) {
                    Some((cwd, _)) => max_by_first(units.iter().filter(|(_, p)| is_under(&cwd, p)), |(_, p)| p.len())
                        .map(|(k, _)| k.clone()),
                    None => exact_unit.clone(),
                };
                if let Some(owner) = owner
                    && newest_files.get(&owner).is_none_or(|current| file.last_write > current.last_write)
                {
                    newest_files.insert(owner, file);
                }
            }
        }

        for (key, file) in newest_files {
            self.update(&key, &file);
        }
    }

    fn update(&mut self, key: &str, file: &FileEntry) {
        let changed =
            self.repos.get(key).is_some_and(|a| !a.session_file.as_deref().is_some_and(|f| eq_ic(f, &file.full_name)));
        let head = if changed { self.head(file) } else { None };
        let is_known = self.is_known_key.clone();
        let Some(activity) = self.repos.get_mut(key) else { return };
        if changed {
            activity.session_file = Some(file.full_name.clone());
            activity.session_id = Some(file.stem());
            activity.prompt_key = None;
            activity.branch = None;
            activity.edited_at = None;
            activity.first_prompt = read_first_prompt(Path::new(&file.full_name));
            activity.entrypoint = head.and_then(|h| h.1);
            activity.file_offset = file.length.saturating_sub(INITIAL_TAIL_BYTES);
        }

        let write = file.last_write;
        if write > activity.last_activity && activity.ended_at.is_none_or(|e| write > e + TimeDelta::seconds(5)) {
            activity.last_activity = write;
            activity.ended_at = None;
        }

        if file.length < activity.file_offset {
            activity.file_offset = 0;
        }
        if file.length > activity.file_offset {
            read_new_lines(activity, file, is_known.as_ref());
        }
    }

    fn head(&mut self, file: &FileEntry) -> Option<HeadInfo> {
        let key = fold(&file.full_name);
        if let Some((_, cached)) = self.session_heads.get(&key) {
            return Some(cached.clone());
        }
        let (cwd, entrypoint) = read_head_info(Path::new(&file.full_name));
        let cwd = cwd?;
        self.session_heads.insert(key, (file.full_name.clone(), (cwd.clone(), entrypoint.clone())));
        Some((cwd, entrypoint))
    }

    /// 讀 session 開頭幾句有內容的指令原文（不截成摘要），給 Claude 估預計工時用；合計最多 max_chars 字。
    /// 指令、系統訊息、「繼續」這類回覆跳過。（C# 預設 maxPrompts = 3、maxChars = 2000，見 [`Self::read_opening_prompts`]）
    pub fn read_opening_prompts_with(path: &Path, max_prompts: usize, max_chars: usize) -> Option<String> {
        let bytes = read_head(path, HEAD_BYTES).ok()?;
        let mut prompts: Vec<String> = Vec::new();
        for line in utf8(&bytes).split('\n') {
            if utf16_len(line) < 2 {
                continue;
            }
            // 讀到上限時最後一行不完整
            if let Ok(doc) = serde_json::from_str::<Value>(line)
                && let Some(text) = user_prompt(&doc)
                && session_digest::summarize(Some(&text)).is_some()
            {
                prompts.push(session_digest::strip_image_tags(&text).trim().to_owned());
            }
            if prompts.len() >= max_prompts {
                break;
            }
        }
        if prompts.is_empty() {
            return None;
        }
        let all = prompts.join("\n---\n");
        Some(if utf16_len(&all) > max_chars { format!("{}…", crate::util::left_utf16(&all, max_chars)) } else { all })
    }

    /// [`Self::read_opening_prompts_with`]，最多 3 句、2000 字。
    pub fn read_opening_prompts(path: &Path) -> Option<String> {
        Self::read_opening_prompts_with(path, 3, 2000)
    }

    /// 列出最近有 Claude Code session 的資料夾（CLI 與桌面版都算），排除桌面版的暫存工作區。
    pub fn discover(max: usize) -> Vec<DiscoveredProject> {
        let root = Self::projects_root();
        let mut found: IndexMap<String, DiscoveredProject> = IndexMap::new();
        if !root.is_dir() {
            return Vec::new();
        }
        fn is_desktop_scratch(path: &str) -> bool {
            use crate::util::contains_ic;
            contains_ic(path, "\\scratch-workspaces\\")
                || contains_ic(path, "\\AppData\\Roaming\\Claude\\")
                || contains_ic(path, "\\AppData\\Local\\Claude\\")
        }
        for dir in subdirs(&root).unwrap_or_default() {
            let Ok(files) = jsonl_files(Path::new(&dir)) else { continue };
            let Some(newest) = newest(files) else { continue };
            let (cwd, entrypoint) = read_head_info(Path::new(&newest.full_name));
            let Some(cwd) = cwd else { continue };
            if is_desktop_scratch(&cwd) || !Path::new(&cwd).is_dir() {
                continue;
            }
            let key = fold(&cwd);
            if found.get(&key).is_none_or(|existing| existing.last_activity < newest.last_write) {
                found.insert(
                    key,
                    DiscoveredProject {
                        path: cwd,
                        last_activity: newest.last_write,
                        source: Self::source_text(entrypoint.as_deref()),
                    },
                );
            }
        }
        let mut list: Vec<DiscoveredProject> = found.into_values().collect();
        list.sort_by_key(|p| std::cmp::Reverse(p.last_activity));
        list.truncate(max);
        list
    }

    /// Claude 在 root 底下改檔的那一筆紀錄 → 時間；改的是外面的檔案（例如 plan mode 寫的計畫檔）不算。
    pub fn edit_time(r: &Value, root: &str) -> Option<NaiveDateTime> {
        if str_prop(r, "type") != Some("assistant") {
            return None;
        }
        let content = r.get("message")?.as_object()?.get("content")?.as_array()?;
        for part in content {
            if !part.is_object() || str_prop(part, "type") != Some("tool_use") {
                continue;
            }
            let Some(name) = str_prop(part, "name") else { continue };
            let Some(input) = part.get("input").filter(|i| i.is_object()) else { continue };
            if SHELL_TOOLS.contains(&name) {
                match str_prop(input, "command") {
                    Some(command) if !command.is_empty() && shell_edits::writes(command, str_prop(r, "cwd"), root) => {}
                    _ => continue,
                }
            } else {
                if !EDIT_TOOLS.contains(&name) {
                    continue;
                }
                let file = str_prop(input, "file_path").or_else(|| str_prop(input, "notebook_path"));
                let Some(file) = file.filter(|f| !f.is_empty()) else { continue };
                match normalize(file) {
                    Ok(full) if is_under(&full, root) => {}
                    _ => continue,
                }
            }
            return Some(
                str_prop(r, "timestamp")
                    .and_then(hb_api::json::parse_date_time_offset)
                    .map(|at| at.with_timezone(&Local).naive_local())
                    .unwrap_or_else(now),
            );
        }
        None
    }

    /// 處理 Claude Code hook 送來的事件；對應到監看單位才回傳 true。
    pub fn on_hook(&mut self, payload: &Value, now: NaiveDateTime) -> bool {
        if !payload.is_object() {
            return false;
        }
        let Some(cwd) = str_prop(payload, "cwd") else { return false };
        let Ok(normalized) = normalize(cwd) else { return false };

        let Some(key) =
            max_by_first(self.repos.iter().filter(|(_, u)| is_under(&normalized, &u.path)), |(_, u)| u.path.len())
                .map(|(k, _)| k.clone())
        else {
            return false;
        };
        let is_known = self.is_known_key.clone();
        let Some(activity) = self.repos.get_mut(&key) else { return false };

        if let Some(sid) = str_prop(payload, "session_id")
            && activity.session_id.as_deref() != Some(sid)
        {
            activity.session_id = Some(sid.to_owned());
            activity.prompt_key = None;
            activity.edited_at = None;
            activity.first_prompt = None;
        }

        match str_prop(payload, "hook_event_name") {
            Some("SessionStart") => {}
            Some("SessionEnd") => activity.ended_at = Some(now),
            Some("UserPromptSubmit") => {
                activity.last_activity = now;
                activity.ended_at = None;
                let prompt = str_prop(payload, "prompt");
                if let Some(key) = KeyFinder::find_in_prompt(prompt, |k| is_known(k)) {
                    activity.prompt_key = Some(key);
                }
                if activity.first_prompt.is_none() {
                    activity.first_prompt = session_digest::summarize(prompt);
                }
            }
            _ => {
                activity.last_activity = now;
                activity.ended_at = None;
            }
        }
        true
    }

    /// 判斷各單位是否活動中，並決定對應的任務（手動指定 → 對話內容 → 分支 → 專案／主資料夾預設）。
    pub fn resolve(&mut self, now: NaiveDateTime, idle_minutes: i32, session_tasks: &IndexMap<String, String>) {
        let idle = TimeDelta::minutes(i64::from(idle_minutes.max(1)));
        let is_known = self.is_known_key.clone();
        for a in self.repos.values_mut() {
            a.active = a.enabled
                && a.ended_at.is_none()
                && a.last_activity != NaiveDateTime::MIN
                && now - a.last_activity < idle;

            let (task, source) = if let Some(picked) = a.session_id.as_ref().and_then(|sid| session_tasks.get(sid)) {
                (Some(picked.clone()), Some("手動指定".to_owned()))
            } else if let Some(prompt_key) = &a.prompt_key {
                (Some(prompt_key.clone()), Some("對話內容".to_owned()))
            } else if let Some(branch_key) = KeyFinder::find_in_branch(a.branch.as_deref(), |k| is_known(k)) {
                (Some(branch_key), Some(format!("分支 {}", a.branch.as_deref().unwrap_or(""))))
            } else {
                // 沒有預設任務：偵測不到任務編號就不計時
                (None, None)
            };
            a.resolved_task = task;
            a.task_source = source;
        }
    }
}

/// `Environment.GetFolderPath(SpecialFolder.UserProfile)`。
fn user_profile() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).or_else(std::env::home_dir).unwrap_or_default()
}

/// 讀檔案開頭，取第一筆 cwd 與 entrypoint；cwd 通常在前幾行，先讀 32KB，找不到才讀 256KB。
fn read_head_info(path: &Path) -> (Option<String>, Option<String>) {
    let quick = read_head_info_bytes(path, QUICK_HEAD_BYTES);
    if quick.0.is_some() { quick } else { read_head_info_bytes(path, HEAD_BYTES) }
}

fn read_head_info_bytes(path: &Path, bytes: u64) -> (Option<String>, Option<String>) {
    let Ok(buffer) = read_head(path, bytes) else { return (None, None) };
    let text = utf8(&buffer);
    let mut cwd = None;
    if let Some(m) = CWD_FIELD.captures(&text) {
        let Ok(raw) = serde_json::from_str::<String>(&format!("\"{}\"", &m[1])) else {
            return (None, None);
        };
        if !raw.trim().is_empty() {
            match normalize(&raw) {
                Ok(n) => cwd = Some(n),
                Err(_) => return (None, None),
            }
        }
    }
    let ep = ENTRYPOINT_FIELD.captures(&text).map(|m| m[1].to_owned());
    (cwd, ep)
}

/// 讀檔案開頭，取第一句有內容的指令摘要（新任務名稱的建議）。
fn read_first_prompt(path: &Path) -> Option<String> {
    let bytes = read_head(path, HEAD_BYTES).ok()?;
    for line in utf8(&bytes).split('\n') {
        if utf16_len(line) < 2 {
            continue;
        }
        // 讀到上限時最後一行不完整
        if let Ok(doc) = serde_json::from_str::<Value>(line)
            && let Some(text) = user_prompt(&doc)
            && let Some(summary) = session_digest::summarize(Some(&text))
        {
            return Some(summary);
        }
    }
    None
}

/// 使用者自己送出的指令文字（排除系統插入的 meta 訊息與子代理）；不是的話回 None。
fn user_prompt(r: &Value) -> Option<String> {
    if !r.is_object() || str_prop(r, "type") != Some("user") || flag(r, "isMeta") || flag(r, "isSidechain") {
        return None;
    }
    let content = r.get("message")?.as_object()?.get("content")?;
    prompt_text(content)
}

/// 訊息內容：字串，或陣列裡各段 text（每段後面加換行，跟 C# `AppendLine` 一樣是 CRLF）。
fn prompt_text(content: &Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        return Some(s.to_owned());
    }
    let parts = content.as_array()?;
    let mut sb = String::new();
    for part in parts {
        if str_prop(part, "type") == Some("text")
            && let Some(text) = part.get("text")
        {
            sb.push_str(text.as_str().unwrap_or(""));
            sb.push_str("\r\n");
        }
    }
    Some(sb)
}

fn read_new_lines(activity: &mut RepoActivity, file: &FileEntry, is_known: &(dyn Fn(&str) -> bool + Send + Sync)) {
    let result = (|| -> std::io::Result<()> {
        let mut fs = std::fs::File::open(&file.full_name)?;
        let file_len = fs.metadata()?.len();
        let mut length = file_len.saturating_sub(activity.file_offset);
        if length > MAX_READ_BYTES {
            // 落後太多（例如剛開工具）：只看最後一段
            activity.file_offset = file_len - INITIAL_TAIL_BYTES;
            length = INITIAL_TAIL_BYTES;
        }
        fs.seek(SeekFrom::Start(activity.file_offset))?;
        let mut buffer = Vec::new();
        (&mut fs).take(length).read_to_end(&mut buffer)?;
        let Some(last_newline) = buffer.iter().rposition(|&b| b == b'\n') else {
            return Ok(()); // 還沒寫完一整行
        };
        activity.file_offset += last_newline as u64 + 1;
        for line in utf8(&buffer[..=last_newline]).split('\n') {
            if utf16_len(line) > 1 {
                parse_line(activity, line, is_known);
            }
        }
        Ok(())
    })();
    if let Err(err) = result {
        log::error!("讀取 Claude session {} 失敗: {err}", file.name);
    }
}

fn parse_line(activity: &mut RepoActivity, line: &str, is_known: &(dyn Fn(&str) -> bool + Send + Sync)) {
    // 從檔案中段開始讀時第一行可能不完整
    let Ok(r) = serde_json::from_str::<Value>(line) else { return };
    if !r.is_object() {
        return;
    }
    if let Some(branch) = str_prop(&r, "gitBranch").filter(|b| !b.is_empty()) {
        activity.branch = Some(branch.to_owned());
    }
    if let Some(entrypoint) = str_prop(&r, "entrypoint").filter(|e| !e.is_empty()) {
        activity.entrypoint = Some(entrypoint.to_owned());
    }
    if let Some(prompt) = user_prompt(&r) {
        if let Some(key) = KeyFinder::find_in_prompt(Some(&prompt), is_known) {
            activity.prompt_key = Some(key);
        }
        if activity.first_prompt.is_none() {
            activity.first_prompt = session_digest::summarize(Some(&prompt));
        }
    } else if let Some(edited) = ClaudeMonitor::edit_time(&r, &activity.path)
        && activity.edited_at.is_none_or(|e| edited > e)
    {
        activity.edited_at = Some(edited);
    }
}
