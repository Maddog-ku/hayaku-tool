# hb-core 對外 API（給 hayakubar UI 與 hb-slack）

對應 C# `src/HayakuBar/Core/`。`TimerEngine` 只在 UI 執行緒用（`Rc`，不是 `Send`）。

## 建立與啟動（App）

```rust
use std::{cell::RefCell, rc::Rc, sync::Arc};
use hb_core::TimerEngine;

let config = Rc::new(RefCell::new(AppConfig::load()));
let state = Rc::new(RefCell::new(AppState::load()));
let api = Arc::new(HayakuApi::new(Arc::new(mcp_client)));          // McpClient 實作 McpTransport
let engine = TimerEngine::new(config.clone(), state.clone(), auth, api, ClaudeMonitor::new());
engine.set_ui_visible(move || panel_open() || workspace_visible()); // C# UiVisible
engine.events().notify.subscribe(|m| toasts.show(ToastRequest::new("HayakuBar", m.clone(), vec![])));
engine.events().toast.subscribe(|r| toasts.show(r.clone()));
let slack = hb_slack::SlackBridge::new(engine.clone());              // 要在 Slint 後端就緒後建立
engine.set_slack(Some(Rc::new(slack.clone())));                    // hb-slack 實作 SlackLink
slack.restart();                                                   // 一定要呼叫，才會讀 slack.bin 並連線
let e = engine.clone();
slint::spawn_local(async move { e.start().await });                 // C# await engine.StartAsync()
```

- `TimerEngine` 是 `Rc<Inner>` 的包裝，`clone()` 很便宜。
- `TimerEngine::current() -> Option<TimerEngine>`：這個執行緒最後建立、還活著的 engine（只存弱參照）。給不收 engine 參數的共用視窗用
  （hb-ui-common 的選任務視窗；Hayaku 1.5.4 起，C# 版沒有）。
- C# 的 `async Task XxxAsync` → `async fn xxx`，呼叫端用 `slint::spawn_local(async move { e.xxx().await })`。
- C# 內部 fire-and-forget（`_ = …`）走 `engine.spawn(fut)`，預設 `slint::spawn_local`；測試可用 `set_spawner` 換掉。
- 回傳 C# `IReadOnlyList<T>` 的屬性回 `Rc<Vec<T>>`（整份換掉的快照）。

## 事件（C# `event`）

`engine.events()` 回 `&EngineEvents`，每個欄位是 `Event<A>`：`subscribe(|a: &A| …) -> HandlerId`、`unsubscribe(id)`。

| 欄位 | 參數 | C# |
|---|---|---|
| `changed` | `()` | `Changed` |
| `notify` | `String` | `Notify` |
| `toast` | `ToastRequest` | `Toast` |
| `toast_dismiss_requested` | `String`（tag） | `ToastDismissRequested` |
| `time_entries_requested` | `NaiveDate` | `TimeEntriesRequested` |
| `workspace_requested` | `(String, Option<WorkspaceArg>)` | `WorkspaceRequested`（arg：`Text`／`Number`） |
| `create_task_requested`／`new_work_ready`／`pick_task_requested` | `NewWorkSuggestion` | 同名 |
| `reject_requested` | `ChangeRequest` | `RejectRequested` |
| `test_action_requested` | `TestActionRequest` | `TestActionRequested` |
| `config_applied` | `()` | `ConfigApplied` |

`create_task_requested`、`pick_task_requested`、`reject_requested`、`test_action_requested` 跟 C# 一樣延遲 180ms（`slint::Timer::single_shot`）才觸發。

## 提示

- `ToastRequest { title, message, actions, sticky, duration: Option<Duration>, from_bottom, important, tag }`，建構：
  `ToastRequest::new(title, message, actions).sticky(true).duration(d).from_bottom().important().tag("n:1")`。
- `ToastAction { label, run: Rc<dyn Fn() -> Pin<Box<dyn Future<Output=()>>>>, primary }`；按下時 `slint::spawn_local((a.run)())`。
  錯誤都在動作裡處理，不會往外丟。

## 屬性（唯讀）

`user()`、`schedule()`、`entries()`、`my_tasks()`、`logged_in()`、`logging_in()`、`error()`、`initial_syncing()`、`syncing()`、
`last_synced()`、`auto_sync_paused()`、`on_duty()`、`running_keys() -> CiSet`、`is_claude_owned(key)`、`resume_candidates()`、
`meetings()`、`todos()`、`reminders()`、`notifications()`、`unread_count()`、`discussions()`、`projects()`、`pins()`、
`pending_reviews()`、`can_review()`、`is_pinned(key)`、`project_of(name_or_key)`、`try_get_cached_project(p)`、`status_rank(key, status)`、
`task_lists()`、`task_list(tab)`、`team()`、`on_leave_today()`、`overdue_count()`、`due_today_count()`、`status_of(key)`、
`parent_title_of(key)`、`known_parent_title(key) -> (Option<String>, bool)`、`title_of(key)`、`my_fixes()`、`my_tests()`、
`can_retest(key, reporter)`、`cached_sub_statuses(key)`、`fix_done_step`／`pass_step`／`batch_step(key, status)`、
`eip_sync() -> Ref<EipSyncState>`、`eip_syncing()`、`is_eip_synced(no)`、`reviews_fetched_at()`、`cached_project_tree(p)`。

