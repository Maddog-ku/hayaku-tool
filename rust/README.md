# HayakuBar（Rust 版）

C# WPF 版（`src/HayakuBar`）的 Rust 改寫。功能、使用者看到的文字、設定檔格式都要跟 C# 版一致；
介面改用 Slint，外觀參考使用者提供的微互動元件牆設計稿（不放進專案）。

## 建置

```powershell
cargo build --release -p hayakubar     # 輸出 target\release\HayakuBar.exe
cargo test --workspace
```

需要 Rust（`stable-x86_64-pc-windows-msvc`）與 VS 2022 Build Tools（C++ 工作負載）。
Git Bash 裡先 `export PATH="$HOME/.cargo/bin:$PATH"`。

## Crate 與相依

```
hb-api      HayakuApi（Mcp/HayakuApi*.cs）、資料型別、WorkMath、KeyFinder；定義 McpTransport 介面，不碰網路
hb-backend  Infra（AppPaths、JsonFile、Log、Housekeeping、StartupRegistration、Web、MiniHttpServer、Diagnostics、DPAPI）、
            Config（AppConfig、AppState）、Auth（OAuthClient、TokenStore）、McpClient（實作 McpTransport）、McpStats、tokio 執行環境 rt
hb-win       原生 Win32：TaskbarWidget、TaskbarIconScanner、TrayIcon、GlobalHotkey、系統事件（喚醒、鎖定）、Native、單一執行個體
hb-ui-kit    Slint 主題與基本元件（ui/theme.slint、ui/components/*.slint、ui/fonts/），其他 crate 以 library path `@hb` 匯入
hb-claude    Claude/*（ClaudeMonitor、SessionDigest、ShellEdits、HookInstaller、ClaudeEstimator）
hb-eip       Eip/*（EipClient、EipCredentials、EipSyncState、LeavePlanner）
hb-core      Core/*（TimerEngine 全部 partial、TaskTree、TaskOrder、SyncPolicy…）
hb-slack     Slack/*（SlackClient、SocketModeClient、SlackBridge、SlackMessages、SlackCredentials、SlackState）
hb-ui-common UI 共用部分與小視窗（TaskText、ProjectIcons、ViewModels、DayTimeline、DateRangePicker、TaskMenu、
             QuickCreate、MoveTask、TestAction…），Slint 以 library path `@hbc` 匯入；用法見 API.md
hb-ui-work   工作台（UI/Work/**）：外框 WorkspaceWindow 與 18 個頁面
hayakubar    執行檔：App 接線、面板、HoverCard、提示、設定、CLI；assets/ 放 app.ico、app.manifest
```

相依方向：`hb-api ← hb-backend ← hb-claude / hb-eip ← hb-core ← hb-slack`，
UI 是 `hb-ui-kit ← hb-ui-common ← hb-ui-work ← hayakubar`（hb-ui-common 起依賴 hb-core）。
`hb-win`、`hb-ui-kit` 不依賴其他 hb crate；程式圖示只放一份在 `crates/hayakubar/assets/app.ico`，
hb-win（系統匣）、hb-ui-common、hb-ui-work（視窗圖示）用相對路徑引用它。

## 改寫規則（每個參與的人都要遵守）

1. **照 C# 原始碼逐檔移植**，不是重新設計。每個 Rust 模組開頭註明對應的 C# 檔。邏輯、判斷順序、容錯解析（各種備用欄位名）、
   時間點、節流、重試、退避都要一樣。C# 裡的中文註解一起搬過來（可精簡）。
2. **使用者看得到的文字一字不改**（提示、按鈕、錯誤訊息、紀錄內容）。
3. **檔案格式相容**：`%APPDATA%\HayakuBar\` 底下的 `config.json`、`state.json`、`eip-sync.json`、`slack.json` 用 camelCase、縮排、
   不跳脫中文（C# `JsonFile.Options`），寫入先寫 `.tmp` 再取代；`auth.bin`、`eip.bin`、`slack.bin` 是 DPAPI（CurrentUser）加密的 JSON，
   entropy 跟 C# 一樣（例 `HayakuBar.v1`）。舊版存的檔案要能直接讀，Rust 版存的 C# 版也要能讀。缺欄位用 C# 的預設值。
4. **命名**：型別保留 C# 名稱（`TimerEngine`、`DaySchedule`），方法與欄位改 snake_case（`RefreshAsync` → `refresh`）。
   C# 的 `XxxAsync` 在 Rust 是 `async fn xxx`。一個 C# 檔對應一個 Rust 模組（partial class 可拆成多個 `impl` 檔）。
5. **紀錄**：一律用 `log` crate（`log::info!` / `log::error!`），由 hb-backend 的 `infra::log` 寫成 C# 一樣的檔案（`logs\yyyyMMddHHmmss.log`）。
6. **執行緒模型**：
   - UI 執行緒跑 Slint 事件迴圈；`TimerEngine` 跟 C# 一樣只在 UI 執行緒使用（`Rc` / `RefCell`，不需要 `Send`），
     非同步方法用 `slint::spawn_local` 跑（等同 WPF Dispatcher 上的 async/await）。
   - 網路 IO 在 hb-backend 的 tokio 執行環境（`hb_backend::rt`）。`McpClient`、`EipClient`、`SlackClient` 對外回傳的 future
     必須能在**非 tokio** 的執行器（`slint::spawn_local`）裡 await：內部用 `hb_backend::rt::bg(async move { … })` 丟到 tokio 再等結果。
   - C# 的 `event Action` 在 Rust 用回呼清單（`Vec<Box<dyn Fn(..)>>`）或 `Rc<dyn Fn>`。
7. **錯誤處理**：C# 吃掉例外寫紀錄的地方，Rust 一樣寫紀錄、不 panic。不要用 `unwrap()` 處理外部資料。
8. **測試**：`tests/HayakuBar.Tests` 裡測到你負責程式碼的案例，全部移植成 Rust 測試（`#[cfg(test)]` 或 `crates/<crate>/tests/`），
   斷言內容不可放寬。
9. **範圍**：只改分配給你的 crate；不要改根目錄 `Cargo.toml`、其他 crate、C# 原始碼；不要 commit／push／切分支。
   需要新的第三方套件時加在自己 crate 的 `Cargo.toml`（優先用根目錄 `[workspace.dependencies]` 已列的）。
10. **建置**：多人同時建置時各自用自己的 `CARGO_TARGET_DIR`（分配時會給），用 `cargo check -p <crate>`、`cargo test -p <crate>`，
    交付時不能有編譯錯誤，`cargo clippy -p <crate>` 不能有 error。

## 外觀（hb-ui-kit）

色票、圓角、字型、緩動都在 `hb-ui-kit/ui/theme.slint`；淺色照設計稿，深色由同一套推導。
Windows「顯示動畫」關閉時 `Theme.reduce-motion` 為 true，所有彈簧改成不回彈的平滑緩動。
Slint 使用免版稅授權，設定頁「關於」要放 `AboutSlint`。
