//! 悬停智能选区（类企业微信）：检测鼠标所在处的「最小可见区域」。
//!
//! 检测优先级：
//! 1. UI Automation（UIA）：定位光标下的顶层窗口，再沿元素树向下钻取到包含
//!    光标的最深元素 —— 浏览器 / Office 等可精确到卡片、按钮级别；
//! 2. 回退：子窗口细化（ChildWindowFromPointEx）+ 顶层窗口 DWM 可见边界。
//!
//! 全部坐标为**物理屏幕像素**。UIA/COM 有线程亲和性，检测在独立线程中串行
//! 执行；调用方带超时等待，超时即放弃（晚到的结果由前端序号作废），保证悬停不卡顿。

use serde::Serialize;

/// 检测到的悬停区域（物理屏幕坐标）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HoverRegion {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// 当前鼠标物理坐标（直接读系统光标位置，与窗口无关）。
#[cfg(target_os = "windows")]
pub fn cursor_pos() -> Option<(i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut pt: POINT = unsafe { std::mem::zeroed() };
    if unsafe { GetCursorPos(&mut pt) } != 0 {
        Some((pt.x, pt.y))
    } else {
        None
    }
}

#[cfg(not(target_os = "windows"))]
pub fn cursor_pos() -> Option<(i32, i32)> {
    None
}

/// 检测 (x, y) 处的最小可见区域。exclude 为需要排除的自身窗口 HWND（isize）。
#[cfg(target_os = "windows")]
pub fn detect_at(x: i32, y: i32, exclude: &[isize]) -> Option<HoverRegion> {
    win_impl::detect(x, y, exclude)
}

#[cfg(not(target_os = "windows"))]
pub fn detect_at(_x: i32, _y: i32, _exclude: &[isize]) -> Option<HoverRegion> {
    None
}

// ==================== Windows 实现 ====================

