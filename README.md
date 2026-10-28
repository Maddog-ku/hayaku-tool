# HayakuBar

嵌在 Windows 工作列裡的 Hayaku 工時小工具：顯示「今天工時 / 應出勤」，點一下往上彈出面板看計時中的任務；
依 Hayaku 班表自動上下班，並偵測 Claude Code 的活動自動為對應任務計時。

## 下載

到 [Releases](https://github.com/Maddog-ku/hayaku-tool/releases/latest) 下載 **HayakuBar.exe**
（單一檔案，不用另外安裝 .NET），放到固定的資料夾後雙擊執行，第一次按「登入」在瀏覽器授權 Hayaku 即可。
Windows 若跳出「Windows 已保護您的電腦」，按「其他資訊」→「仍要執行」。
更新時先在工作列右鍵結束 HayakuBar，再用新的 exe 取代舊的（設定存在 `%APPDATA%\HayakuBar`，不會不見）。

## 功能

- **嵌在工作列裡**：顯示 `3.5h / 8h`＋進度條＋計時數＋Claude 標記，背景透明、跟著工作列的深淺色。
  可選「一排」（較寬）或「兩排」（較窄，預設），在右鍵選單「顯示方式」或設定裡切換。
  今天工時是所有計時段的聯集，並行的計時只算一次。
  - 位置可選：最左邊、工作列圖示左側、工作列圖示右側、通知區域左側（右鍵選單或設定），也可以直接左右拖曳微調。
  - 用 UI Automation 讀工作列圖示的範圍，開了很多 app 放不下時會自動移到不會蓋到圖示的位置。
  - explorer 重啟後會自動重新嵌入。
  - 有未讀通知時顯示紅色數字；滑鼠停在上面會跳出今天各任務的明細與時間軸。
- **彈出面板**：點工作列上的工時（或按快捷鍵，預設 Win+Shift+H），面板從工作列往上滑出。
  上方是今天的時間軸（班表時段、有計時的時段、班表內沒計到的空檔標紅，滑鼠移上去看那時在跑哪些任務）。分四頁：
  - **計時**：計時中的任務（● 手動、✦ Claude 自動）與今天累計時間、今天做過的任務；點任務在 Hayaku 開啟；
    「開始任務」打字即篩選（↑↓ 選、Enter 開始）；各專案的 Claude 狀態
  - **任務**：我的任務（進行中、逾期…），右鍵可停止並完成、改狀態、提醒、聊天
  - **今天**：今天的會議（快開始時可「加入並計時」）、待辦（直接勾掉）、即將到來的提醒、隨手記（可存成待辦）
  - **通知**：最新通知，點一下在 Hayaku 開啟並標為已讀；全部標為已讀
  - 底部：上班／下班（下班後可「復原」）、工時紀錄、同步狀態
- **工作台**：我的任務、我的工時（月曆）、工時紀錄、工作日誌、提醒、會議、聊天、團隊狀況、請假、統計（Ctrl+1～9、0 切換）。
  - **工時紀錄**：任一天的計時段與時間軸，直接改起訖時間、備註、刪除；列出班表裡沒有紀錄的時段，點一下帶入補登。
  - **工作日誌 › ✦ Claude 整理**：把那天在「專案」資料夾裡的 Claude Code 指令與自己的 git commit 依任務整理成草稿，
    可以先改再複製或存到隨手記；也能把同一段時間的指令與 commit 寫進還沒有備註的計時段（不會動已有的備註）。
- **提示**：從工作列的工時上方滑出。會議快開始（加入並計時）、提醒到點（10 分鐘後再提醒）、新通知、
  下班（復原、工時沒滿時補登工時、整理日誌）。全螢幕、簡報、Windows 勿擾時段或鎖定時先存起來，結束後再跳出（會議提示照跳，可在設定關掉）。
- **深色模式**：跟隨 Windows，或在設定指定淺色／深色。
- **依班表自動上下班**（班表、請假、國定假日都以 Hayaku 為準）
  - 上班時間到：恢復上次那組任務
  - 休息開始：全部暫停（Hayaku 的「依工作時間自動暫停」有開也不衝突）
  - 休息結束：**自動恢復**休息前在跑的任務（Hayaku 本身只會發通知問你）
  - 下班時間到：全部停止
  - 手動「上班」隨時可以按；按了「下班」當天就不再自動上班。電腦睡眠錯過的時間點，醒來後只補做最後一個。
- **Claude Code 自動計時**
  - 只要加入放專案的**主資料夾**（例 `D:\repo`），裡面的專案會自動偵測（git repo，或 Claude 開過 session 的子資料夾），
    每個專案各自計時，可個別停用；主資料夾本身是 git repo 時，一般子資料夾不算專案
  - CLI 和桌面版的 session 都在 `%USERPROFILE%\.claude\projects\<編碼路徑>\*.jsonl`，用檔案裡的 `cwd` 決定歸屬
    （避免中文路徑撞名、名稱前綴相同的 repo 誤判），Git Bash 格式的路徑（`/d/repo/...`）也認得
  - 設定頁可「從最近的 session 加入」，直接挑 Claude 用過的資料夾
  - 任務判斷順序：手動指定（面板 Claude 區的「選擇」，只對那個 session 有效）→ 對話裡提到的任務編號 → git 分支名稱（`feature/F2P-97-xxx`）。
    **沒有預設任務**：都偵測不到就不計時
  - Claude 有動作就開始計時，跟其他計時並行，之後**持續計時**，不會因為閒置而暫停；休息、下班或手動暫停才會停。
  - 選用：一鍵安裝 Claude Code hooks（寫進 repo 的 `.claude/settings.local.json`），反應更即時。
  - 預設只在上班時間（或你有手動計時）時才自動計時。
  - **自動建立任務**（「專案」頁逐一勾選，預設關）：Claude 開始改程式（Edit、Write…）、卻對應不到任務時跳提示；
    只是問問題、規劃不會跳，同一個 session 只問一次。按「建立任務」開小視窗，專案自動選好（記住資料夾對應的專案與上層），
    選專案樹位置、狀態、預計起訖，建好後就為這個 session 計時；也可以「選既有任務」。

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

### 發佈新版本

推一個版本 tag，GitHub Actions（`.github/workflows/release.yml`）會跑測試、打包成單一 exe（約 72MB，不用裝 .NET），
並建立 Release 附上 `HayakuBar.exe` 給使用者下載：

```powershell
git tag v1.0.1
git push origin v1.0.1
```

也可以在 GitHub 的 Actions › Release › Run workflow 輸入版本號手動發佈。

### 啟動參數

| 參數 | 用途 |
| --- | --- |
| `--settings[:分頁]` | 開設定頁（account、taskbar、schedule、repos、claude、general） |
| `--workspace[:頁面]` | 開工作台（tasks、hours、entries、worklog、reminders、meetings、chat、team、leaves、stats） |
| `--panel[:分頁]` | 打開面板且不會自動收起（timer、tasks、today、notify），檢查畫面用 |
| `--entries` | 開工作台的工時紀錄（今天） |
| `--shot:頁面,…` | 把各頁畫面存成 PNG 到 `%APPDATA%\HayakuBar\shots`（例 `panel-today,tasks,settings-general,toast`） |
| `--toast-test` | 顯示一則範例提示 |
| `--meeting-test` | 模擬兩場快開始的會議提示（參加者／不是參加者），按鈕不會真的加入 |
| `--mcp-call <工具> <參數JSON> <輸出檔>` | 診斷：用目前登入呼叫一支 MCP 工具（先關掉執行中的 HayakuBar） |

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
├─ Core/      TimerEngine（同步、上下班自動化、Claude 計時、提示）、WorkMath（工時聯集、班表空檔）、KeyFinder
├─ Claude/    ClaudeMonitor（讀 session jsonl、hook 事件）、SessionDigest（對話與 commit 整理成日誌）、HookInstaller
├─ UI/        TaskbarWidget（嵌進 Shell_TrayWnd 的 layered 子視窗）、TaskbarIconScanner（UIA 讀圖示範圍）、
│             PanelWindow（彈出面板）、DayTimeline（時間軸）、HoverCard、ToastWindow、TaskPickerWindow、SettingsWindow、TrayIcon
│  └─ Work/   WorkspaceWindow（工作台）與各頁
└─ Infra/     MiniHttpServer、Log、JsonFile、StartupRegistration
```
