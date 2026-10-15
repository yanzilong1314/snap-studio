//! 前端调用的 Tauri 命令。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::{AppHandle, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
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
            .map_err(|e| format!("构造图像失败：{e}"))?
            .to_owned();
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

/// 把合成好的 PNG 字节写入「图片/SnapStudio」固定文件夹，返回完整路径。
/// 桌面端「一键存到固定文件夹」走这里；网页端没有该命令时回退为浏览器下载。
#[tauri::command]
pub fn save_image_to_folder(app: AppHandle, bytes: Vec<u8>, name: String) -> Result<String, String> {
    let dir = app
        .path()
        .picture_dir()
        .map_err(|e| format!("找不到图片目录：{e}"))?
        .join("SnapStudio");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败：{e}"))?;
    let path = dir.join(sanitize_name(&name));
    std::fs::write(&path, &bytes).map_err(|e| format!("写入失败：{e}"))?;
    Ok(path.to_string_lossy().to_string())
}

fn sanitize_name(name: &str) -> String {
    let n = name.trim();
    if n.is_empty() { return "snap.png".into(); }
    // 去除路径分隔符，避免越界写入
    n.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_")
}

/// 解析 PNG 文件头中的宽高（IHDR）。
fn png_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() >= 24 && &bytes[0..8] == &[137, 80, 78, 71, 13, 10, 26, 10] {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        Some((w, h))
    } else {
        None
    }
}

/// 钉图：把一张图作为透明、置顶、无边框、可拖动的独立窗口钉在桌面上。
/// 使用有限窗口标签池 pin-0..pin-15（见 capabilities/pins.json 授权）。
#[tauri::command]
pub fn pin_image(app: AppHandle, bytes: Vec<u8>) -> Result<String, String> {
    let (w, h) = png_dims(&bytes).unwrap_or((480, 320));
    let scale = (520.0 / (w.max(h) as f64)).min(1.0);
    let (dw, dh) = (w as f64 * scale, h as f64 * scale);

    let data_url = format!("data:image/png;base64,{}", STANDARD.encode(&bytes));
    let html = format!(
        "<!doctype html><html><head><meta charset='utf-8'>\
<style>html,body{{margin:0;background:transparent;overflow:hidden}}\
img{{display:block;width:100%;height:100%;object-fit:contain;cursor:move}}\
#hint{{position:fixed;right:8px;bottom:8px;font:11px sans-serif;color:#fff;\
background:rgba(0,0,0,.55);padding:3px 8px;border-radius:8px;pointer-events:none;opacity:0;transition:opacity .2s}}\
body:hover #hint{{opacity:.95}}</style></head>\
<body><img src='{}'><div id='hint'>拖动移动 · 双击关闭</div>\
<script>var w=window.__TAURI__.window.getCurrentWindow();\
var img=document.querySelector('img');\
img.addEventListener('mousedown',function(e){{w.startDragging();}});\
document.body.addEventListener('dblclick',function(){{w.close();}});\
</script></body></html>",
        data_url
    );

    let existing: Vec<String> = app
        .webview_windows()
        .values()
        .map(|w| w.label().to_string())
        .collect();
    let mut label: Option<String> = None;
    for i in 0..16u32 {
        let l = format!("pin-{}", i);
        if !existing.contains(&l) {
            label = Some(l);
            break;
        }
    }
    let label = label.ok_or_else(|| "最多同时钉 16 张，请先在托盘清除钉图".to_string())?;

    let doc_url = format!("data:text/html;base64,{}", STANDARD.encode(html.as_bytes()));

    let builder = WebviewWindowBuilder::new(&app, label.clone(), WebviewUrl::Web(doc_url))
        .title("钉图")
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(true)
        .inner_size(dw, dh);

    match builder.build() {
        Ok(w) => {
            let _ = w.set_focus();
            Ok(label)
        }
        Err(e) => Err(format!("创建钉图窗口失败：{e}")),
    }
}

/// 关闭所有钉图窗口。
pub fn clear_pins_impl(app: &AppHandle) -> usize {
    let labels: Vec<String> = app
        .webview_windows()
        .values()
        .map(|w| w.label().to_string())
        .filter(|l| l.starts_with("pin-"))
        .collect();
    let mut count = 0;
    for l in labels {
        if let Some(w) = app.get_webview_window(&l) {
            let _ = w.close();
            count += 1;
        }
    }
    count
}

#[tauri::command]
pub fn clear_pins(app: AppHandle) -> Result<usize, String> {
    Ok(clear_pins_impl(&app))
}
