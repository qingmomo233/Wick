//! 「已添加设备」持久化：读到过电量的设备永久保留，睡眠断开时由
//! collector 合成离线卡片。手动移除后进入隐藏名单，设备从系统消失
//! 后重连即自动恢复。

use serde::{Deserialize, Serialize};

use crate::model::DeviceBattery;

/// 存储结构版本：不匹配时整体重置（结构或语义变更时递增，用于清掉历史脏数据）
const STORE_VERSION: u32 = 3;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct KnownStore {
    #[serde(default)]
    pub version: u32,
    /// 所有检测到过的设备（最近一次在线快照，按 id 去重）
    pub devices: Vec<DeviceBattery>,
    /// 用户手动移除、当前仍被隐藏的设备 id
    pub removed: Vec<String>,
    /// 展示顺序（id 列表，顶在前）。新设备插入到最前，拖拽排序覆盖此列表
    #[serde(default)]
    pub order: Vec<String>,
}

fn store_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    use tauri::Manager;
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("known_devices.json"))
}

pub fn load(app: &tauri::AppHandle) -> KnownStore {
    let Some(path) = store_path(app) else {
        return KnownStore::default();
    };
    let store = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<KnownStore>(&text).unwrap_or_else(|e| {
            eprintln!("[wick] warn: known_devices parse failed ({e}), starting empty");
            KnownStore::default()
        }),
        Err(_) => KnownStore::default(),
    };
    // 版本不匹配（旧格式/语义变更）→ 重置，避免旧脏数据（幽灵设备）残留
    if store.version != STORE_VERSION {
        return KnownStore {
            version: STORE_VERSION,
            ..Default::default()
        };
    }
    store
}

pub fn save(app: &tauri::AppHandle, store: &KnownStore) {
    let Some(path) = store_path(app) else {
        eprintln!("[wick] warn: no app config dir, known devices not saved");
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut store = store.clone();
    store.version = STORE_VERSION;
    if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&store).unwrap_or_default()) {
        eprintln!("[wick] warn: known_devices save failed: {e}");
    }
}