#[cfg(target_os = "windows")]
mod win_impl {
    use std::sync::mpsc;
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    use uiautomation::core::{UIAutomation, UIElement};
    use uiautomation::types::TreeScope;
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ChildWindowFromPointEx, CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT, GetWindowRect,
        ScreenToClient,
    };

    use super::HoverRegion;

    type Request = (i32, i32, Vec<isize>, mpsc::Sender<Option<HoverRegion>>);

    /// 检测线程的请求管道（惰性初始化，线程内复用 UIA 实例）。
    static PIPE: OnceLock<Mutex<mpsc::Sender<Request>>> = OnceLock::new();
    /// 单次检测等待上限：超时放弃，晚到结果由前端序号作废。
    const WAIT: Duration = Duration::from_millis(260);

    pub fn detect(x: i32, y: i32, exclude: &[isize]) -> Option<HoverRegion> {
        let tx = PIPE
            .get_or_init(|| {
                let (tx, rx) = mpsc::channel::<Request>();
                let _ = std::thread::Builder::new()
                    .name("snap-hover-detect".into())
                    .spawn(move || worker(rx));
                Mutex::new(tx)
            })
            .lock()
            .unwrap()
            .clone();
        let (rtx, rrx) = mpsc::channel();
        if tx.send((x, y, exclude.to_vec(), rtx)).is_err() {
            return None;
        }
        rrx.recv_timeout(WAIT).ok().flatten()
    }

    fn worker(rx: mpsc::Receiver<Request>) {
        let mut uia: Option<UIAutomation> = None;
        while let Ok(req) = rx.recv() {
            // 只处理最新请求：调用方超时放弃的积压请求直接丢弃
            let mut req = req;
            while let Ok(newer) = rx.try_recv() {
                req = newer;
            }
            let (x, y, exclude, rtx) = req;
            if uia.is_none() {
                uia = UIAutomation::new().ok();
            }
            let _ = rtx.send(detect_impl(uia.as_ref(), x, y, &exclude));
        }
    }

    fn detect_impl(
        uia: Option<&UIAutomation>,
        x: i32,
        y: i32,
        exclude: &[isize],
    ) -> Option<HoverRegion> {
        let hwnd = top_window_at(x, y, exclude)?;
        if let Some(aut) = uia {
            if let Some(r) = uia_element_at(aut, hwnd, x, y) {
                return normalize(r, hwnd);
            }
        }
        window_rect_at(hwnd, x, y)
    }

    /// Z 序从上到下找到第一个「可见、未隐藏、未遮罩、包含该点」的外部顶层窗口。
    fn top_window_at(x: i32, y: i32, exclude: &[isize]) -> Option<isize> {
        use windows_sys::Win32::Foundation::LPARAM;
        use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetWindowRect, IsIconic, IsWindowVisible,
        };

        struct Ctx<'a> {
            x: i32,
            y: i32,
            exclude: &'a [isize],
            found: Option<isize>,
        }
        unsafe extern "system" fn on_win(hwnd: HWND, lparam: LPARAM) -> i32 {
            let ctx = unsafe { &mut *(lparam as *mut Ctx) };
            if ctx.found.is_some() {
                return 1;
            }
            if ctx.exclude.contains(&(hwnd as isize)) {
                return 1;
            }
            if unsafe { IsWindowVisible(hwnd) } == 0 || unsafe { IsIconic(hwnd) } != 0 {
                return 1;
            }
            // DWM 遮罩（如最小化到宿主的 UWP 窗口）跳过
            let mut cloaked: u32 = 0;
            let hr = unsafe {
                DwmGetWindowAttribute(
                    hwnd,
                    DWMWA_CLOAKED,
                    &mut cloaked as *mut u32 as *mut core::ffi::c_void,
                    4,
                )
            };
            if hr == 0 && cloaked != 0 {
                return 1;
            }
            let mut r: RECT = unsafe { std::mem::zeroed() };
            if unsafe { GetWindowRect(hwnd, &mut r) } == 0 {
                return 1;
            }
            if r.right - r.left < 8 || r.bottom - r.top < 8 {
                return 1;
            }
            let (px, py) = (ctx.x, ctx.y);
            if px < r.left || px >= r.right || py < r.top || py >= r.bottom {
                return 1;
            }
            ctx.found = Some(hwnd as isize);
            0
        }
        let mut ctx = Ctx { x, y, exclude, found: None };
        unsafe { EnumWindows(Some(on_win), &mut ctx as *mut Ctx as LPARAM) };
        ctx.found
    }

    /// UIA：从顶层窗口元素树向下钻取包含光标的最深元素（逐层只看直接子级）。
    fn uia_element_at(
        aut: &UIAutomation,
        hwnd: isize,
        x: i32,
        y: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        let cond = aut.create_true_condition().ok()?;
        let root = aut.element_from_handle(hwnd.into()).ok()?;
        let mut el: UIElement = root;
        for _ in 0..16 {
            let children = match el.find_all(TreeScope::Children, &cond) {
                Ok(c) => c,
                Err(_) => break,
            };
            let mut next = None;
            for c in children {
                if let Ok(r) = c.get_bounding_rectangle() {
                    let (l, t, rt, b) = (r.get_left(), r.get_top(), r.get_right(), r.get_bottom());
                    if r.get_width() > 2 && r.get_height() > 2 && x >= l && x <= rt && y >= t && y <= b {
                        next = Some(c);
                        break;
                    }
                }
            }
            match next {
                Some(c) => el = c,
                None => break,
            }
        }
        let r = el.get_bounding_rectangle().ok()?;
        Some((r.get_left(), r.get_top(), r.get_width(), r.get_height()))
    }

    /// 回退：可见子窗口下钻两层 + DWM 可见边界。
    fn window_rect_at(hwnd: isize, x: i32, y: i32) -> Option<HoverRegion> {
        unsafe {
            let mut h = hwnd as HWND;
            for _ in 0..2 {
                let mut pt = POINT { x, y };
                if ScreenToClient(h, &mut pt) == 0 {
                    break;
                }
                let child = ChildWindowFromPointEx(h, pt, CWP_SKIPTRANSPARENT | CWP_SKIPINVISIBLE);
                if child == 0 || child == h {
                    break;
                }
                h = child;
            }
            let mut r: RECT = std::mem::zeroed();
            if GetWindowRect(h, &mut r) == 0 {
                return None;
            }
            normalize((r.left, r.top, r.right - r.left, r.bottom - r.top), hwnd)
        }
    }

    /// DWM 可见边界（排除窗口阴影与不可见缩放边框）。
    fn extended_bounds(hwnd: HWND) -> Option<RECT> {
        use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
        let mut r: RECT = unsafe { std::mem::zeroed() };
        let hr = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut RECT as *mut core::ffi::c_void,
                std::mem::size_of::<RECT>() as u32,
            )
        };
        if hr == 0 {
            Some(r)
        } else {
            None
        }
    }

    /// 收尾：矩形太碎丢弃；若元素矩形就是整个窗口（含阴影边），换成 DWM 可见边界。
    fn normalize(r: (i32, i32, i32, i32), hwnd: isize) -> Option<HoverRegion> {
        let (mut rx, mut ry, mut rw, mut rh) = r;
        if rw < 4 || rh < 4 {
            return None;
        }
        let hwnd_h = hwnd as HWND;
        unsafe {
            let mut wr: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd_h, &mut wr) != 0
                && rx == wr.left
                && ry == wr.top
                && rx + rw - 1 == wr.right
                && ry + rh - 1 == wr.bottom
            {
                if let Some(b) = extended_bounds(hwnd_h) {
                    rx = b.left;
                    ry = b.top;
                    rw = b.right - b.left;
                    rh = b.bottom - b.top;
                }
            }
        }
        if rw < 4 || rh < 4 {
            return None;
        }
        Some(HoverRegion { x: rx, y: ry, w: rw, h: rh })
    }
}
