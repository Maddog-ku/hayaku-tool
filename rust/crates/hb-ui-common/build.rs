// hb-ui-common 的 Slint：入口 ui/windows.slint（各小視窗＋共用元件）；`@hb` → hb-ui-kit/ui、`@hbc` → 自己的 ui/。
// 其他 crate 用 library path `hbc` 指到這裡的 ui/，`import { DayTimeline } from "@hbc/index.slint";`。
fn main() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let library_paths = std::collections::HashMap::from([
        ("hb".to_string(), dir.join("../hb-ui-kit/ui")),
        ("hbc".to_string(), dir.join("ui")),
    ]);
    let config = slint_build::CompilerConfiguration::new().with_library_paths(library_paths);
    slint_build::compile_with_config("ui/windows.slint", config).expect("編譯 ui/windows.slint 失敗");
}
