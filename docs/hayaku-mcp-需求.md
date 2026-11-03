# 請 Hayaku 補的 MCP 工具

HayakuBar 只用「已連線的應用程式」（OAuth → `/mcp`）操作 Hayaku，目標是平常不用開網頁。
2026-11-03 比對 Hayaku 網頁（`/api/v1` 的功能）與 MCP 的 100 支工具，下面這些**網頁做得到、MCP 做不到**，
HayakuBar 目前只能放「在網頁開啟」。依使用頻率排序，參數是建議的草稿。

## 一、個人設定（設定頁目前全部開網頁）

| 工具 | 用途 | 建議參數 |
| --- | --- | --- |
| `get_my_settings` | 讀個人設定 | 無；回 `displayName`、`title`、`timeUnit`（minutes／hours）、`singleRunningTimer`、`theme`、`autoPauseByWorkTime` |
| `update_my_settings` | 改個人設定 | 上面任一欄位（只帶要改的） |
| `get_notification_preferences` | 讀通知偏好 | 無；回通知量（all／suggested／mentions）、各類通知（任務、提醒與到期、會議、審核、頻道與論壇、團隊空間、系統）的層級、提醒的預設方式 |
| `update_notification_preferences` | 改通知偏好 | `volume`、`categories: { 類別: none／inApp／push }`、`reminderDefaults`；可帶 `project` 只改某專案 |

## 二、任務

| 工具 | 用途 | 建議參數 |
| --- | --- | --- |
| `watch_task` ／ `unwatch_task` | 關注／取消關注任務（網頁「我的任務 › 關注中」） | `task` |
| `list_watching` | 我關注的任務 | `limit`、`cursor` |
| `restore_task` | 從垃圾桶還原刪掉的任務 | `task` 或 `id` |
| `list_trash` | 專案垃圾桶 | `project` |
| `move_task_to_project` | 跨專案移動（網頁有預覽） | `task`、`project`、`parent`、`preview: true` 先看影響 |
| `set_task_series` ／ `stop_task_series` | 重複任務 | `task`、重複規則（同 `create_reminder` 的 `repeat`） |

## 三、投票、聊天

| 工具 | 用途 | 建議參數 |
| --- | --- | --- |
| `vote_poll` | 投票（`get_poll` 目前唯讀） | `poll`、`options: [選項 label 或 id]` |
| `create_poll` | 在聊天、論壇、文件發起投票 | `task`／`project`+`channel`、`question`、`options`、`multiple`、`anonymous`、`closesAt` |
| `mark_chat_read` ／ `mute_chat` | 聊天已讀、靜音 | `task` 或 `project`+`channel` |

## 四、專案視圖、統計

| 工具 | 用途 | 建議參數 |
| --- | --- | --- |
| `create_view` ／ `update_view` ／ `delete_view` | 存自己的篩選視圖（`list_views` 目前只能讀） | `project`、`name`、`filters`（同 `list_tasks` 的篩選）、`private` |
| `list_stats_views` ／ `save_stats_view` | 統計的自訂視圖與工時預算 | `name`、`scope`、`period`、`groupBy` |

## 五、筆記本

| 工具 | 用途 | 建議參數 |
| --- | --- | --- |
| `update_my_note` ／ `delete_my_note` | 改、刪筆記頁（目前只能新增、附加） | `pageId`、`title`、`markdown` |
| `move_my_note` | 筆記頁換資料夾 | `pageId`、`folderId` |
| `list_inbox` ／ `update_inbox_item` | 隨手記收件匣（轉成任務、刪除） | `id`、`action` |

## 不需要 MCP（留在網頁）

- 安全：兩步驟驗證、已連線的應用程式、存取權杖 —— 涉及登入安全，留在網頁比較好。
- 專案設定（成員、狀態、自訂欄位、標籤、自動化、範本）、系統管理 —— 低頻、要管理員權限。
- Excel 匯入、檔案上傳 —— MCP 無狀態 POST 不適合傳大檔。
