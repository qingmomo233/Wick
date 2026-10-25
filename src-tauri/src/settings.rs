use serde::{Deserialize, Serialize};

fn default_poll_interval() -> u32 {
    5
}

/// 用户设置（持久化到 app_config_dir/settings.json）
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppSettings {
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "default_close_action")]
    pub close_action: String,
    #[serde(default = "default_bg_opacity")]
    pub bg_opacity: f32,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u32,
    #[serde(default)]
    pub always_on_top: bool,
    /// 记住的窗口尺寸（逻辑像素），None = 首次启动用默认值
    #[serde(default)]
    pub window_width: Option<u32>,
    #[serde(default)]
    pub window_height: Option<u32>,
}

fn default_close_action() -> String {
    "minimize".to_string()
}

fn default_bg_opacity() -> f32 {
    1.0
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            autostart: false,
            close_action: "minimize".to_string(),
            bg_opacity: 1.0,
            poll_interval_secs: 5,
            always_on_top: false,
            window_width: None,
            window_height: None,
        }
    }
}

/// 兼容旧配置缺字段 + 钳制非法值。
impl AppSettings {
    pub fn sanitized(self) -> Self {
        Self {
            autostart: self.autostart,
            close_action: if self.close_action == "quit" {
                "quit".to_string()
            } else {
                "minimize".to_string()
            },
            bg_opacity: self.bg_opacity.clamp(0.2, 1.0),
            poll_interval_secs: self.poll_interval_secs.clamp(2, 60),
            always_on_top: self.always_on_top,
            // 不小于窗口最小尺寸（tauri.conf.json 的 minWidth/minHeight）
            window_width: self.window_width.map(|w| w.max(360)),
            window_height: self.window_height.map(|h| h.max(480)),
        }
    }
}

fn settings_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

pub fn load(app: &tauri::AppHandle) -> AppSettings {
    let Some(path) = settings_path(app) else {
        return AppSettings::default();
    };
    if let Ok(text) = std::fs::read_to_string(&path) {
        return serde_json::from_str::<AppSettings>(&text)
            .map(|s| s.sanitized())
            .unwrap_or_else(|e| {
                eprintln!("[wick] warn: settings parse failed ({e}), using defaults");
                AppSettings::default()
            });
    }
    // 迁移旧版（Pulse, com.pulse.battery）配置：同一配置根目录下的兄弟目录
    if let Some(legacy) = path
        .parent()
        .and_then(|dir| dir.parent())
        .map(|config_root| config_root.join("com.pulse.battery").join("settings.json"))
        .filter(|p| p.exists())
    {
        if let Ok(text) = std::fs::read_to_string(&legacy) {
            if let Ok(s) = serde_json::from_str::<AppSettings>(&text).map(|s| s.sanitized()) {
                save(app, &s);
                return s;
            }
        }
    }
    AppSettings::default()
}

pub fn save(app: &tauri::AppHandle, settings: &AppSettings) {
    let Some(path) = settings_path(app) else {
        eprintln!("[wick] warn: no app config dir, settings not saved");
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(settings).unwrap_or_default()) {
        eprintln!("[wick] warn: settings save failed: {e}");
    }
}
