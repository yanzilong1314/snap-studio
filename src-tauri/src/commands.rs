//! 前端调用的 Tauri 命令。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::HashMap;
use std::sync::{Mutex, atomic::Ordering};
use tauri::{AppHandle, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::capture;

/// 钉图窗口待显示的图片（base64），按窗口标签暂存，页面加载后取走。
#[derive(Default)]
pub struct PinImages(pub Mutex<HashMap<String, String>>);

/// 触发一次截屏（隐藏主窗口 → 截屏 → 铺满显示器 → 推事件给前端）。
/// 前端无需关心返回值，截屏结果通过 `snap:capture` 事件送达。
#[tauri::command]
pub fn grab_screen(app: AppHandle) -> Result<(), String> {
    capture::capture_now(&app)
}

/// 触发多屏拼接截屏：隐藏主窗口 → 抓取全部显示器并拼成一张大图 →
/// 铺满并集区域 → 推事件给前端。结果同样通过 `snap:capture` 事件送达。
#[tauri::command]
pub fn grab_all_screens(app: AppHandle) -> Result<(), String> {
    capture::capture_all_now(&app)
}

/// 触发前台窗口截屏：抓取后裁剪到前台窗口矩形；取不到则回退整屏。
#[tauri::command]
pub fn grab_window(app: AppHandle) -> Result<(), String> {
    capture::capture_window_now(&app)
}

/// 当前截屏模式（供全局热键分流）："screen" | "all" | "window"。
#[derive(Default)]
pub struct CaptureMode(pub std::sync::Mutex<String>);

/// 前端切换「范围」选择器时同步截屏模式，供全局热键分流使用。
#[tauri::command]
pub fn set_capture_mode(state: tauri::State<'_, CaptureMode>, mode: String) -> Result<(), String> {
    let m = match mode.as_str() {
        "all" => "all",
        "window" => "window",
        _ => "screen",
    };
    *state.0.lock().unwrap() = m.to_string();
    Ok(())
}

/// 长截图：按物理坐标裁剪捕获指定区域，返回 data URL 供前端逐帧拼接。
/// 仅当主窗与捕获区域重叠时才短暂隐藏（长截图浮动控制条多数情况不重叠，避免逐帧闪烁）。
#[tauri::command]
pub fn capture_region(app: AppHandle, x: i32, y: i32, w: i32, h: i32) -> Result<crate::capture::Shot, String> {
    let win = app.get_webview_window("main");
    let overlap = win.as_ref().map_or(true, |wd| {
        match wd.outer_position().ok().zip(wd.outer_size().ok()) {
            Some((p, s)) => {
                let ix = (x + w).min(p.x + s.width as i32) - x.max(p.x);
                let iy = (y + h).min(p.y + s.height as i32) - y.max(p.y);
                ix > 0 && iy > 0
            }
            None => true,
        }
    });
    if overlap {
        if let Some(wd) = &win {
            let _ = wd.hide();
            std::thread::sleep(std::time::Duration::from_millis(90));
        }
    }
    let result = capture::capture_region_png(&app, x, y, w, h);
    if overlap {
        if let Some(wd) = &win {
            let _ = wd.show();
        }
    }
    result
}

/// 长截图模式：把主窗口缩小为底部居中的浮动控制条，露出目标窗口供滚动。
/// (cx, cy) 为捕获区域中心的物理坐标，用于选择所在显示器。
#[tauri::command]
pub fn enter_long_mode(app: AppHandle, w: f64, h: f64, cx: i32, cy: i32) -> Result<(), String> {
    capture::enter_long_mode(&app, w, h, cx, cy)
}

/// 长截图自动滚动：向区域中心发送一次「向下翻页」（Windows 滚轮 / macOS Page Down）。
#[tauri::command]
pub fn scroll_at(app: AppHandle, x: i32, y: i32, notches: i32) -> Result<(), String> {
    capture::scroll_at(&app, x, y, notches)
}

/// 结束自动滚动：还原光标位置（Windows）。
#[tauri::command]
pub fn end_auto_scroll() -> Result<(), String> {
    capture::end_auto_scroll();
    Ok(())
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

/// 结束截屏会话（前端在复制/保存/取消后调用）：
/// 复位防重入标志 + 恢复窗口几何并隐藏回托盘 —— 类企业微信「用完即走」，
/// 截完图不再把主窗口弹到前台；全局热键在隐藏状态下依旧可用。
#[tauri::command]
pub fn finish_capture(app: AppHandle) -> Result<(), String> {
    capture::CAPTURING.store(false, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_always_on_top(false);
        let _ = win.set_size(LogicalSize::new(1180.0, 780.0));
        let _ = win.center();
        let _ = win.hide();
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
/// 图片 base64 暂存在 PinImages 中，由 pin.html 加载后取走。
#[tauri::command]
pub fn pin_image(app: AppHandle, bytes: Vec<u8>) -> Result<String, String> {
    let (w, h) = png_dims(&bytes).unwrap_or((480, 320));
    let scale = (520.0 / (w.max(h) as f64)).min(1.0);
    let (dw, dh) = (w as f64 * scale, h as f64 * scale);

    // 选取空闲标签
    let existing: Vec<String> = app
        .webview_windows()
        .values()
        .map(|w| w.label().to_string())
        .collect();

    // 清理已关闭窗口但残留的暂存图片，避免标签池被占满
    if let Some(store) = app.try_state::<PinImages>() {
        let mut map = store.0.lock().unwrap();
        let alive: std::collections::HashSet<String> = existing.iter().cloned().collect();
        map.retain(|k, _| alive.contains(k));
    }

    let mut label: Option<String> = None;
    for i in 0..16u32 {
        let l = format!("pin-{}", i);
        if !existing.contains(&l) {
            label = Some(l);
            break;
        }
    }
    let label = label.ok_or_else(|| "最多同时钉 16 张，请先在托盘清除钉图".to_string())?;

    // 暂存图片 base64，等待 pin.html 加载后取走（取走即删除，避免堆积）
    let b64 = STANDARD.encode(&bytes);
    if let Some(store) = app.try_state::<PinImages>() {
        store.0.lock().unwrap().insert(label.clone(), b64);
    }

    let builder = WebviewWindowBuilder::new(&app, label.clone(), WebviewUrl::App("pin.html".into()))
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

/// 钉图窗口（pin.html）加载时调用，取走对应标签暂存的图片 base64。
#[tauri::command]
pub fn get_pin_image(app: AppHandle, label: String) -> Result<String, String> {
    let store = app
        .try_state::<PinImages>()
        .ok_or_else(|| "钉图状态未初始化".to_string())?;
    let mut map = store.0.lock().unwrap();
    map.remove(&label)
        .ok_or_else(|| "图片已取走或窗口标签无效".to_string())
}

/// 关闭所有钉图窗口。
pub fn clear_pins_impl(app: &AppHandle) -> usize {
    // 同时清理暂存图片，防止标签池被残留占满
    if let Some(store) = app.try_state::<PinImages>() {
        let mut map = store.0.lock().unwrap();
        let keys: Vec<String> = map
            .keys()
            .cloned()
            .filter(|k| k.starts_with("pin-"))
            .collect();
        for k in keys {
            map.remove(&k);
        }
    }

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
