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

/// 以显式物理坐标定位目标显示器（区域捕获 / 窗口捕获用）。
fn resolve_point(app: &AppHandle, cpx: i32, cpy: i32) -> Target {
    let mut t = Target {
        monitor_index: None,
        win_x: 0,
        win_y: 0,
        scale: 1.0,
        name: String::from("显示器"),
    };
    let win = app.get_webview_window("main");
    let monitors: Vec<tauri::Monitor> = win
        .as_ref()
        .and_then(|w| w.available_monitors().ok())
        .unwrap_or_default();
    let cx = cpx as f64;
    let cy = cpy as f64;
    let chosen = monitors
        .iter()
        .find(|m| {
            let p = m.position();
            let s = m.size();
            let x0 = p.x as f64;
            let y0 = p.y as f64;
            cx >= x0 && cx < x0 + s.width as f64 && cy >= y0 && cy < y0 + s.height as f64
        })
        .or_else(|| monitors.first());
    if let Some(m) = chosen {
        t.win_x = m.position().x;
        t.win_y = m.position().y;
        t.scale = if m.scale_factor() > 0.0 {
            m.scale_factor()
        } else {
            1.0
        };
        t.name = m.name().cloned().unwrap_or_else(|| String::from("显示器"));
    }
    let lx = (cpx as f64 / t.scale).round() as i32;
    let ly = (cpy as f64 / t.scale).round() as i32;
    if let Ok(hit) = Monitor::from_point(lx, ly) {
        if let Ok(hit_id) = hit.id() {
            if let Ok(all) = Monitor::all() {
                t.monitor_index = all.iter().position(|m| m.id().ok() == Some(hit_id));
            }
        }
    }
    t
}

/// 前台窗口矩形（物理屏幕坐标）。
/// - Windows：user32 GetForegroundWindow + GetWindowRect（本身就是物理像素）；
/// - macOS：osascript 调 System Events 取前台窗口（逻辑点），排除自身应用后
///   按所在显示器缩放比换算成物理像素；取不到 / 未授权时返回 None（回退整屏）。
fn front_window_rect(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    let _ = app; // Windows 实现不使用 app（macOS 换算缩放比需要）
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::{HWND, RECT};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd == 0 {
                return None;
            }
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut r) == 0 {
                return None;
            }
            Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
        }
    }
    #[cfg(target_os = "macos")]
    {
        front_window_rect_macos(app)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = app;
        None
    }
}

/// macOS：osascript + System Events 读取前台窗口矩形（逻辑点 → 物理像素）。
/// 需要「辅助功能」权限；被拒绝、无窗口或解析失败时返回 None（调用方回退整屏）。
#[cfg(target_os = "macos")]
fn front_window_rect_macos(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    use std::process::Command;
    // 输出：应用路径|窗口x|窗口y|宽|高（System Events 使用逻辑点坐标）
    let script = "tell application \"System Events\"\n  try\n    set fp to first application process whose frontmost is true\n    set appPath to POSIX path of (application file of fp) as string\n    set fw to front window of fp\n    set p to position of fw\n    set s to size of fw\n    return appPath & \"|\" & (item 1 of p as string) & \"|\" & (item 2 of p as string) & \"|\" & (item 1 of s as string) & \"|\" & (item 2 of s as string)\n  end try\nend tell";
    let out = Command::new("osascript").arg("-e").arg(script).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<&str> = text.trim().split('|').collect();
    if parts.len() != 5 {
        return None;
    }
    // 前台若为本应用（按钮触发时主窗在前台），交回退逻辑处理
    if let Some(our) = our_app_bundle_path() {
        let a = parts[0].trim_end_matches('/');
        if a == our.trim_end_matches('/') {
            return None;
        }
    }
    let lx: f64 = parts[1].trim().parse().ok()?;
    let ly: f64 = parts[2].trim().parse().ok()?;
    let lw: f64 = parts[3].trim().parse().ok()?;
    let lh: f64 = parts[4].trim().parse().ok()?;
    if lw < 8.0 || lh < 8.0 {
        return None;
    }
    Some(logical_to_physical(app, lx, ly, lw, lh))
}

