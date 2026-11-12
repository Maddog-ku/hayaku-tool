//! HayakuBar 程式主體（對應 C# `src/HayakuBar`：App.xaml(.cs)、UI/PanelWindow、HoverCard、ToastWindow、ThemeManager、SettingsWindow）。
//!
//! 啟動順序跟 C# `App.OnStartup` 一樣：`--mcp-call` 診斷模式 → 單一執行個體 → 紀錄 → 設定與 TimerEngine → 各視窗與系統匣。
//! 測試用：設環境變數 `HAYAKUBAR_DATA_DIR` 時，設定檔與紀錄改放那個資料夾，單一執行個體的名稱也不同（不跟正在用的 HayakuBar 衝突）。
//!
//! 程式碼放在 lib（crate 名稱 `hayakubar`），紀錄的 target 才會是 `hayakubar::…`（hb-backend 的紀錄只收 `hb_`、`hayakubar` 開頭的）；
//! 執行檔 `src/main.rs` 只呼叫 [`run`]。

pub mod app;
pub mod cli;
pub mod dialogs;
pub mod hover_card;
pub mod menu;
pub mod panel;
pub mod popup;
pub mod settings;
pub mod shot;
pub mod theme_manager;
pub mod toast;

#[allow(clippy::all)]
pub(crate) mod ui {
    slint::include_modules!();
}

use hb_backend::infra::AppPaths;

/// 測試用的資料夾（不碰使用者真正的設定）。
const DATA_DIR_ENV: &str = "HAYAKUBAR_DATA_DIR";

/// 程式進入點（C# `App.OnStartup`／`OnExit`）。
pub fn run() {
    // 中文字型退回設定要在建立任何 Slint 視窗、其他執行緒之前（會改環境變數）
    hb_ui_kit::init_fonts();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = cli::parse(&args);
    let test_dir = std::env::var_os(DATA_DIR_ENV).filter(|d| !d.is_empty());
    if let Some(dir) = &test_dir {
        AppPaths::set_dir(Some(std::path::Path::new(dir)));
    }

    // 診斷用：HayakuBar.exe --mcp-call <工具名稱> <參數 JSON> <輸出檔>
    if options.mcp_call {
        std::process::exit(hb_backend::infra::diagnostics::mcp_call(&args));
    }

    let instance_name = match &test_dir {
        Some(_) => format!("{}.Test", hb_win::single_instance::MUTEX_NAME),
        None => hb_win::single_instance::MUTEX_NAME.to_owned(),
    };
    let Some(instance) = hb_win::SingleInstance::acquire_named(&instance_name) else {
        dialogs::show(None, "HayakuBar", "HayakuBar 已經在執行了（看工作列）。", dialogs::Level::Info);
        return;
    };

    hb_backend::infra::log::init();
    // C# DispatcherUnhandledException／UnobservedTaskException：寫紀錄
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("未處理的例外: {info}");
        default_hook(info);
    }));
    hb_backend::mcp::mcp_stats::start();

    // 面板、提示、摘要建立時就設好「無邊框、最上層、不佔工作列、不搶焦點」
    if let Err(e) = slint::BackendSelector::new().with_winit_window_attributes_hook(popup::attributes_hook).select() {
        log::error!("無法啟動介面: {e}");
        return;
    }

    let Some(app) = app::App::start(options) else {
        log::error!("無法建立面板");
        return;
    };
    if let Err(e) = slint::run_event_loop_until_quit() {
        log::error!("事件迴圈結束: {e}");
    }
    app.exit();
    drop(instance);
}
