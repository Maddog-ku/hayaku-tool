//! 對應 C# `UI/FilePreviewWindow.cs`：直接看檔案內容（read_file）。
//! 截圖、白板、設計稿這類圖片顯示圖，PDF、Word、Excel、CSV、文字檔顯示文字（很長的按「繼續讀」）。
//! 同一個檔案讀過就用快取；Hayaku 每次讀都留稽核紀錄，系統管理員可以關掉這個功能。

use std::cell::RefCell;
use std::rc::Rc;

use hb_api::FileContent;
use hb_core::TimerEngine;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::dialog;
use crate::ui::FilePreviewWindow;
use crate::window_util::{self, Placement};

/// 圖片 bytes → Slint 圖（image crate 解碼）。
pub fn decode_image(bytes: &[u8]) -> Result<Image, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(img.as_raw(), w, h);
    Ok(Image::from_rgba8(buffer))
}

/// 另存圖片的副檔名（依 MIME）。
pub fn image_ext(mime: Option<&str>) -> &'static str {
    match mime {
        Some("image/jpeg") => ".jpg",
        Some("image/webp") => ".webp",
        Some("image/gif") => ".gif",
        _ => ".png",
    }
}

/// 千分位（C# `N0`）。
pub fn thousands(n: i32) -> String {
    let s = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// 下面那一行（伺服器的說明、讀到哪裡）。
pub fn status_text(note: Option<&str>, next: Option<i32>) -> String {
    [note.map(str::to_owned), next.map(|n| format!("已讀到第 {} 字，還有後面", thousands(n)))]
        .into_iter()
        .flatten()
        .filter(|x| !x.trim().is_empty())
        .collect::<Vec<_>>()
        .join("　")
}

struct State {
    id: String,
    web_url: Option<String>,
    title: String,
    text: Option<String>,
    next: Option<i32>,
    content: Option<FileContent>,
}

/// 開預覽；同一個檔案已經開著就拉到前面。webUrl：讀不到時的退路（到網頁下載）。
pub fn show(engine: &TimerEngine, id: &str, name: &str, owner: Option<&slint::Window>, web_url: Option<&str>) {
    let tag = format!("file:{id}");
    if window_util::activate_tag(&tag) {
        return;
    }
    let Some(ui) = dialog::create("檔案預覽", FilePreviewWindow::new) else { return };
    ui.set_window_title(name.into());
    ui.set_can_web(web_url.is_some());
    let wid = window_util::keep(&ui, Some(tag), ());
    let state = Rc::new(RefCell::new(State {
        id: id.to_owned(),
        web_url: web_url.map(str::to_owned),
        title: name.to_owned(),
        text: None,
        next: None,
        content: None,
    }));
    let load: Rc<dyn Fn(i32)> = {
        let (weak, engine, state) = (ui.as_weak(), engine.clone(), state.clone());
        Rc::new(move |offset: i32| {
            let (weak, engine, state) = (weak.clone(), engine.clone(), state.clone());
            dialog::spawn(async move {
                let file_id = state.borrow().id.clone();
                let result = engine.files().read(&file_id, offset).await;
                let Some(ui) = weak.upgrade() else { return };
                load_done(&ui, &state, offset, result.map_err(|e| e.0));
            });
        })
    };
    {
        let load = load.clone();
        let state = state.clone();
        ui.on_continue_reading(move || {
            if let Some(next) = state.borrow().next {
                load(next);
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ui.on_copy_text(move || {
            let Some(ui) = weak.upgrade() else { return };
            let text = state.borrow().text.clone().unwrap_or_default();
            match crate::task_text::set_clipboard(&text) {
                Ok(()) => ui.set_status("已複製".into()),
                Err(_) => ui.set_status("剪貼簿被別的程式占用，再試一次".into()),
            }
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ui.on_save_image(move || {
            if let Some(ui) = weak.upgrade() {
                save_image(&ui, &state);
            }
        });
    }
    {
        let state = state.clone();
        ui.on_open_web(move || {
            if let Some(url) = &state.borrow().web_url {
                hb_win::open(url);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_dismiss(move || {
            if let Some(ui) = weak.upgrade() {
                window_util::close(&ui, wid);
            }
        });
    }
    window_util::present(&ui, wid, Placement::Owner(owner), false, (860.0, 640.0), crate::apply_theme);
    load(0);
}

fn load_done(ui: &FilePreviewWindow, state: &Rc<RefCell<State>>, offset: i32, result: Result<FileContent, String>) {
    let content = match result {
        Ok(c) => c,
        Err(message) => {
            let id = state.borrow().id.clone();
            log::error!("讀取檔案 {id} 失敗: {message}");
            if offset == 0 {
                ui.set_error(format!("讀不到內容：{message}").into());
            } else {
                ui.set_status(format!("讀不到後面的內容：{message}").into());
            }
            render_actions(ui, &state.borrow());
            return;
        }
    };
    let mut s = state.borrow_mut();
    if let Some(name) = content.name.as_deref().filter(|n| !n.is_empty())
        && offset == 0
    {
        s.title = name.to_owned();
        ui.set_window_title(name.into());
    }
    if let Some(bytes) = &content.image {
        match decode_image(bytes) {
            Ok(img) => {
                ui.set_picture(img);
                ui.set_is_image(true);
            }
            Err(e) => {
                log::error!("圖片解碼失敗: {e}");
                ui.set_error(format!("讀不到內容：{e}").into());
            }
        }
    } else {
        let text = content.text.clone().unwrap_or_default();
        match (&mut s.text, offset == 0) {
            (Some(existing), false) => existing.push_str(&text),
            _ => {
                s.text = Some(if text.is_empty() {
                    "（沒有文字；掃描的 PDF 沒有文字可讀）".to_owned()
                } else {
                    text
                })
            }
        }
        ui.set_text_content(s.text.clone().unwrap_or_default().into());
        ui.set_is_image(false);
    }
    s.next = content.next_offset;
    ui.set_status(status_text(content.note.as_deref(), s.next).into());
    s.content = Some(content);
    ui.set_loading(false);
    render_actions(ui, &s);
}

fn render_actions(ui: &FilePreviewWindow, s: &State) {
    ui.set_can_continue(s.next.is_some());
    ui.set_can_copy(s.text.is_some());
    ui.set_can_save(s.content.as_ref().is_some_and(|c| c.image.is_some()));
}

fn save_image(ui: &FilePreviewWindow, state: &Rc<RefCell<State>>) {
    let (bytes, ext, stem) = {
        let s = state.borrow();
        let Some(bytes) = s.content.as_ref().and_then(|c| c.image.clone()) else { return };
        let ext = image_ext(s.content.as_ref().and_then(|c| c.mime_type.as_deref()));
        let stem =
            std::path::Path::new(&s.title).file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        (bytes, ext, stem)
    };
    let mut dialog = rfd::FileDialog::new()
        .set_file_name(format!("{stem}{ext}"))
        .add_filter(format!("圖片 (*{ext})"), &[ext.trim_start_matches('.')]);
    let handle = ui.window().window_handle();
    if handle.window_handle().is_ok() && handle.display_handle().is_ok() {
        dialog = dialog.set_parent(&handle);
    }
    let Some(path) = dialog.save_file() else { return };
    match std::fs::write(&path, bytes) {
        Ok(()) => {
            ui.set_status(format!("已存到 {}（長邊縮到 1568 px 的版本；原檔請到網頁下載）", path.display()).into())
        }
        Err(e) => {
            log::error!("另存圖片失敗: {e}");
            ui.set_status(e.to_string().into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 千分位與狀態列() {
        assert_eq!("12,345", thousands(12345));
        assert_eq!("999", thousands(999));
        assert_eq!("1,000,000", thousands(1_000_000));
        assert_eq!("部分內容　已讀到第 20,000 字，還有後面", status_text(Some("部分內容"), Some(20000)));
        assert_eq!("", status_text(None, None));
    }

    #[test]
    fn 副檔名() {
        assert_eq!(".jpg", image_ext(Some("image/jpeg")));
        assert_eq!(".png", image_ext(None));
    }

    #[test]
    fn 解碼_png() {
        let mut buf = Vec::new();
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        let decoded = decode_image(&buf).unwrap();
        assert_eq!((3, 2), (decoded.size().width, decoded.size().height));
        assert!(decode_image(b"not an image").is_err());
    }
}
