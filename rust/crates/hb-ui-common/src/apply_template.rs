//! 對應 C# `UI/ApplyTemplateWindow.cs`：套用子任務範本（Hayaku 1.5.0 範本中心）。
//! 選範本，〔預覽〕先看會建哪些、日期（dryRun，不寫入），〔套用〕才建立。
//! 範本清單在打開時才讀；任務右鍵選單（一個任務）與專案頁勾選多個任務開這個。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use hb_api::{SubtaskTemplate, TemplateApplyResult};
use hb_core::TimerEngine;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::dialog;
use crate::ui::ApplyTemplateWindow;
use crate::window_util::{self, OnDrop, Placement};

dialog::impl_dialog_ui!(ApplyTemplateWindow);

const MAX: usize = 500;

/// 每個任務一行；認不出格式時顯示伺服器的說明或原文（截短，以 UTF-16 計 800）。
pub fn summary(result: &TemplateApplyResult) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(m) = result.message.as_deref().filter(|m| !m.is_empty()) {
        parts.push(m.to_owned());
    }
    parts.extend(result.lines.iter().cloned());
    if parts.is_empty() {
        parts.push(if result.raw.is_empty() {
            "完成".to_owned()
        } else if result.raw.encode_utf16().count() > 800 {
            format!("{}…", truncate_utf16(&result.raw, 800))
        } else {
            result.raw.clone()
        });
    }
    parts.join("\n")
}

fn truncate_utf16(s: &str, units: usize) -> String {
    let v: Vec<u16> = s.encode_utf16().take(units).collect();
    String::from_utf16_lossy(&v)
}

fn strings(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>()))
}

/// 開套用範本；視窗關掉時 done(有沒有套用成功)（C# `Applied`，呼叫端據此重新整理）。
pub fn open(
    engine: &TimerEngine,
    project: &str,
    tasks: Vec<String>,
    owner: Option<&slint::Window>,
    done: Box<dyn Fn(bool)>,
) {
    let Some(ui) = dialog::create("套用子任務範本", ApplyTemplateWindow::new) else {
        done(false);
        return;
    };
    let total = tasks.len();
    let tasks: Rc<Vec<String>> = Rc::new(tasks.into_iter().take(MAX).collect());
    ui.set_heading(
        if tasks.len() == 1 {
            format!("套到 {} 底下", tasks[0])
        } else {
            format!("套到 {project} 的 {} 個任務底下", tasks.len())
        }
        .into(),
    );
    if tasks.len() > 1 && tasks.len() <= 12 {
        ui.set_keys(tasks.join("、").into());
    }
    if total > MAX {
        ui.set_limit_note(format!("一次最多 {MAX} 個，只套前 {MAX} 個").into());
    }
    let applied = Rc::new(Cell::new(false));
    let templates: Rc<RefCell<Vec<SubtaskTemplate>>> = Rc::default();
    let flag = applied.clone();
    let id = window_util::keep(&ui, None, OnDrop::new(move || done(flag.get())));

    let selected = {
        let templates = templates.clone();
        move |ui: &ApplyTemplateWindow| -> Option<SubtaskTemplate> {
            usize::try_from(ui.get_current()).ok().and_then(|i| templates.borrow().get(i).cloned())
        }
    };
    {
        let weak = ui.as_weak();
        ui.on_selection_changed(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_result("".into());
            }
        });
    }
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let tasks = tasks.clone();
        let selected = selected.clone();
        ui.on_preview(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(template) = selected(&ui) else { return };
            ui.set_previewing(true);
            ui.set_error("".into());
            let (engine, tasks, weak) = (engine.clone(), tasks.clone(), weak.clone());
            let skip = ui.get_skip_existing();
            dialog::spawn(async move {
                let result = engine.api().apply_subtask_template(&tasks, &template.id, skip, true).await;
                let Some(ui) = weak.upgrade() else { return };
                match result {
                    Ok(r) => ui.set_result(format!("預覽（還沒建立）\n{}", summary(&r)).into()),
                    Err(e) => {
                        log::error!("預覽套用範本失敗: {}", e.0);
                        ui.set_error(e.0.into());
                    }
                }
                ui.set_previewing(false);
            });
        });
    }
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let tasks = tasks.clone();
        ui.on_apply(move || {
            let Some(ui) = weak.upgrade() else { return };
            let template = selected(&ui);
            let skip = ui.get_skip_existing();
            ui.set_error("".into());
            let (engine, tasks, applied) = (engine.clone(), tasks.clone(), applied.clone());
            dialog::run(&ui, id, "套用子任務範本", async move {
                let Some(template) = template else { return Err("請先選範本".to_owned()) };
                let result =
                    engine.api().apply_subtask_template(&tasks, &template.id, skip, false).await.map_err(|e| e.0)?;
                let text = summary(&result);
                let shown: Vec<&str> = tasks.iter().take(20).map(String::as_str).collect();
                log::info!(
                    "套用範本「{}」到 {}{}：{text}",
                    template.name,
                    shown.join("、"),
                    if tasks.len() > 20 { "…" } else { "" }
                );
                engine.show_notice(&if text.encode_utf16().count() > 120 {
                    format!("{}…", truncate_utf16(&text, 120))
                } else {
                    text
                });
                applied.set(true);
                engine.after_external_change().await;
                Ok(())
            });
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                window_util::close(&ui, id);
            }
        });
    }
    window_util::present(&ui, id, Placement::Owner(owner), true, (500.0, 320.0), crate::apply_theme);

    // 打開時才讀範本清單
    let weak = ui.as_weak();
    let engine = engine.clone();
    let project = project.to_owned();
    dialog::spawn(async move {
        let result = engine.api().list_subtask_templates(Some(&project)).await;
        let Some(ui) = weak.upgrade() else { return };
        match result {
            Err(e) => {
                log::error!("讀取子任務範本失敗: {}", e.0);
                ui.set_templates(strings(Vec::new()));
                ui.set_current(-1);
                ui.set_error(format!("讀取範本失敗：{}", e.0).into());
            }
            Ok(list) if list.is_empty() => {
                ui.set_templates(strings(["沒有能套到這個專案的範本".to_owned()]));
                ui.set_current(0);
            }
            Ok(list) => {
                ui.set_templates(strings(list.iter().map(SubtaskTemplate::label)));
                ui.set_descriptions(strings(list.iter().map(|t| t.description.clone().unwrap_or_default())));
                ui.set_current(0);
                *templates.borrow_mut() = list;
                ui.set_loaded(true);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 摘要_說明加每行_都沒有時用原文() {
        let r =
            TemplateApplyResult {
                lines: vec!["A-1：建 3 個".into()], message: Some("完成".into()), raw: "x".into()
            };
        assert_eq!("完成\nA-1：建 3 個", summary(&r));
        let r = TemplateApplyResult { lines: vec![], message: None, raw: String::new() };
        assert_eq!("完成", summary(&r));
        let long = "字".repeat(900);
        let r = TemplateApplyResult { lines: vec![], message: None, raw: long };
        assert_eq!(801, summary(&r).chars().count());
    }
}
