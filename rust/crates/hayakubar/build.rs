// HayakuBar 執行檔的建置：
// 1. 編譯 ui/app.slint（`@hb` → hb-ui-kit/ui、`@hbc` → hb-ui-common/ui）。
// 2. 嵌入 Windows 資源：程式圖示（assets/app.ico）、manifest（assets/app.manifest，跟 C# src/HayakuBar/app.manifest 一樣：
//    PerMonitorV2、supportedOS Windows 10/11；沒有它 hb-win 嵌進工作列的 layered 子視窗建不起來）、版本資訊（workspace 版本）。
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let library_paths = std::collections::HashMap::from([
        ("hb".to_string(), dir.join("../hb-ui-kit/ui")),
        ("hbc".to_string(), dir.join("../hb-ui-common/ui")),
    ]);
    let config = slint_build::CompilerConfiguration::new().with_library_paths(library_paths);
    slint_build::compile_with_config("ui/app.slint", config).expect("編譯 ui/app.slint 失敗");

    let assets = dir.join("assets");
    println!("cargo:rerun-if-changed={}", assets.join("app.ico").display());
    println!("cargo:rerun-if-changed={}", assets.join("app.manifest").display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc = out.join("app.rc");
    std::fs::write(&rc, resource_script(&assets)).expect("寫入 app.rc 失敗");
    // 只嵌進 HayakuBar.exe（測試執行檔不用）
    embed_resource::compile_for(&rc, ["HayakuBar"], embed_resource::NONE)
        .manifest_required()
        .expect("嵌入圖示與 manifest 失敗");
}

/// 版本資訊用 workspace 的版本（例 2.0.0 → 2,0,0,0）。
fn resource_script(assets: &std::path::Path) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let mut parts: Vec<u32> = version.split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect();
    parts.resize(4, 0);
    let comma = parts.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let path = |name: &str| assets.join(name).display().to_string().replace('\\', "\\\\");
    format!(
        r#"#pragma code_page(65001)
1 ICON "{icon}"
1 24 "{manifest}"
1 VERSIONINFO
FILEVERSION {comma}
PRODUCTVERSION {comma}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040404B0"
    BEGIN
      VALUE "CompanyName", "HayakuBar"
      VALUE "FileDescription", "HayakuBar"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "HayakuBar"
      VALUE "OriginalFilename", "HayakuBar.exe"
      VALUE "ProductName", "HayakuBar"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0404, 1200
  END
END
"#,
        icon = path("app.ico"),
        manifest = path("app.manifest"),
    )
}
