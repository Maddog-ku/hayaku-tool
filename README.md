# HayakuBar

嵌在 Windows 工作列裡的 [Hayaku](https://hayaku.revivalsi.com) 工時小工具。

- 工作列上隨時看到「今天工時 / 應出勤」，點一下往上彈出面板，管理計時中的任務、今天的會議與待辦、通知。
- 依 Hayaku 班表**自動上下班**：上班恢復計時、休息暫停、休息結束自動恢復、下班全部停止。
- 偵測 **Claude Code** 的活動，自動為對應的任務計時；對應不到任務時還能當場建立。
- 工作台集中處理任務、工時紀錄、工作日誌、提醒、會議、聊天、請假與統計，不用一直開瀏覽器。

所有資料都以 Hayaku 為準，工具本身不另外存工時；工時的算法跟 Hayaku 網頁一致（見[工時怎麼算](#工時怎麼算)）。

## 目錄

- [系統需求](#系統需求)
- [下載與安裝](#下載與安裝)
- [快速開始](#快速開始)
- [功能](#功能)
- [工時怎麼算](#工時怎麼算)
- [設定](#設定)
- [登入與安全](#登入與安全)
- [常見問題](#常見問題)
- [開發](#開發)
- [檔案位置](#檔案位置)
- [程式結構](#程式結構)

## 系統需求

- Windows 10 / 11（x64）
- Hayaku 帳號，且管理員沒有停用「應用程式透過 MCP 操作」
- Claude Code 自動計時（選用）：本機有用 Claude Code（CLI 或桌面版）

## 下載與安裝

1. 到 [Releases](https://github.com/Maddog-ku/hayaku-tool/releases/latest) 下載 **HayakuBar.exe**（單一檔案，不用另外安裝 .NET）。
2. 放到固定的資料夾（例 `D:\Tools\HayakuBar\`），雙擊執行。
3. Windows 若跳出「Windows 已保護您的電腦」，按「其他資訊」→「仍要執行」。

**更新**：在工作列的工時上按右鍵結束 HayakuBar，再用新的 exe 取代舊的。設定存在 `%APPDATA%\HayakuBar`，不會不見。

**移除**：結束 HayakuBar、在設定關掉「開機時自動啟動」，刪掉 exe 與 `%APPDATA%\HayakuBar` 資料夾；
有裝 Claude Code hooks 的話先在設定的「專案」頁移除。最後到 Hayaku「我的 › 已連線的應用程式」撤銷授權。

## 快速開始

1. **登入**：第一次開啟會出現設定頁，按「登入 Hayaku」，在瀏覽器授權後回到工具。
2. **看工作列**：工時會出現在通知區域左側（預設）。在上面按右鍵可以換位置、換「一排／兩排」，也可以直接左右拖曳微調。
3. **開始計時**：點工作列上的工時（或按 `Win+Shift+H`）打開面板，在「開始任務」輸入任務編號或關鍵字，Enter 開始。
4. **（選用）Claude Code 自動計時**：設定 › 專案 › 「選擇資料夾」，加入放專案的主資料夾（例 `D:\repo`）。
   之後 Claude 在裡面工作時，會依對話裡的任務編號或 git 分支名稱自動計時。
5. **（選用）開機自動啟動**：設定 › 一般 › 「開機時自動啟動」。

## 功能

### 工作列

- 顯示 `3.5h / 8h`＋進度條＋計時中的任務數＋Claude 標記，背景透明、跟著工作列的深淺色。
- 可選「一排」（較寬）或「兩排」（較窄，預設），在右鍵選單「顯示方式」或設定裡切換。
- 位置可選：最左邊、工作列圖示左側、工作列圖示右側、通知區域左側（右鍵選單或設定），也可以直接左右拖曳微調。
- 用 UI Automation 讀工作列圖示的範圍，開了很多 app 放不下時會自動移到不會蓋到圖示的位置。
- explorer 重啟後會自動重新嵌入。
- 有未讀通知時顯示紅色數字；滑鼠停在上面會跳出今天各任務的明細與時間軸。

### 彈出面板

點工作列上的工時（或按快捷鍵，預設 `Win+Shift+H`），面板從工作列往上滑出。
上方是今天的時間軸：班表時段、有計時的時段、班表內沒計到的空檔標紅，滑鼠移上去看那時在跑哪些任務。分四頁：

| 分頁 | 內容 |
| --- | --- |
| **計時** | 計時中的任務（● 手動、✦ Claude 自動）與今天累計時間、今天做過的任務；點任務在 Hayaku 開啟；「開始任務」打字即篩選（↑↓ 選、Enter 開始）；各專案的 Claude 狀態，可「選擇」指定 session 的任務 |
| **任務** | 我的任務（進行中、逾期…），右鍵可停止並完成、改狀態、提醒、聊天 |
| **今天** | 今天的會議（快開始時可「加入並計時」）、待辦（直接勾掉）、即將到來的提醒、隨手記（可存成待辦） |
| **通知** | 最新通知，點一下在 Hayaku 開啟並標為已讀；全部標為已讀 |

底部：上班／下班（下班後可「復原」）、工時紀錄、工作台、同步狀態。

### 工作台

從面板底部或右鍵選單開啟，`Ctrl+1`～`9`、`0` 切換頁面：

| 頁面 | 內容 |
| --- | --- |
| **我的任務** | 清單、樹狀與看板檢視，依專案篩選、排序；新增任務可從專案樹選上層，日期用日曆選擇 |
| **我的工時** | 月曆檢視每天的工時與應出勤，標出沒填滿的日子 |
| **工時紀錄** | 任一天的計時段與時間軸，直接改起訖時間、備註、刪除；列出班表裡沒有紀錄的時段，點一下帶入補登 |
| **工作日誌** | 每天各任務的工時；**✦ Claude 整理**把那天的 Claude Code 指令與自己的 git commit 依任務整理成草稿，可以先改再複製或存到隨手記，也能寫進還沒有備註的計時段（不會動已有的備註） |
| **提醒** | 新增、修改、刪除提醒，可綁任務、設重複規則 |
| **會議** | 期間內的會議，建立、加入、離開、取消 |
| **聊天** | 任務或專案頻道的聊天 |
| **團隊狀況** | 團隊成員今天的班表與狀態 |
| **請假** | 請假紀錄，新增與取消 |
| **統計** | 期間內的實際／應出勤工時、出勤天數、任務完成與逾期 |

### 提示

從工作列的工時上方滑出：

- 會議快開始（可「加入並計時」）、提醒到點（可「10 分鐘後再提醒」）、新通知。
- 下班時：可復原；工時沒滿時可補登工時、整理日誌。
- 全螢幕、簡報、Windows 勿擾時段或鎖定螢幕時先存起來，結束後再一起跳出（會議提示照跳，可在設定關掉）。

### 依班表自動上下班

班表、請假、國定假日都以 Hayaku 為準。

| 時間點 | 自動做的事 |
| --- | --- |
| 上班時間到 | 恢復上次那組任務開始計時 |
| 休息開始 | 全部暫停（Hayaku 的「依工作時間自動暫停」有開也不衝突） |
| 休息結束 | **自動恢復**休息前在跑的任務（Hayaku 本身只會發通知問你） |
| 下班時間到 | 全部停止 |

手動「上班」隨時可以按；按了「下班」當天就不再自動上班。電腦睡眠錯過的時間點，醒來後只補做最後一個。

### Claude Code 自動計時

- 只要加入放專案的**主資料夾**（例 `D:\repo`），裡面的專案會自動偵測（git repo，或 Claude 開過 session 的子資料夾），
  每個專案各自計時，可個別停用；主資料夾本身是 git repo 時，一般子資料夾不算專案。
- CLI 和桌面版的 session 都在 `%USERPROFILE%\.claude\projects\<編碼路徑>\*.jsonl`，用檔案裡的 `cwd` 決定歸屬
  （避免中文路徑撞名、名稱前綴相同的 repo 誤判），Git Bash 格式的路徑（`/d/repo/...`）也認得。
- 設定頁可「從最近的 session 加入」，直接挑 Claude 用過的資料夾。
- **任務判斷順序**：手動指定（面板 Claude 區的「選擇」，只對那個 session 有效）→ 對話裡提到的任務編號 → git 分支名稱（`feature/F2P-97-xxx`）。
  **沒有預設任務**：都偵測不到就不計時。
- Claude 有動作就開始計時，跟其他計時並行，之後**持續計時**，不會因為閒置而暫停；休息、下班或手動暫停才會停。
- 預設只在上班時間（或你有手動計時）時才自動計時。
- 選用：一鍵安裝 Claude Code hooks（寫進 repo 的 `.claude/settings.local.json`），送出指令就馬上開始計時。
- **自動建立任務**（「專案」頁逐一勾選，預設關）：Claude 開始改程式（Edit、Write…）、卻對應不到任務時跳提示；
  只是問問題、規劃不會跳，同一個 session 只問一次。按「建立任務」開小視窗，專案自動選好（記住資料夾對應的專案與上層），
  選專案樹位置、狀態、預計起訖，建好後就指定給這個 session 並馬上計時；也可以「選既有任務」。

### 外觀

深色模式跟隨 Windows，或在設定指定淺色／深色；面板高度可固定或跟著內容伸縮。

## 工時怎麼算

- **今天工時是所有計時段的聯集**：同時有好幾個任務在計時（例如手動一個、Claude 一個），重疊的時間只算一次。
  各任務自己的時間則分開算，所以「各任務加起來」可能比今天工時多。
- **顯示成小時時，先捨去不滿一分鐘的秒數**，再換算到小數點後兩位，跟 Hayaku 網頁一致：
  41 分 36 秒 → 41 分 → `0.68h`；剛好整數就不顯示小數（`8h`）。
- 計時中的任務在面板上以碼表（`0:41:42`）顯示，含秒數。
- 進行中的計時算到現在；跨日的計時只算當天的部分。

## 設定

在工作列的工時上按右鍵 ›「設定」開啟：

| 分頁 | 可以設定的項目 |
| --- | --- |
| **帳號** | 登入／登出 Hayaku |
| **工作列** | 顯示位置、一排／兩排、位置微調 |
| **上下班** | 依班表自動上下班開關、今天的班表 |
| **Claude Code** | 自動計時開關、只在上班時間自動計時 |
| **專案** | 主資料夾、各專案停用、自動建立任務、安裝／移除 hooks |
| **一般** | 外觀、面板高度、快捷鍵、會議／提醒／通知提示、全螢幕時暫緩提示、下班補登提示、開機自動啟動、只在上班時間同步、同步頻率（預設 30 秒） |

## 登入與安全

- 用 Hayaku 的「已連線的應用程式」（OAuth 2.1 + PKCE）：第一次點「登入 Hayaku」會開瀏覽器到 Hayaku 登入並授權，
  之後會出現在「我的 › 已連線的應用程式」，可以隨時撤銷。**工具不會碰到你的密碼。**
- token 用 Windows DPAPI 加密存在 `%APPDATA%\HayakuBar\auth.bin`，只有同一個 Windows 帳號解得開。
- 所有操作都透過 Hayaku 的 MCP 端點（`/mcp`），跟 Claude 用的是同一組工具、同一套權限。
  如果管理員停用了「應用程式透過 MCP 操作」，這個工具就不能用。
- 本機只開兩個連接埠，都只聽本機 `127.0.0.1`：`47824` 接收 OAuth 登入回呼（登入時才開）、`47823` 接收 Claude Code hooks。

## 常見問題

**工作列上看不到工時？**
放不下時會自動移位；在系統匣找 HayakuBar 圖示按右鍵換位置。explorer 重啟後會自動重新嵌入，等幾秒即可。

**跟 Hayaku 網頁的工時不一樣？**
兩邊都是「聯集、捨去秒數、到小數點後兩位」。網頁不會自動更新，重新整理後再比；工具預設每 30 秒同步一次，按面板底部的 ⟳ 可馬上同步。

**Claude 在工作卻沒有計時？**
確認：資料夾有加進「專案」頁、該專案沒被停用；對話或分支名稱裡有任務編號（或在面板 Claude 區按「選擇」指定）；
現在是上班時間（或關掉「只在上班時間自動計時」）。

**下班後還是自動開始計時？**
按過「下班」當天就不再自動上班；Claude 自動計時預設也只在上班時間。如果有手動計時在跑，會視為加班照常計時。

**出問題要看哪裡？**
`%APPDATA%\HayakuBar\log.txt`，設定 › 一般 ›「紀錄與設定檔」按「開啟資料夾」。

## 開發

需要 .NET 10 SDK。

```powershell
dotnet build
dotnet test
dotnet run --project src\HayakuBar
.\publish.ps1                  # 輸出 publish\HayakuBar.exe（需要 .NET 10 Desktop Runtime）
.\publish.ps1 -SelfContained   # 不需要安裝 .NET（約 70MB）
```

HayakuBar 正在執行時會鎖住 `bin\Debug` 裡的 exe，建置前先結束它。

### 發佈新版本

1. 改 `src/HayakuBar/HayakuBar.csproj` 的 `<Version>`，commit 並 push。
2. 推一個同版本的 tag，GitHub Actions（`.github/workflows/release.yml`）會跑測試、打包成單一 exe（不用裝 .NET），
   並建立 Release 附上 `HayakuBar.exe` 給使用者下載：

```powershell
git tag v1.0.4
git push origin v1.0.4
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
| `%APPDATA%\HayakuBar\shots\` | `--shot` 存的截圖 |

**清暫存**：啟動 3 分鐘後、之後每 30 分鐘、以及鎖定電腦時，自動刪除 3 天前的截圖（`shots\`）與 14 天前的舊紀錄（`log.txt.1`），
清掉 Claude session、專案狀態、搜尋的快取，並把用不到的記憶體還給 Windows（面板、工作台、設定開著時不整理記憶體）；
結果寫在 `log.txt`（「清暫存」）。

## 程式結構

```
src/HayakuBar
├─ Auth/      OAuthClient（動態註冊、PKCE、refresh）、TokenStore（DPAPI）
├─ Config/    AppConfig（config.json）、AppState（state.json）
├─ Mcp/       McpClient（JSON-RPC over POST /mcp）、HayakuApi（計時、班表、任務、會議、提醒、待辦、通知、
│             聊天、請假、團隊、工作日誌、統計等 Hayaku MCP 工具的強型別包裝）
├─ Core/      TimerEngine（同步、上下班自動化、Claude 計時、提示）、WorkMath（工時聯集、顯示格式、班表空檔）、
│             KeyFinder（找任務編號）、TaskTree／MyTaskTree／TaskOrder（任務樹與排序）、ProjectGuess（資料夾對應專案）
├─ Claude/    ClaudeMonitor（讀 session jsonl、hook 事件）、SessionDigest（對話與 commit 整理成日誌）、HookInstaller
├─ UI/        TaskbarWidget（嵌進 Shell_TrayWnd 的 layered 子視窗）、TaskbarIconScanner（UIA 讀圖示範圍）、
│             PanelWindow（彈出面板）、DayTimeline（時間軸）、HoverCard、ToastWindow、TaskPickerWindow、
│             QuickCreateWindow（自動建立任務）、DateRangePicker、SettingsWindow、TrayIcon、ThemeManager
│  └─ Work/   WorkspaceWindow（工作台）與各頁
├─ Themes/    Light／Dark 色彩
└─ Infra/     MiniHttpServer、GlobalHotkey、Housekeeping（清暫存）、Log、JsonFile、StartupRegistration
tests/HayakuBar.Tests   xUnit 單元測試
```