/// 本应用 .app bundle 路径（macOS），如 /Applications/SnapStudio.app
#[cfg(target_os = "macos")]
fn our_app_bundle_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let s = exe.to_str()?;
    let idx = s.find("/Contents/MacOS/")?;
    Some(s[..idx].to_string())
}

/// 逻辑点矩形 → 物理像素矩形：取包含窗口中心的显示器，按该屏缩放比换算。
/// （混合 DPI 跨屏窗口为近似值，以窗口中心所在屏的缩放比为准。）
#[cfg(target_os = "macos")]
fn logical_to_physical(app: &AppHandle, lx: f64, ly: f64, lw: f64, lh: f64) -> (i32, i32, i32, i32) {
    let win = app.get_webview_window("main");
    let monitors: Vec<tauri::Monitor> = win
        .as_ref()
        .and_then(|w| w.available_monitors().ok())
        .unwrap_or_default();
    let cx = lx + lw / 2.0;
    let cy = ly + lh / 2.0;
    for m in &monitors {
        let p = m.position();
        let s = m.size();
        let sc = if m.scale_factor() > 0.0 {
            m.scale_factor()
        } else {
            1.0
        };
        let x0 = p.x as f64 / sc;
        let y0 = p.y as f64 / sc;
        let x1 = x0 + s.width as f64 / sc;
        let y1 = y0 + s.height as f64 / sc;
        if cx >= x0 && cx < x1 && cy >= y0 && cy < y1 {
            return (
                (lx * sc).round() as i32,
                (ly * sc).round() as i32,
                (lw * sc).round() as i32,
                (lh * sc).round() as i32,
            );
        }
    }
    (
        lx.round() as i32,
        ly.round() as i32,
        lw.round() as i32,
        lh.round() as i32,
    )
}

/// 两矩形 (x,y,w,h) 相交面积是否≥前者面积一半（判断前台窗口是否即本应用主窗口）。
fn rects_overlap(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    let ix0 = ax.max(bx);
    let iy0 = ay.max(by);
    let ix1 = (ax + aw).min(bx + bw);
    let iy1 = (ay + ah).min(by + bh);
    let inter = (ix1 - ix0).max(0) * (iy1 - iy0).max(0);
    let fa = aw.max(0) * ah.max(0);
    fa > 0 && inter * 2 >= fa
}

/// 捕获指定物理矩形区域并裁剪返回（长截图逐帧 / 窗口捕获共用）。
fn capture_region_image(
    app: &AppHandle,
    px: i32,
    py: i32,
    w: i32,
    h: i32,
) -> Result<image::RgbaImage, String> {
    let t = resolve_point(app, px + w / 2, py + h / 2);
    let monitors = Monitor::all().map_err(|e| format!("枚举显示器失败：{e}"))?;
    if monitors.is_empty() {
        return Err(String::from("未检测到任何显示器"));
    }
    let monitor = monitors.get(t.monitor_index.unwrap_or(0)).unwrap_or(&monitors[0]);
    let image = monitor.capture_image().map_err(|e| format!("截屏失败：{e}"))?;
    let (iw, ih) = image.dimensions();
    let ox = (px - t.win_x).max(0) as u32;
    let oy = (py - t.win_y).max(0) as u32;
    if ox >= iw || oy >= ih {
        return Err(String::from("区域超出屏幕范围"));
    }
    let cw = (w.max(0) as u32).min(iw - ox);
    let ch = (h.max(0) as u32).min(ih - oy);
    if cw == 0 || ch == 0 {
        return Err(String::from("区域尺寸非法"));
    }
    // 用负偏移把整屏图盖到目标画布上，等效裁剪出 [ox,oy,cw,ch]（屏幕图不透明）
    let mut out = image::RgbaImage::new(cw, ch);
    imageops::overlay(&mut out, &image, -(ox as i64), -(oy as i64));
    Ok(out)
}

