use tauri::{Manager, State};

use crate::model::DeviceBattery;
use crate::settings::AppSettings;
use crate::state::AppState;

/// 返回最近一次采集结果（前端初始加载时调用，后续更新走事件推送）。
#[tauri::command]
pub fn get_devices(state: State<'_, AppState>) -> Vec<DeviceBattery> {
    state
        .last_devices
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// 手动触发一次立即采集并推送（刷新按钮）：重新检测全部设备（含被移除的）。
#[tauri::command]
pub async fn refresh_now(app: tauri::AppHandle) {
    crate::collector::refresh(&app).await;
}

/// 手动移除设备：卡片右下角红色按钮调用。
#[tauri::command]
pub fn remove_device(app: tauri::AppHandle, id: String) {
    crate::collector::remove_device(&app, &id);
}

/// 手动拖拽排序（id 列表，顶在前）。
#[tauri::command]
pub fn reorder_devices(app: tauri::AppHandle, order: Vec<String>) {
    crate::collector::reorder_devices(&app, &order);
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> AppSettings {
    state
        .settings
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn set_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    // 开机自启：交给插件落系统注册表 / LaunchAgent
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    let current = autolaunch.is_enabled().unwrap_or(false);
    if settings.autostart != current {
        let result = if settings.autostart {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        };
        result.map_err(|e| format!("设置开机自启失败: {e}"))?;
    }

    crate::settings::save(&app, &settings);
    if let Ok(mut s) = state.settings.lock() {
        *s = settings;
    }
    Ok(())
}

/// 标题栏关闭按钮：按用户设置最小化到托盘或退出程序。
#[tauri::command]
pub fn close_app(app: tauri::AppHandle, state: State<'_, AppState>) {
    let quit = state
        .settings
        .lock()
        .map(|s| s.close_action == "quit")
        .unwrap_or(false);
    if quit {
        app.exit(0);
        return;
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}