設定與狀態（跟 App 共用）：`config()`／`config_mut()`／`config_rc()`、`state()`／`state_mut()`／`state_rc()`；
改完設定呼叫 `apply_config()`（存檔、套用同步頻率、觸發 `config_applied`）。
Claude 監看：`claude()`／`claude_mut()`（`Ref<ClaudeMonitor>`）。API：`api() -> Arc<HayakuApi>`。讀檔：`files() -> Rc<FileReader>`。
網址：`web_url(path)`、`task_url(key)`、`meeting_url(id)`、`notification_url(&n)`。
常數：`TimerEngine::TASK_TABS`、`TASK_VIEW_TABS`、`ALL_TAB`、`MAX_PINS`、`STATUS_FIXING`、`STATUS_RETEST`、`STATUS_WONT_FIX`。

## 動作（async，錯誤寫進 `error()`，C# 經過 Guarded 的）

`refresh(user_action)`、`refresh_all(user_action)`、`auto_refresh(force)`、`login()`、`logout()`、`start_task(key)`、`pause_task(key)`、
`switch_task(key, keep_others)`、`clock_in(Option<Vec<String>>)`、`clock_out()`、`undo_clock_out(&[String])`、`pin_task`、`unpin_task`、
`move_task(key, parent)`、`join_meeting(&m)`、`leave_meeting(&m)`、`complete_todo(&t)`、`snooze_reminder(&r, minutes)`、
`quick_capture(text, todo)`、`mark_notifications_read(Option<Vec<String>>)`、`archive_notification(&n)`、`snooze_notification(&n, preset)`、
`open_notification(&n)`、`add_time_entry`、`update_time_entry`、`delete_time_entry`、`set_entry_notes(&IndexMap)`、
`complete_task(key, cascade)`、`set_task_status(key, status)`、`refresh_reviews()`、`review(&c, approve, note)`、
`assign_session_task(sid, key, start_now)`、`pick_task_for_repo(path, key)`、`after_external_change()`、`prefetch_new_work(&s)`、
`sync_eip_leaves(manual, morning) -> String`。

會回錯誤給呼叫端的（`Result<_, CoreError>`，`err.message()` 是 C# 的 `ex.Message`）：
`load_day`、`move_time_entry`、`load_discussions`、`load_parent_title`、`load_eip_year`、`get_project`、`creatable_projects`、
`start_fix`、`submit_retest`、`advance_test_results`、`retest`、`wont_fix`、`mark_read_from_slack`、`archive_from_slack`、`review_from_slack`。

同步（非 async）：`show_notice(text)`、`on_hook(&Value)`、`check_alerts()`、`trim_caches()`、`reset_eip()`、`remember_folder`、
`guess_project`、`project_tree(p, fresh) -> TreeLoad`（`Shared` future，可多處 await）、`forget_project_tree`、`dismiss_toast(tag)`、
`open_task_in_workspace(key)`、`request_test_action(action, key, title)`、`meeting_toast(&m, now, demo)`、`review_toast(&c, demo)`、
`load_projects_of(keys)`（async，回 bool）、`build_digest(day, &log, &entries)`（async，回 `ClaudeDigest`）。
靜態：`TimerEngine::notification_target(&n)`、`TimerEngine::split_for_meeting(meeting, &before, &after)`。

## Slack（hb-slack 實作）

```rust
pub trait SlackLink {
    fn locked(&self) -> bool;  fn set_locked(&self, locked: bool);
    fn configured(&self) -> bool;  fn healthy(&self) -> bool;  fn status_text(&self) -> String;
    fn restart(&self);  fn dispose(&self);
    fn subscribe_status_changed(&self, f: Box<dyn Fn()>) -> u64;  fn unsubscribe_status_changed(&self, id: u64);
    fn as_any(&self) -> &dyn std::any::Any;
}
```
`engine.slack() -> Option<Rc<dyn SlackLink>>`、`engine.set_slack(…)`。

## 其他模組

`SyncPolicy`、`OffDayCache`、`ProjectGuess`、`MyTaskTabs`、`MyTaskTree`（`MyTaskNode`／`MyTaskProject`）、`TaskTree`（`TaskTreeNode`）、
`TaskOrder`、`StatusFilter`、`TaskColumns`（C# `Move` → `move_column`）、`TaskOutline`（節點是 `hb_api::OutlineNode`）、
`TaskSchedule`（`Drag`、`Segment<T>`）、`ViewFilter`、`FileReader`、`CiSet`（不分大小寫的字串集合）、`CoreError`。
