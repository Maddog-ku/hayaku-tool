# hb-ui-common 對外 API（給面板、工作台、程式主體）

對應 C# `src/HayakuBar/UI/` 的共用元件與小視窗。**所有函式只能在 UI 執行緒（Slint 事件迴圈）呼叫**；
`TimerEngine` 用 `&TimerEngine` 傳入（內部會 clone）。開窗函式都不會 panic，建立視窗失敗只寫紀錄。

## 1. 引用方式

`Cargo.toml`：`hb-ui-common.workspace = true`。

`build.rs`（同時設 `hb` 與 `hbc`）：

```rust
fn main() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let library_paths = std::collections::HashMap::from([
        ("hb".to_string(), dir.join("../hb-ui-kit/ui")),
        ("hbc".to_string(), dir.join("../hb-ui-common/ui")),
    ]);
    let config = slint_build::CompilerConfiguration::new().with_library_paths(library_paths);
    slint_build::compile_with_config("ui/xxx.slint", config).unwrap();
}
```

`.slint`：`import { DayTimeline, DayTimelineModel, DateRangePicker, TaskMenuArea, TaskMenuState } from "@hbc/index.slint";`

> **同一個 .slint struct／global 被不同 crate 編譯會產生不同的 Rust 型別。** 這裡的共用元件只用基本型別屬性；
> 需要 struct（`DayTimelineModel`）或 global（`TaskMenuState`）的，用本 crate 的巨集轉成**呼叫端自己的型別**（欄位名稱一致）。
> 呼叫端的根 .slint 要 `export { DayTimelineModel, TaskMenuState }`（或把它們用在 export 的元件屬性上），Rust 才拿得到型別。

## 2. 深淺色

```rust
hb_ui_common::set_theme(dark, reduce_motion);   // 啟動時與系統切換深淺色／顯示動畫時呼叫
```
本 crate 開的小視窗會跟著換（Slint 的 global 是每個視窗各一份，這裡會一個一個設，標題列也跟著換）。
呼叫端自己的視窗照常設自己的 `Theme`。字型請先呼叫 `hb_ui_kit::init_fonts()`。

## 3. 共用 Slint 元件（`@hbc/index.slint`）

| 元件 | 對應 C# | 屬性／回呼 |
|---|---|---|
| `DayTimeline` | `UI/DayTimeline.cs` | `in model: DayTimelineModel`、`in bar-height`（預設 8px）；高度固定 23px，寬度跟著容器 |
| `DateRangePicker` | `UI/DateRangePicker.xaml` | `in-out start`、`in-out end`（yyyymmdd，0＝未指定）、`in single`、`in today`、`in enabled`；`callback applied()`；`function open-calendar()`、`close-calendar()` |
| `TaskMenuArea` ＋ `TaskMenuState` | `UI/TaskMenu.cs` | `in key`、`in enabled`；`function open-at(abs: Point)`（視窗座標） |
| `CopyKey` | `KeyCopy.Run` | `in key`、`font-size`、`font-weight`、`color`；`callback copy(key)` → `KeyCopy::copy(&engine, &key)` |
| `ProjectIcon`／`ProjectBadge` | `ProjectIcons.Icon`／`Badge` | `glyph`、`tint: color`、`size`（＋`name`、`text-color`） |
| `ColorKeys.color(code)` | `Brushes2.Get` | `ColorKey::code()` → Theme 色 |
| `StatusLine` | `StatusText`／`Inputs.Show` | `text`、`tone`（0 錯誤、1 成功、2 警告、3 一般）；空字串收起來 |
| `TextArea` | `SmallDialog.MultiLine` | `in-out text`、`placeholder`、`enabled`、`read-only`、`font-family`、`accepts-tab`；`edited`、`tab-pressed`；`out cursor-offset`、`anchor-offset`；`function set-selection(a, b)`、`select-all()` |
| `PlainText` | Shared `PlainText` | 繼承 TextInput（唯讀、可選取、換行） |
| `CheckRow` | CheckBox | `in-out checked`、`text`、`enabled`、`tip`；`toggled(bool)` |
| `RadioDot` | RadioButton | `on`、`text`；`clicked` |
| `LinkButton` | Shared `LinkButton` | `text`、`enabled`、`font-size`；`clicked` |
| `ComboField` | 可輸入的 ComboBox | `in-out text`、`items: [string]`、`placeholder`、`enabled`；`edited` |
| `SelectField` | ComboBox（可捲動） | `items`、`in-out current`、`placeholder`、`enabled`、`max-rows`；`selected(i)` |
| `NavItem` | Shared `NavItem` | `text`、`selected`、`badge`；`clicked` |
| `InlineEdit` | Shared `InlineEdit` | `in-out text`、`single-line`、`font-size`、`font-weight`；`accepted`、`edited`、`focus-lost` |
| `FieldLabel`／`MutedText`／`GroupLabel`／`PageTitle` | Shared 同名樣式 | 繼承 Text |
| `InfoCard` | Shared `InfoCard` | 淡底卡片，`content-padding` |

