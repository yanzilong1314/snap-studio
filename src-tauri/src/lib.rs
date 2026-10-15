mod capture;
mod commands;
mod tray;

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
                        if let Err(e) = capture::capture_now(app) {
                            eprintln!("全局热键截屏失败：{e}");
                        }
                    }
                })
                .build(),
        )
        .setup(|app| {
            tray::build_tray(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::grab_screen,
            commands::export_capture,
            commands::reset_window,
            commands::save_image_to_folder,
            commands::pin_image,
            commands::clear_pins,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
