# HayakuBar

嵌在 Windows 工作列裡的 Hayaku 工時小工具：顯示「今天工時 / 應出勤」，點一下往上彈出面板看計時中的任務；
依 Hayaku 班表自動上下班，並偵測 Claude Code 的活動自動為對應任務計時。

## 功能

- **嵌在工作列裡**：顯示 `3.5h / 8h`＋進度條＋計時數＋Claude 標記，背景透明、跟著工作列的深淺色。
  可選「一排」（較寬）或「兩排」（較窄，預設），在右鍵選單「顯示方式」或設定裡切換。
  今天工時是所有計時段的聯集，並行的計時只算一次。
  - 位置可選：最左邊、工作列圖示左側、工作列圖示右側、通知區域左側（右鍵選單或設定），也可以直接左右拖曳微調。
  - 用 UI Automation 讀工作列圖示的範圍，開了很多 app 放不下時會自動移到不會蓋到圖示的位置。
  - explorer 重啟後會自動重新嵌入。
- **展開面板**：計時中的任務（● 手動、✦ Claude 自動）與各自今天累計的時間、今天做過的任務、
  開始任意任務、各 repo 的 Claude 狀態、上班／下班按鈕。
- **依班表自動上下班**（班表、請假、國定假日都以 Hayaku 為準）
  - 上班時間到：恢復上次那組任務
  - 休息開始：全部暫停（Hayaku 的「依工作時間自動暫停」有開也不衝突）
  - 休息結束：**自動恢復**休息前在跑的任務（Hayaku 本身只會發通知問你）
  - 下班時間到：全部停止
  - 手動「上班」隨時可以按；按了「下班」當天就不再自動上班。電腦睡眠錯過的時間點，醒來後只補做最後一個。
- **Claude Code 自動計時**
  - 在設定裡加入 repo 路徑，工具會讀 `~/.claude/projects/<編碼路徑>/*.jsonl`
  - 任務判斷順序：手動指定 → 對話裡提到的任務編號 → git 分支名稱（`feature/F2P-97-xxx`）→ repo 預設任務
  - Claude 有動作就開始計時，跟其他計時並行，之後**持續計時**，不會因為閒置而暫停；休息、下班或手動暫停才會停。
  - 選用：一鍵安裝 Claude Code hooks（寫進 repo 的 `.claude/settings.local.json`），反應更即時。
  - 預設只在上班時間（或你有手動計時）時才自動計時。

## 登入

用 Hayaku 的「已連線的應用程式」（OAuth 2.1 + PKCE）：第一次點「登入 Hayaku」會開瀏覽器到 Hayaku
登入並授權，之後會出現在「我的 › 已連線的應用程式」，可以隨時撤銷。工具不會碰到你的密碼。
token 用 Windows DPAPI 加密存在 `%APPDATA%\HayakuBar\auth.bin`。

所有操作都透過 Hayaku 的 MCP 端點（`/mcp`），跟 Claude 用的是同一組工具。
如果管理員停用了「應用程式透過 MCP 操作」，這個工具就不能用。

## 建置與執行

需要 .NET 10 SDK。

```powershell
dotnet build
dotnet test
dotnet run --project src\HayakuBar
.\publish.ps1                  # 輸出 publish\HayakuBar.exe（需要 .NET 10 Desktop Runtime）
.\publish.ps1 -SelfContained   # 不需要安裝 .NET
```

## 檔案位置

| 檔案 | 用途 |
| --- | --- |
| `%APPDATA%\HayakuBar\config.json` | 設定（repo 清單、自動化開關、工作列位置） |
| `%APPDATA%\HayakuBar\state.json` | 執行狀態（要恢復的任務、今天已處理的班表時間點、Claude 開的計時） |
| `%APPDATA%\HayakuBar\auth.bin` | 登入資料（DPAPI 加密） |
| `%APPDATA%\HayakuBar\log.txt` | 紀錄 |

本機連接埠：`47824` 接收 OAuth 登入回呼（登入時才開）、`47823` 接收 Claude Code hooks。

## 程式結構

```
src/HayakuBar
├─ Auth/      OAuthClient（動態註冊、PKCE、refresh）、TokenStore（DPAPI）
├─ Mcp/       McpClient（JSON-RPC over POST /mcp）、HayakuApi（whoami、get_my_schedule、list_time_entries、
│             get_running_timers、start_timer、pause_timer、my_tasks、list_projects）
├─ Core/      TimerEngine（同步、上下班自動化、Claude 計時）、WorkMath（工時聯集、班表）、KeyFinder
├─ Claude/    ClaudeMonitor（讀 session jsonl、hook 事件）、HookInstaller
├─ UI/        TaskbarWidget（嵌進 Shell_TrayWnd 的 layered 子視窗）、TaskbarIconScanner（UIA 讀圖示範圍）、
│             PanelWindow（彈出面板）、TaskPickerWindow、SettingsWindow、TrayIcon
└─ Infra/     MiniHttpServer、Log、JsonFile、StartupRegistration
```
