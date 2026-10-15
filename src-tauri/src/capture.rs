//! 屏幕捕获与显示器定位。
//!
//! 坐标系统说明（重要）：
//! - Tauri 的 `Monitor` / `cursor_position` 使用**物理像素**坐标；
//! - xcap 的 `Monitor::x()/y()/width()/height()` 使用**逻辑像素**坐标。
//!
//! 策略：窗口定位一律采用 Tauri 提供的物理坐标（与窗口系统同源，绝对可靠）；
//! 抓屏目标则用「光标物理坐标 ÷ 该屏缩放比」换算成逻辑坐标后交给 xcap 的
//! `from_point` 选择显示器，两侧口径保持一致。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{ImageFormat, imageops};
use serde::Serialize;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager};
use xcap::Monitor;

/// 防止全局热键/托盘菜单在截屏过程中重入。
pub static CAPTURING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Shot {
    /// data:image/png;base64,... 前端可直接喂给 Image / drawImage
    pub data_url: String,
    /// 图像像素尺寸（决定覆盖窗口大小）
    pub width: u32,
    /// 图像像素高度
    pub height: u32,
    /// 覆盖窗口应放置的物理坐标（左上角）
    pub x: i32,
    /// 覆盖窗口物理纵坐标
    pub y: i32,
    /// 显示器名称，用于状态提示
    pub monitor: String,
    /// 该显示器的缩放比（物理像素 / 逻辑像素）
    pub scale: f64,
}

/// 目标显示器信息（来自 Tauri，物理坐标）
struct Target {
    /// 在 xcap 显示器列表中的下标（可能为 None，届时按 0 处理）
    monitor_index: Option<usize>,
    win_x: i32,
    win_y: i32,
    scale: f64,
    name: String,
}

fn resolve_target(app: &AppHandle) -> Target {
    let mut t = Target {
        monitor_index: None,
        win_x: 0,
        win_y: 0,
        scale: 1.0,
        name: String::from("显示器"),
    };

    // 注意：cursor_position / available_monitors 定义在 Window 上，不在 Manager 上
    let win = app.get_webview_window("main");
    let cursor = win.as_ref().and_then(|w| w.cursor_position().ok());
    let monitors: Vec<tauri::Monitor> = win
        .as_ref()
        .and_then(|w| w.available_monitors().ok())
        .unwrap_or_default();

    // 1) 以物理坐标在 Tauri 的显示器列表里找光标所在屏
    let chosen = cursor
        .as_ref()
        .and_then(|c| {
            monitors.iter().find(|m| {
                let p = m.position();
                let s = m.size();
                let x0 = p.x as f64;
                let y0 = p.y as f64;
                let x1 = x0 + s.width as f64;
                let y1 = y0 + s.height as f64;
                c.x >= x0 && c.x < x1 && c.y >= y0 && c.y < y1
            })
        })
        .or_else(|| monitors.iter().find(|m| m.position().x == 0 && m.position().y == 0))
        .or_else(|| monitors.first());

    if let Some(m) = chosen {
        t.win_x = m.position().x;
        t.win_y = m.position().y;
        t.scale = if m.scale_factor() > 0.0 { m.scale_factor() } else { 1.0 };
        t.name = m.name().cloned().unwrap_or_else(|| String::from("显示器"));
    }

    // 2) 换算成逻辑坐标，交给 xcap 定位同一块屏
    if let Some(c) = cursor {
        let lx = (c.x / t.scale).round() as i32;
        let ly = (c.y / t.scale).round() as i32;
        if let Ok(hit) = Monitor::from_point(lx, ly) {
            if let Ok(hit_id) = hit.id() {
                if let Ok(all) = Monitor::all() {
                    t.monitor_index = all.iter().position(|m| m.id().ok() == Some(hit_id));
                }
            }
        }
    }

    t
}