其他 Shared.xaml 樣式直接用 hb-ui-kit：ToggleSwitch → `Toggle`、Segment → `Segmented`、Pill → `Pill`、
Card → `Card`、HintBox → `TextField { placeholder }`。

`SelectField` 選取的文字、清單裡的選項、`NavItem` 的名稱被截掉（elide）時，滑過會用 Tip 顯示完整文字。
`DateRangePicker` 按「套用日期」「清除」「取消」或 single 點日期時，日曆先往上收起（約 160ms）再關；`applied()` 照樣在按下時就呼叫。
`close-calendar()` 是馬上關（不播動畫）。

### 3.0 動畫（軟體繪圖）

程式固定用 Slint 軟體繪圖：**`transform-scale`、`transform-rotation`、`drop-shadow-*` 都畫不出來**，不要用。
動畫只用 x/y 位移、width/height、opacity、clip、顏色、border、Path；全部遵守 `Theme.reduce-motion`（不回彈、不位移、不錯開，只淡入）。
閒置時不能有持續重繪；無限循環只有 `Spinner`（只在畫出來時才會一直重畫）。

hb-ui-kit（`@hb/index.slint`）新增、給其他 crate 用的：

| 名稱 | 說明 |
|---|---|
| `Motion.fast`／`enter-dur`／`stagger`／`exit-dur`／`rise` | 一般過渡 160ms；出現 340ms（reduce-motion 120ms）；依序出現間隔 50ms（reduce-motion 0）；收起／關閉前淡出 120ms（reduce-motion 0）；出現時上浮距離 10px（reduce-motion 0） |
| `Reveal` | 包住內容（VerticalLayout 排，`spacing` 可設），`shown` 為 true 時（建立時就是 true 也算）往上浮出＋淡入，`index` × `Motion.stagger` 延遲；`out progress` |
| `Stagger.at(t, i)` | 清單依序浮現：整份清單共用進度 t（0→1），第 i 列的出現程度 0–1 |
| `SoftShadow` | 假陰影（疊三層半透明圓角矩形）：放在元素前面同位置，`radius`、`strength`（0–1，可動畫）、`offset-y`、`spread` |
| `Chevron` | 會翻轉的下拉箭頭：`flip` 0 朝下、1 朝上（`color`、`weight`） |
| `Card.hoverable` | 預設 false；true 時滑過出現外框、深色時底色略亮 |
| `ProgressBar.grow-in` | 預設 false；true 時建立時從 0 長到 value |

小視窗（本 crate 的 `ui/windows/`）：`window_util::present` 顯示後設 `DialogMotion.shown`，各區塊（`Section`，繼承 `Reveal`）依序往上浮出；
清單（選擇任務、移動任務、建立任務的專案樹）在打開或篩選結果改變時依序浮現（`ListIntro`＋`Stagger`，只是點選不會重播）；
`window_util::close`（程式關或按 ×）先淡出約 120ms、擋掉滑鼠鍵盤，再藏起視窗（reduce-motion 時直接關）。
`QuickCreate::close` 一樣先淡出；`QuickCreate::find` 不會再找到已經關掉（還在淡出）的視窗。

### 3.1 DayTimeline：資料轉換

```rust
use hb_ui_common::{DayTimelineData, day_timeline_model};
let data = DayTimelineData::new(day, schedule_segments /* Option<&[WorkSegment]> */, &entries, now, /* show_gaps */ true);
ui.set_timeline(day_timeline_model!(DayTimelineModel, data));   // DayTimelineModel 是呼叫端 crate 編譯出的型別
```
`data.describe(t)` 是懸停說明（「HH:mm  計時中：A、B」）；元件內已經用 `hover-texts`／`hover-index` 顯示，不用另外接。

### 3.2 DateRangePicker：日期轉換

