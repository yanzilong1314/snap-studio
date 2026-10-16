mod capture;
mod commands;
mod tray;

use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcut("CmdOrControl+Shift+A")
                .expect("无效的全局快捷键")
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        // 按前端同步的截屏模式分流：整屏 / 多屏拼接 / 前台窗口
                        let mode = app
                            .try_state::<commands::CaptureMode>()
                            .map(|s| s.0.lock().unwrap().clone())
                            .unwrap_or_else(|| "screen".to_string());
                        let r = match mode.as_str() {
                            "all" => capture::capture_all_now(app),
                            "window" => capture::capture_window_now(app),
                            _ => capture::capture_now(app),
                        };
                        if let Err(e) = r {
                            eprintln!("全局热键截屏失败：{e}");
                        }
                    }
                })
                .build(),
        )
        .manage(commands::PinImages::default())
        .manage(commands::CaptureMode::default())
        .setup(|app| {
            tray::build_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::grab_screen,
            commands::grab_all_screens,
            commands::grab_window,
            commands::capture_region,
            commands::enter_long_mode,
            commands::scroll_at,
            commands::end_auto_scroll,
            commands::set_capture_mode,
            commands::export_capture,
            commands::reset_window,
            commands::finish_capture,
            commands::save_image_to_folder,
            commands::pin_image,
            commands::get_pin_image,
            commands::clear_pins,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