/// 采样判断是否为全黑画面（macOS 未授权「屏幕录制」时的典型表现）
fn looks_blank(img: &image::RgbaImage) -> bool {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return true;
    }
    let step_x = (w / 10).max(1);
    let step_y = (h / 10).max(1);
    let mut samples = 0u32;
    let mut bright = 0u32;
    let mut y = 0;
    while y < h {
        let mut x = 0;
        while x < w {
            let p = img.get_pixel(x, y);
            samples += 1;
            if p[0] as u32 + p[1] as u32 + p[2] as u32 > 24 {
                bright += 1;
            }
            x += step_x;
        }
        y += step_y;
    }
    samples > 0 && bright * 20 < samples
}

/// 抓取光标所在显示器的整屏画面
pub fn grab(app: &AppHandle) -> Result<Shot, String> {
    let t = resolve_target(app);

    let monitors = Monitor::all().map_err(|e| format!("枚举显示器失败：{e}"))?;
    if monitors.is_empty() {
        return Err(String::from("未检测到任何显示器"));
    }
    let monitor = monitors
        .get(t.monitor_index.unwrap_or(0))
        .unwrap_or(&monitors[0]);

    let image = monitor
        .capture_image()
        .map_err(|e| format!("截屏失败：{e}"))?;

    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return Err(String::from("截屏结果为空"));
    }

    if looks_blank(&image) {
        #[cfg(target_os = "macos")]
        return Err(String::from(
            "画面全黑：请在「系统设置 → 隐私与安全性 → 屏幕录制」中勾选 SnapStudio 后重启应用",
        ));
    }

    // 兜底：若 Tauri 未提供显示器坐标，则用 xcap 的逻辑坐标 × 实测缩放比推算
    let (mut win_x, mut win_y) = (t.win_x, t.win_y);
    if win_x == 0 && win_y == 0 {
        let logical_w = monitor.width().unwrap_or(width).max(1) as f32;
        let real_scale = width as f32 / logical_w;
        win_x = (monitor.x().unwrap_or(0) as f32 * real_scale).round() as i32;
        win_y = (monitor.y().unwrap_or(0) as f32 * real_scale).round() as i32;
    }

    let mut png: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败：{e}"))?;

    Ok(Shot {
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(&png)),
        width,
        height,
        x: win_x,
        y: win_y,
        monitor: t.name,
        scale: t.scale,
    })
}

/// 触发一次完整截屏流程：隐藏主窗口 → 截取光标所在显示器 → 将主窗口铺满该显示器
/// → 通过事件把画面推给前端。由全局热键、托盘菜单、前端按钮共用。
pub fn capture_now(app: &AppHandle) -> Result<(), String> {
    if CAPTURING.swap(true, Ordering::SeqCst) {
        return Ok(()); // 已在截屏中，忽略重入
    }

    let result = (|| -> Result<(), String> {
        // 先隐藏主窗口，避免它出现在截图里
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.hide();
            std::thread::sleep(std::time::Duration::from_millis(160));
        }

        let shot = grab(app)?;

        // 把主窗口几何铺满整块显示器（用逻辑坐标，由 Tauri 乘以缩放比得到物理像素）
        if let Some(win) = app.get_webview_window("main") {
            let scale = if shot.scale > 0.0 { shot.scale } else { 1.0 };
            let _ = win.set_always_on_top(true);
            let _ = win.set_position(LogicalPosition::new(
                shot.x as f64 / scale,
                shot.y as f64 / scale,
            ));
            let _ = win.set_size(LogicalSize::new(
                shot.width as f64 / scale,
                shot.height as f64 / scale,
            ));
        }

        app.emit("snap:capture", &shot)
            .map_err(|e| format!("事件发送失败：{e}"))?;
        Ok(())
    })();

    if result.is_err() {
        CAPTURING.store(false, Ordering::SeqCst);
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.show();
        }
    }
    result
}

