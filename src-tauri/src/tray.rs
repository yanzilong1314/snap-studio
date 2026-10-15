//! 系统托盘：菜单 + 点击图标唤起截屏。

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, MenuEvent},
    tray::{TrayIconBuilder, TrayIconEvent},
    App, Manager,
};

pub fn build_tray(app: &App) -> Result<(), Box<dyn std::error::Error>> {
    let icon = app
        .default_window_icon()
        .cloned()
        .expect("未找到默认窗口图标，请在 tauri.conf.json 配置 icon");

    let menu = Menu::new(app)?;
    let capture = MenuItem::with_id(
        app,
        "capture",
        "截图 (Ctrl/⌘+Shift+A)",
        true,
        Some("CmdOrControl+Shift+A"),
    )?;
    let show = MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
    let clearpins = MenuItem::with_id(app, "clearpins", "清除所有钉图", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    menu.append_items(&[&capture, &show, &clearpins, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("SnapStudio 截图标注工作台")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event: MenuEvent| match event.id.as_ref() {
            "quit" => app.exit(0),
            "capture" => {
                if let Err(e) = crate::capture::capture_now(app) {
                    eprintln!("托盘触发截屏失败：{e}");
                }
            }
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "clearpins" => {
                let n = crate::commands::clear_pins_impl(app);
                eprintln!("已清除 {} 个钉图窗口", n);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { .. } = event {
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}
