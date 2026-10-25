use std::collections::HashMap;
use std::sync::Mutex;

use crate::model::DeviceBattery;
use crate::settings::AppSettings;

/// 应用共享状态（std::sync::Mutex，持锁期间不做 await / HID IO）。
#[derive(Default)]
pub struct AppState {
    /// 最近一次展示列表（db 身份归并后的设备）
    pub last_devices: Mutex<Vec<DeviceBattery>>,
    /// 最近一次 provider 原始输出（保留原始 id，供 hid 层 keep-last 链使用）
    pub raw_last: Mutex<Vec<DeviceBattery>>,
    /// 是否已向前端推送过至少一次（首轮强制推送，避免启动竞态丢事件）
    pub emitted_once: std::sync::atomic::AtomicBool,
    /// 当前处于「探测失败」状态的设备 id -> 展示名（用于 toast 去重）
    pub probe_failed: Mutex<HashMap<String, String>>,
    /// 设备 id -> 上次低电量通知时间戳（毫秒），1 小时内不重复通知
    pub low_notified_at: Mutex<HashMap<String, i64>>,
    /// 用户设置（setup 时从磁盘加载）
    pub settings: Mutex<AppSettings>,
    /// 已添加设备持久化存储（setup 时从磁盘加载，变更时写回）
    pub known: Mutex<crate::known::KnownStore>,
    /// 设备资料库 devices/*.json（setup 与手动刷新时重载）
    pub device_db: Mutex<Vec<crate::device_db::DeviceInfo>>,
    /// 序列化整轮采集：手动刷新与定时轮询并发时避免两个任务同时读写同一接收器
    pub collect_lock: tokio::sync::Mutex<()>,
}