/// 抓取**全部显示器**并拼接成一张大图（多屏拼接）。
///
/// 以 Tauri 提供的物理坐标计算所有显示器的并集矩形，对每块屏用其中心点的
/// 逻辑坐标交给 xcap 定位并整屏捕获，再按物理偏移贴到同一张画布上。
pub fn grab_all(app: &AppHandle) -> Result<Shot, String> {
    let win = app.get_webview_window("main");
    let monitors: Vec<tauri::Monitor> = win
        .as_ref()
        .and_then(|w| w.available_monitors().ok())
        .unwrap_or_default();
    if monitors.is_empty() {
        return Err(String::from("未检测到任何显示器"));
    }

    // 计算所有显示器的物理并集矩形，并逐一抓取
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    struct Cap {
        img: image::RgbaImage,
        x: i32,
        y: i32,
    }
    let mut caps: Vec<Cap> = Vec::with_capacity(monitors.len());
    let mut window_scale = 1.0f64;

    for (i, m) in monitors.iter().enumerate() {
        let p = m.position();
        let s = m.size();
        let px = p.x;
        let py = p.y;
        let pw = s.width as i32;
        let ph = s.height as i32;
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px + pw);
        max_y = max_y.max(py + ph);

        let scale = if m.scale_factor() > 0.0 {
            m.scale_factor()
        } else {
            1.0
        };
        // 以第一块屏（含并集左上角）的缩放作为覆盖窗口缩放基准
        if i == 0 {
            window_scale = scale;
        }

        // 用屏幕中心的逻辑坐标在 xcap 中定位同一块显示器
        let cx = ((px + pw / 2) as f64 / scale).round() as i32;
        let cy = ((py + ph / 2) as f64 / scale).round() as i32;
        let xm = Monitor::from_point(cx, cy).map_err(|e| format!("定位显示器失败：{e}"))?;
        let image = xm.capture_image().map_err(|e| format!("截屏失败：{e}"))?;
        let (w, h) = image.dimensions();
        if w == 0 || h == 0 {
            return Err(String::from("截屏结果为空"));
        }
        caps.push(Cap { img: image, x: px, y: py });
    }

    let union_w = (max_x - min_x) as u32;
    let union_h = (max_y - min_y) as u32;
    if union_w == 0 || union_h == 0 {
        return Err(String::from("显示器并集尺寸非法"));
    }

    let mut canvas = image::RgbaImage::new(union_w, union_h);
    for c in &caps {
        let ox = (c.x - min_x) as i64;
        let oy = (c.y - min_y) as i64;
        imageops::overlay(&mut canvas, &c.img, ox, oy);
    }

    let mut png: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(canvas)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败：{e}"))?;

    Ok(Shot {
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(&png)),
        width: union_w,
        height: union_h,
        x: min_x,
        y: min_y,
        monitor: String::from("全部显示器"),
        scale: window_scale,
    })
}

/// 触发多屏拼接截屏流程（逻辑同 `capture_now`，但抓取全部显示器）。
pub fn capture_all_now(app: &AppHandle) -> Result<(), String> {
    if CAPTURING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let result = (|| -> Result<(), String> {
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.hide();
            std::thread::sleep(std::time::Duration::from_millis(160));
        }

        let shot = grab_all(app)?;

        if let Some(win) = app.get_webview_window("main") {
            let scale = if shot.scale > 0.0 { shot.scale } else { 1.0 };
            let _ = win.set_always_on_top(true);
            let _ = win.set_position(LogicalPosition::new(
                shot.x as f64 / scale,
                shot.y as f64 / scale,
            ));
            let _ = win.set_size(LogicalSize::new(
                shot.width as f64 / scale,
                shot.height as f64 / scale,
            ));
        }

        app.emit("snap:capture", &shot)
            .map_err(|e| format!("事件发送失败：{e}"))?;
        Ok(())
    })();

    if result.is_err() {
        CAPTURING.store(false, Ordering::SeqCst);
        if let Some(win) = app.get_webview_window("main") {
            let _ = win.show();
        }
    }
    result
}
