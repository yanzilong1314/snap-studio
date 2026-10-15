// 桌面端入口：在 Windows 上隐藏控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    snap_studio_lib::run()
}