/// 把主窗口铺满到 shot 覆盖的屏幕区域（物理坐标 ÷ 缩放比 = 逻辑坐标）。
fn place_overlay(app: &AppHandle, shot: &Shot) {
    if let Some(win) = app.get_webview_window("main") {
        let scale = if shot.scale > 0.0 { shot.scale } else { 1.0 };
        let _ = win.set_always_on_top(true);
        let _ = win.set_position(LogicalPosition::new(shot.x as f64 / scale, shot.y as f64 / scale));
        let _ = win.set_size(LogicalSize::new(shot.width as f64 / scale, shot.height as f64 / scale));
    }
}

/// 捕获前台窗口：抓取后裁剪到前台窗口矩形；若前台窗口就是本应用主窗口或取不到，
/// 则优雅回退为整屏捕获。结果同样通过 `snap:capture` 事件推送。
pub fn capture_window_now(app: &AppHandle) -> Result<(), String> {
    if CAPTURING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = (|| -> Result<(), String> {
        // 先在隐藏主窗前取前台窗口（热键触发时目标应用在前台）
        let front = front_window_rect(app);
        let main_rect = app.get_webview_window("main").and_then(|w| {
            let p = w.outer_position().ok()?;
            let s = w.outer_size().ok()?;
            Some((p.x, p.y, s.width as i32, s.height as i32))
        });
        let target = front.filter(|f| main_rect.map_or(true, |mr| !rects_overlap(*f, mr)));

        if let Some(win) = app.get_webview_window("main") {
            let _ = win.hide();
            std::thread::sleep(std::time::Duration::from_millis(160));
        }

        let shot = if let Some((x, y, w, h)) = target {
            let scale = resolve_point(app, x + w / 2, y + h / 2).scale;
            let img = capture_region_image(app, x, y, w, h)?;
            let (cw, ch) = img.dimensions();
            let mut png: Vec<u8> = Vec::new();
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                .map_err(|e| format!("PNG 编码失败：{e}"))?;
            Shot {
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(&png)),
                width: cw,
                height: ch,
                x,
                y,
                monitor: String::from("前台窗口"),
                scale,
            }
        } else {
            grab(app)?
        };

        place_overlay(app, &shot);
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

/// 按物理坐标裁剪捕获指定区域（长截图逐帧捕获命令用）。
pub fn capture_region_png(
    app: &AppHandle,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
) -> Result<Shot, String> {
    let img = capture_region_image(app, x, y, w, h)?;
    let (cw, ch) = img.dimensions();
    let mut png: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|e| format!("PNG 编码失败：{e}"))?;
    Ok(Shot {
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(&png)),
        width: cw,
        height: ch,
        x,
        y,
        monitor: String::from("区域"),
        scale: 1.0,
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

// ==================== 长截图：自动滚动 + 浮动控制条 ====================

/// Windows：自动滚动前记录原始光标位置，结束时还原（避免光标乱跳）。
#[cfg(target_os = "windows")]
static ORIG_CURSOR: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// 向指定物理坐标发送一次「向下翻页」滚动。
/// - Windows：先把光标移到目标点再发滚轮事件，事件直达光标下的目标窗口；
///   控制条容器 pointer-events:none，滚轮可穿透本应用窗口。
/// - macOS：向最前面应用发送 Page Down（需辅助功能权限，每次一页）。
#[cfg(target_os = "windows")]
pub fn scroll_at(_app: &AppHandle, x: i32, y: i32, notches: i32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOUSEEVENTF_WHEEL, mouse_event};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};
    let n = notches.clamp(-10, 10);
    if n == 0 {
        return Ok(());
    }
    unsafe {
        // 首次调用记录原光标位置
        let mut guard = ORIG_CURSOR.lock().unwrap();
        if guard.is_none() {
            let mut pt: POINT = std::mem::zeroed();
            if GetCursorPos(&mut pt) != 0 {
                *guard = Some((pt.x, pt.y));
            }
        }
        drop(guard);
        if SetCursorPos(x, y) == 0 {
            return Err(String::from("移动光标失败，无法自动滚动"));
        }
        // 负值 = 向下滚动（内容上移）；一档 = 120 单位
        let delta: i32 = n * -120;
        mouse_event(MOUSEEVENTF_WHEEL, 0, 0, delta, 0);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn scroll_at(_app: &AppHandle, _x: i32, _y: i32, notches: i32) -> Result<(), String> {
    use std::process::Command;
    if notches <= 0 {
        return Ok(());
    }
    // System Events 无滚轮注入，用 Page Down（key code 121）代替，每次一页
    let out = Command::new("osascript")
        .args(["-e", "tell application \"System Events\" to key code 121"])
        .output()
        .map_err(|e| format!("调用 osascript 失败：{e}"))?;
    if !out.status.success() {
        return Err(String::from(
            "无法发送翻页键：请在「系统设置 → 隐私与安全性 → 辅助功能」中授权 SnapStudio",
        ));
    }
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn scroll_at(_app: &AppHandle, _x: i32, _y: i32, _notches: i32) -> Result<(), String> {
    Err(String::from("当前平台不支持自动滚动"))
}

/// 结束自动滚动：还原光标位置（Windows；其他平台为空操作）。
pub fn end_auto_scroll() {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;
        let orig = {
            let mut guard = ORIG_CURSOR.lock().unwrap();
            guard.take()
        };
        if let Some((x, y)) = orig {
            unsafe {
                let _ = SetCursorPos(x, y);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // 其他平台无需还原
    }
}

/// 进入长截图模式：把主窗口缩小为底部居中的浮动控制条（露出目标窗口）。
/// (cx, cy) 为捕获区域中心的物理坐标，用于选择所在显示器定位控制条。
pub fn enter_long_mode(app: &AppHandle, w: f64, h: f64, cx: i32, cy: i32) -> Result<(), String> {
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| String::from("找不到主窗口"))?;
    let monitors: Vec<tauri::Monitor> = win
        .available_monitors()
        .map_err(|e| format!("枚举显示器失败：{e}"))?;
    let cxf = cx as f64;
    let cyf = cy as f64;
    let chosen = monitors
        .iter()
        .find(|m| {
            let p = m.position();
            let s = m.size();
            let x0 = p.x as f64;
            let y0 = p.y as f64;
            cxf >= x0 && cxf < x0 + s.width as f64 && cyf >= y0 && cyf < y0 + s.height as f64
        })
        .or_else(|| monitors.first());
    let (lx, ly) = if let Some(m) = chosen {
        let p = m.position();
        let s = m.size();
        let sc = if m.scale_factor() > 0.0 {
            m.scale_factor()
        } else {
            1.0
        };
        let mw = s.width as f64 / sc;
        let mh = s.height as f64 / sc;
        let mx0 = p.x as f64 / sc;
        let my0 = p.y as f64 / sc;
        let x = mx0 + (mw - w).max(0.0) / 2.0;
        let y = (my0 + mh - h - 88.0).max(my0);
        (x, y)
    } else {
        (120.0, 120.0)
    };
    win.set_always_on_top(true)
        .map_err(|e| format!("设置置顶失败：{e}"))?;
    win.set_size(LogicalSize::new(w, h))
        .map_err(|e| format!("调整窗口大小失败：{e}"))?;
    win.set_position(LogicalPosition::new(lx, ly))
        .map_err(|e| format!("移动窗口失败：{e}"))?;
    let _ = win.show();
    Ok(())
}
