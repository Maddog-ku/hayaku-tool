// 工作台的 Slint：入口 ui/workspace.slint；`@hb` → hb-ui-kit/ui、`@hbc` → hb-ui-common/ui
fn main() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let library_paths = std::collections::HashMap::from([
        ("hb".to_string(), dir.join("../hb-ui-kit/ui")),
        ("hbc".to_string(), dir.join("../hb-ui-common/ui")),
    ]);
    let config = slint_build::CompilerConfiguration::new().with_library_paths(library_paths);
    slint_build::compile_with_config("ui/workspace.slint", config).expect("編譯 ui/workspace.slint 失敗");
}