```rust
use hb_ui_common::date_range::{to_int, from_int, today_int};
ui.set_plan_start(to_int(task.planned_start));   // Option<NaiveDate> → i32
ui.set_today(today_int());
let start: Option<NaiveDate> = from_int(ui.get_plan_start());
```
`.slint`：`DateRangePicker { start <=> root.plan-start; end <=> root.plan-end; today: root.today; applied => { root.dates-applied(); } }`
（C# `Applied` 事件；值有沒有變由呼叫端比較。`single: true` 是只選一天。）
日曆是 PopupWindow，畫在視窗裡、會自動移進視窗範圍；視窗至少要約 600×400 邏輯像素才放得下兩個月。

### 3.3 TaskMenuArea：右鍵選單

做法：Slint `ContextMenuArea`，Windows 上是 muda 的**原生** Win32 選單（項目與文字照 C#）。
```slint
// 呼叫端根 .slint：export { TaskMenuState } from "@hbc/index.slint";
for t in tasks: Rectangle {
    menu := TaskMenuArea { key: t.key; }          // 墊在列底下，空白處直接右鍵就開
    TouchArea {                                    // 列自己的 TouchArea 會先吃掉右鍵，要自己轉給選單
        pointer-event(e) => {
            if e.button == PointerEventButton.right && e.kind == PointerEventKind.down {
                menu.open-at({ x: self.absolute-position.x + self.mouse-x, y: self.absolute-position.y + self.mouse-y });
            }
        }
    }
}
```
```rust
let menu = hb_ui_common::wire_task_menu!(ui, TaskMenuState, engine, hb_ui_common::TaskMenuHooks {
    open: Rc::new(move |key| { /* 查看詳細 */ }),
    delete: Rc::new(move |key| { /* 問過再刪：見 delete_task::confirm */ }),
    template_applied: Some(Rc::new(move || { /* 重新整理 */ })),
});   // 回傳 Rc<TaskMenu>，留著或丟掉都可以（回呼裡有 clone）
```
「改狀態」子選單：原生選單打開後內容固定，所以按右鍵時就讀專案的狀態清單；有快取馬上顯示，沒有就等讀完（最多 600ms，
逾時先顯示「讀取中…」）。選了狀態會 `set_task_status` 再 `after_external_change`。

### 3.4 ProjectIcon／ColorKey

```rust
let icon = hb_ui_common::ProjectIcons::icon(Some(&project), &project.name);   // ProjectIconData { glyph, color: Rgb }
row.glyph = icon.glyph.into();
row.tint = icon.color.to_slint();
row.time_color = vm.time_color.code();   // .slint：color: ColorKeys.color(row.time-color);
```

## 4. 純邏輯（Rust）

- `TaskText::title(title, &test)`、`TaskText::title_of(&summary)`；`KeyCopy::copy(engine, key)`、`KeyCopy::copy_link(engine, key)`、`task_text::set_clipboard(text)`。
- `ProjectIcons::glyph(icon, name)`、`color_of(color, key) -> Rgb`、`icon(project, name) -> ProjectIconData`。
- `inputs`（C# `UI/Work/Inputs.cs`）：`parse_date(text, today)`、`parse_time`、`date_text`、`short_date`、`local(day, time)`、`iso`、`ago`；
  `run(set_busy, show, action, success).await -> bool`（C# `Inputs.Run`：按鈕文字用 `BUSY_TEXT`「處理中…」，錯誤用 `ActionError`：
  Hayaku／網路錯誤直接顯示，其他加「發生錯誤：」）；`StatusMessage { text, error }`。
  C# `IWorkPage` 介面屬於工作台，沒有放這裡。
- `MarkdownTable::from_tsv`、`template(cols, rows)`、`insert(text, selection, table) -> (新文字, 游標)`；`INSERT_BUTTON_TEXT`／`_TIP`。
- `view_models`：`TaskRowVm`、`RepoRowVm::from(&repo, now, idle)`、`MeetingRowVm::from(&m, &now)`、`TodoRowVm`、`ReminderRowVm`、
  `NotificationRowVm::new(item, discussion)`；Brush → `ColorKey`，Visibility → bool。
- `window_util`：`hwnd`、`set_tool_window`（不佔工作列）、`set_dark_title_bar`、`activate`、`work_area`、`center_on_screen`、`center_on_owner`、`place_near`。

## 5. 小視窗

