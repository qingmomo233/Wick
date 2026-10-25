use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const TRANSPORT_BLUETOOTH: &str = "bluetooth";
pub const TRANSPORT_NEARLINK: &str = "nearlink";
pub const TRANSPORT_24G: &str = "2.4g";
pub const TRANSPORT_UNKNOWN: &str = "unknown";

pub const STATUS_OK: &str = "ok";
pub const STATUS_UNSUPPORTED: &str = "unsupported";

/// 统一设备电量模型，前端 `src/types.ts` 与之严格对应。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DeviceBattery {
    pub id: String,
    pub name: String,
    /// headset|mouse|keyboard|gamepad|speaker|phone|other
    pub kind: String,
    pub level: Option<u8>,
    pub charging: Option<bool>,
    pub connected: bool,
    /// "bluetooth" | "nearlink" | "2.4g" | "unknown"
    pub transport: String,
    /// "windows-bt" | "bluez" | "hidpp" | "vendor-hid" | "generic-hid"
    pub source: String,
    /// "ok" | "unsupported" | "probe_failed"
    pub protocol_status: String,
    pub updated_at: i64,
}

impl DeviceBattery {
    pub fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// 判断两次采集是否为同一读数（忽略 updated_at）。
    pub fn same_reading(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.kind == other.kind
            && self.level == other.level
            && self.charging == other.charging
            && self.connected == other.connected
            && self.transport == other.transport
            && self.source == other.source
            && self.protocol_status == other.protocol_status
    }
}

/// 依据设备名称粗分设备类型（尽力而为，失败归为 other）。
pub fn classify_kind(name: &str) -> &'static str {
    let n = name.to_lowercase();
    let contains_any = |kws: &[&str]| kws.iter().any(|k| n.contains(k));
    if contains_any(&[
        "headset", "headphone", "earbud", "earphone", "buds", "airpods", "耳机", "耳塞",
    ]) {
        return "headset";
    }
    if contains_any(&["mouse", "鼠标", "trackball", "trackpad"]) {
        return "mouse";
    }
    if contains_any(&["keyboard", "键盘", "keychron"]) {
        return "keyboard";
    }
    if contains_any(&[
        "gamepad", "controller", "手柄", "handle", "xbox", "dualsense", "dualshock", "joy-con",
    ]) {
        return "gamepad";
    }
    if contains_any(&["speaker", "音箱", "soundlink"]) {
        return "speaker";
    }
    if contains_any(&["手机", "iphone", "phone"]) {
        return "phone";
    }
    "other"
}
