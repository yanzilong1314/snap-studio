//! 前端调用的 Tauri 命令。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::{AppHandle, LogicalSize};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::capture;

/// 触发一次截屏（隐藏主窗口 → 截屏 → 铺满显示器 → 推事件给前端）。
/// 前端无需关心返回值，截屏结果通过 `snap:capture` 事件送达。
#[tauri::command]
pub fn grab_screen(app: AppHandle) -> Result<(), String> {
    capture::capture_now(&app)
}

/// 把合成好的 PNG（base64）复制到系统剪贴板，或弹出保存对话框写盘。
#[tauri::command]
pub fn export_capture(
    app: AppHandle,
    png_base64: String,
    action: String,
) -> Result<String, String> {
    let bytes = STANDARD
        .decode(png_base64.trim())
        .map_err(|e| format!("PNG 解码失败：{e}"))?;

    if action == "copy" {
        let img = tauri::image::Image::from_bytes(&bytes)
            .map_err(|e| format!("构造图像失败：{e}"))?;
        app.clipboard()
            .write_image(&img)
            .map_err(|e| format!("写入剪贴板失败：{e}"))?;
        Ok("copied".into())
    } else {
        let picked = rfd::FileDialog::new()
            .set_file_name("snap.png")
            .add_filter("PNG 图片", &["png"])
            .save_file();
        match picked {
            Some(path) => {
                std::fs::write(&path, &bytes).map_err(|e| format!("保存失败：{e}"))?;
                Ok("saved".into())
            }
            None => Err("已取消保存".into()),
        }
    }
}

/// 截屏结束后把主窗口恢复成正常的编辑器尺寸并居中显示。
#[tauri::command]
pub fn reset_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(false);
        let _ = win.set_size(LogicalSize::new(1180.0, 780.0));
        let _ = win.center();
        let _ = win.show();
    }
    Ok(())
}