「最上層／不佔工作列／置中」都照 C#。owner：`Option<&slint::Window>`，置中在那個視窗上（C# CenterOwner）；Slint 沒有模態視窗，
**不會停用 owner**（C# ShowDialog 會）。C# `ShowDialog` 取結果的，這裡回傳 future（視窗關掉時完成），用 `slint::spawn_local` await。

| 函式 | C# | 說明 |
|---|---|---|
| `test_action::open(engine, &TestActionRequest, done: Option<Rc<dyn Fn(&str)>>)` | `TestActionWindow` | action "fix"＝開始修，其他＝退回；同一筆（action:key）只開一個；done 收伺服器說明 |
| `add_test_result::open(engine, test_task, test_title, parent: Option<&str>, siblings: &[(String, String)], added: Option<Rc<dyn Fn(&str)>>)` | `AddTestResultWindow` | 同一個測試任務只開一個（App 的 action "add" 開這個） |
| `advance_tests::open(engine, project, &TestSubStatus, tasks: Option<Vec<String>>, done)` | `AdvanceTestsWindow` | tasks None＝那個子狀態全部 |
| `apply_template::open(engine, project, tasks: Vec<String>, owner, done: Box<dyn Fn(bool)>)` | `ApplyTemplateWindow` | 視窗關掉時 done(有沒有套用)（C# `Applied`） |
| `batch_create::open(engine, project, parent: Option<&str>, created: Option<Rc<dyn Fn()>>)` | `BatchCreateWindow` | 建好後同步一次再呼叫 created（C# `Created`） |
| `delete_task::open(engine, key, title, children: Option<i32>, owner) -> impl Future<Output = bool>` | `DeleteTaskWindow` | 有沒有刪 |
| `delete_task::confirm(engine, key, title, children, owner).await -> bool` | `ConfirmAsync` | 刪了就 `after_external_change` |
| `move_task::pick_parent(engine, task, current_parent: Option<&str>, owner) -> impl Future<Output = Option<Option<String>>>` | `MoveTaskWindow` | None＝取消；Some(None)＝最上層；移動由呼叫端 `engine.move_task` |
| `task_picker::pick(&[TaskItem], prompt, multi_select) -> impl Future<Output = Option<Vec<String>>>` | `TaskPickerWindow` | None＝取消；多選可 Ctrl／Shift；可直接輸入編號 |
| `poll::show(engine, poll_id: Option<&str>, owner)` | `PollWindow.Show` | |
| `feedback::open(engine, number)` | `FeedbackWindow` | 同一則只開一個（App.OpenWorkspace("feedback")） |
| `file_preview::show(engine, id, name, owner, web_url: Option<&str>)` | `FilePreviewWindow.Show` | 同一個檔案已開就拉到前面；圖片 image crate 解碼、另存用 rfd；文字「繼續讀」 |
| `reject_review::open(engine, &ChangeRequest, demo: bool)` | `RejectReviewWindow` | 同一筆申請只開一個；demo 不送出 |

### 5.1 QuickCreate（建立任務）

```rust
use hb_ui_common::QuickCreate;
// engine.events().new_work_ready：提示跳出、資料讀好 → 先建好（C# PrewarmQuickCreate，800ms 後）
if QuickCreate::find(&s.session_id).is_none() && let Some(q) = QuickCreate::new(&engine, &s) {
    q.on_closed(move |r| if let Some(key) = &r.created_key { /* 提示「已建立 {key}」，r.assigned 決定第二行 */ });
    q.prewarm();
    // 12 分鐘沒按就 q.close()（C# App 的 DispatcherTimer，由呼叫端做）
}
// engine.events().create_task_requested：按了「建立任務」
let q = QuickCreate::find(&s.session_id).or_else(|| QuickCreate::new(&engine, &s));   // 新建的也要 on_closed
if let Some(q) = q { if q.presented() { q.activate() } else { q.present(anchor /* hb_win::Rect，實體像素 */, align_left) } }
```
`QuickCreateResult { created_key: Option<String>, created_title, assigned }`；`q.result()` 隨時可讀。

## 6. 其他

- 視窗圖示用 `crates/hayakubar/assets/app.ico`（跟 C# 一樣是 app.ico）。
- `✦`、`⤒` 這類符號 Inter／正黑體沒有，需要 hb-ui-kit 的 `init_fonts()` 把 `seguisym.ttf`（Segoe UI Symbol）加進 `SLINT_FONT_PATH`。
