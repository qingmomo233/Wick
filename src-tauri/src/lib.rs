mod collector;
mod commands;
mod device_db;
mod known;
mod model;
pub mod providers;
mod settings;
mod state;

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

use crate::state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_devices,
            commands::refresh_now,
            commands::remove_device,
            commands::reorder_devices,
            commands::get_settings,
            commands::set_settings,
            commands::close_app
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // 按用户设置分流：quit 则放行关闭（进程随之退出），否则最小化到托盘
                let quit = window
                    .app_handle()
                    .state::<AppState>()
                    .settings
                    .lock()
                    .map(|s| s.close_action == "quit")
                    .unwrap_or(false);
                if !quit {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .setup(|app| {
            // 加载用户设置
            let loaded = settings::load(app.handle());
            if let Ok(mut s) = app.state::<AppState>().settings.lock() {
                *s = loaded.clone();
            }
            // 窗口置顶按设置恢复
            if loaded.always_on_top {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_always_on_top(true);
                }
            }
            // 加载「已添加设备」库：检测到过的设备重启后依然保留
            let store = known::load(app.handle());
            // 播种展示列表：打开即见上次的设备（按保存的顺序），
            // 首轮采集完成后自动修正在线状态与读数
            {
                let order = store.order.clone();
                let mut seeded = store.devices.clone();
                seeded.sort_by_key(|d| order.iter().position(|id| *id == d.id).unwrap_or(usize::MAX));
                if let Ok(mut last) = app.state::<AppState>().last_devices.lock() {
                    *last = seeded;
                }
            }
            if let Ok(mut k) = app.state::<AppState>().known.lock() {
                *k = store;
            }
            // 加载设备资料库 devices/*.json（不存在则生成默认）
            let db = device_db::load(app.handle());
            if let Ok(mut d) = app.state::<AppState>().device_db.lock() {
                *d = db;
            }
            build_tray(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                collector::run(handle).await;
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run tauri application");
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示 Wick", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("wick-tray")
        .tooltip("Wick — 无线设备电量")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

fn show_main(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}
