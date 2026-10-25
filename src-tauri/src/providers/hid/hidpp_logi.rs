//! 罗技 HID++ 协议（自实现；crates.io 的 hidpp crate 纯异步、缺 0x1000/0x1001）。
//! - 短报文 ReportID 0x10（7 字节），长报文 0x11（20 字节）
//! - 设备索引：0xFF = 接收器自身，0x00 = 直连设备，1..=6 = 接收器配对槽位
//! - HID++ 2.0：Root(0x0000).GetFeature 枚举 feature，读 UnifiedBattery(0x1004)
//!   / BatteryLevelStatus(0x1000)；HID++ 1.0：SubID 0x81 寄存器 0x07
//! - 错误应答 feature 位为 0x8F

use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};

use crate::model::{
    classify_kind, DeviceBattery, STATUS_OK, TRANSPORT_24G, TRANSPORT_BLUETOOTH,
};
use crate::providers::hid::{HidDev, ProbeError};

const REPORT_SHORT: u8 = 0x10;
const REPORT_LONG: u8 = 0x11;
const SHORT_LEN: usize = 7;
const LONG_LEN: usize = 20;
const FEATURE_IDX_ERROR: u8 = 0x8F;
const ROOT_FEATURE_IDX: u8 = 0x00;
const ROOT_GET_FEATURE_FN: u8 = 0x00;

const FID_DEVICE_TYPE_NAME: u16 = 0x0005;
const FID_BATTERY_LEVEL_STATUS: u16 = 0x1000;
const FID_UNIFIED_BATTERY: u16 = 0x1004;

/// 单个 HID++ 请求的等待上限。
const REQUEST_MS: u64 = 300;

/// 一组罗技接口探测的整体结果。
pub enum GroupProbe {
    /// 至少一台设备（或明确无设备）
    Devices(Vec<DeviceBattery>),
    /// 静默跳过（直连设备不可用 / 无电池 / 有线连接）
    Nothing,
    /// 打开接口失败（多为 Linux hidraw 权限问题）
    IoError(String),
}

struct Channel {
    dev: HidDevice,
}

impl Channel {
    /// 发送 HID++ 2.0 请求并等待匹配应答。
    /// 返回应答 payload（从字节 4 起）；无应答 / 错误应答返回 None。
    fn request(
        &self,
        dev_idx: u8,
        feature_idx: u8,
        function: u8,
        params: &[u8],
        long: bool,
        timeout_ms: u64,
    ) -> Option<Vec<u8>> {
        let mut buf = [0u8; LONG_LEN];
        let (rid, used) = if long {
            (REPORT_LONG, LONG_LEN)
        } else {
            (REPORT_SHORT, SHORT_LEN)
        };
        buf[0] = rid;
        buf[1] = dev_idx;
        buf[2] = feature_idx;
        buf[3] = (function << 4) | 0x01; // software id = 1
        if params.len() + 4 > used {
            return None;
        }
        buf[4..4 + params.len()].copy_from_slice(params);

        if !self.write_with_fallback(&buf[..used]) {
            return None;
        }
        self.read_response(dev_idx, feature_idx, timeout_ms)
    }

    fn write_with_fallback(&self, frame: &[u8]) -> bool {
        // Windows 下输出报文长度需匹配报告描述符；失败时尝试另一长度补零
        if self.dev.write(frame).is_ok() {
            return true;
        }
        let alt_len = if frame.len() == LONG_LEN { SHORT_LEN } else { LONG_LEN };
        let mut alt = frame.to_vec();
        if alt.len() < alt_len {
            alt.resize(alt_len, 0);
        } else {
            alt.truncate(alt_len);
        }
        self.dev.write(&alt).is_ok()
    }

    /// 读取直到拿到匹配应答（0x8F 错误应答视为「不支持」返回 None）。
    fn read_response(&self, dev_idx: u8, feature_idx: u8, timeout_ms: u64) -> Option<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut buf = [0u8; 64];
        while Instant::now() < deadline {
            match self.dev.read(&mut buf) {
                Ok(n) if n >= 4 => {
                    if buf[0] != REPORT_SHORT && buf[0] != REPORT_LONG {
                        continue; // DJ / 其他报告，跳过
                    }
                    if buf[1] != dev_idx {
                        continue;
                    }
                    if buf[2] == FEATURE_IDX_ERROR {
                        return None;
                    }
                    if buf[2] == feature_idx {
                        return Some(buf[4..n].to_vec());
                    }
                    // 其他 feature 的异步通知，忽略
                }
                _ => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        None
    }

    /// HID++ 1.0 寄存器读取（SubID 0x81）。返回 3 字节寄存器值。
    fn request_register_read(&self, dev_idx: u8, register: u8, timeout_ms: u64) -> Option<[u8; 3]> {
        let mut buf = [0u8; SHORT_LEN];
        buf[0] = REPORT_SHORT;
        buf[1] = dev_idx;
        buf[2] = 0x81; // 寄存器读
        buf[3] = register;
        if !self.write_with_fallback(&buf) {
            return None;
        }
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut rbuf = [0u8; 64];
        while Instant::now() < deadline {
            match self.dev.read(&mut rbuf) {
                Ok(n) if n >= 7 => {
                    if rbuf[0] != REPORT_SHORT && rbuf[0] != REPORT_LONG {
                        continue;
                    }
                    if rbuf[1] != dev_idx {
                        continue;
                    }
                    if rbuf[2] == FEATURE_IDX_ERROR {
                        return None;
                    }
                    if rbuf[2] == 0x81 && rbuf[3] == register {
                        return Some([rbuf[4], rbuf[5], rbuf[6]]);
                    }
                }
                _ => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        None
    }

    /// Root.GetFeature：查询 feature id 对应的 feature index（0 表示不支持）。
    fn feature_index(&self, dev_idx: u8, feature_id: u16) -> Option<u8> {
        let resp = self.request(
            dev_idx,
            ROOT_FEATURE_IDX,
            ROOT_GET_FEATURE_FN,
            &[(feature_id >> 8) as u8, (feature_id & 0xFF) as u8],
            true,
            REQUEST_MS,
        )?;
        let idx = *resp.first()?;
        if idx == 0 {
            None
        } else {
            Some(idx)
        }
    }
}

fn open_channels(api: &HidApi, interfaces: &[HidDev]) -> Vec<Channel> {
    let mut sorted: Vec<&HidDev> = interfaces.iter().collect();
    // 经典 HID++ 接口（usage page 0xFF00, usage 1-4）优先
    sorted.sort_by_key(|d| {
        let classic = d.usage_page == 0xFF00 && (1..=4).contains(&d.usage);
        (!classic, d.usage_page)
    });
    sorted
        .iter()
        .filter_map(|d| {
            let path = std::ffi::CString::from_vec_with_nul(d.path.clone()).ok()?;
            let dev = api.open_path(&path).ok()?;
            let _ = dev.set_blocking_mode(false);
            Some(Channel { dev })
        })
        .collect()
}

// ---------- 对外探测入口 ----------

/// 接收器：顺序扫描配对槽位 1..=6。
pub fn probe_receiver(interfaces: &[HidDev]) -> GroupProbe {
    let api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => return GroupProbe::IoError(e.to_string()),
    };
    let channels = open_channels(&api, interfaces);
    if channels.is_empty() {
        return GroupProbe::IoError("no hidpp interface opened (udev permission?)".into());
    }

    let mut devices = Vec::new();
    let id_base = format!("{:04x}-{:04x}", interfaces[0].vid, interfaces[0].pid);
    for slot in 1u8..=6 {
        // 先 2.0（长报文），再 1.0（短报文寄存器）
        if let Some(db) = try_slot_hidpp20(&channels, slot, &id_base) {
            devices.push(db);
            continue;
        }
        if let Some(db) = try_slot_hidpp10(&channels, slot, &id_base) {
            devices.push(db);
        }
        // 均无应答：空槽或设备离线，静默跳过
    }
    GroupProbe::Devices(devices)
}

/// 直连设备（蓝牙直连 / USB 线）：仅探测设备索引 0x00。
/// 只保留「蓝牙直连且有电量」的设备；有线连接不在本应用监控范围。
pub fn probe_direct(interfaces: &[HidDev]) -> GroupProbe {
    let bluetooth: Vec<HidDev> = interfaces
        .iter()
        .filter(|d| d.bus_is_bluetooth())
        .cloned()
        .collect();
    if bluetooth.is_empty() {
        return GroupProbe::Nothing; // USB 有线直连，跳过
    }
    let api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => return GroupProbe::IoError(e.to_string()),
    };
    let channels = open_channels(&api, &bluetooth);
    if channels.is_empty() {
        return GroupProbe::IoError("no hidpp interface opened (udev permission?)".into());
    }

    let id_base = format!("{:04x}-{:04x}", bluetooth[0].vid, bluetooth[0].pid);
    let device = try_slot_hidpp20(&channels, 0x00, &id_base)
        .or_else(|| try_slot_hidpp10(&channels, 0x00, &id_base));
    match device {
        Some(mut db) => {
            db.transport = TRANSPORT_BLUETOOTH.to_string();
            GroupProbe::Devices(vec![db])
        }
        None => GroupProbe::Nothing,
    }
}

// ---------- 槽位探测 ----------

fn try_slot_hidpp20(channels: &[Channel], slot: u8, id_base: &str) -> Option<DeviceBattery> {
    for ch in channels {
        // UnifiedBattery(0x1004)：新设备，含充电状态
        if let Some(fidx) = ch.feature_index(slot, FID_UNIFIED_BATTERY) {
            let caps = ch.request(slot, fidx, 0x00, &[], true, REQUEST_MS);
            let percentage_caps = caps
                .as_ref()
                .and_then(|c| c.first().copied())
                .map(|c| c & 0x01 != 0)
                .unwrap_or(false);
            if let Some(status) = ch.request(slot, fidx, 0x01, &[], true, REQUEST_MS) {
                let level = status.first().copied().map(|v| {
                    if percentage_caps {
                        v.min(100)
                    } else {
                        // 粗粒度 0..=10 档位折算
                        (v.min(10)).saturating_mul(10)
                    }
                });
                let charging = status.get(1).copied().map(|s| s == 1 || s == 2);
                return Some(build_device(ch, slot, id_base, level, charging));
            }
        }
        // BatteryLevelStatus(0x1000)：上一代
        if let Some(fidx) = ch.feature_index(slot, FID_BATTERY_LEVEL_STATUS) {
            if let Some(status) = ch.request(slot, fidx, 0x00, &[], true, REQUEST_MS) {
                let level = status.first().copied().filter(|v| *v <= 100);
                // status[2]: 0=放电 1=充电中 2=已充满 3=故障
                let charging = status.get(2).copied().map(|s| s == 1 || s == 2);
                return Some(build_device(ch, slot, id_base, level, charging));
            }
        }
    }
    None
}

fn try_slot_hidpp10(channels: &[Channel], slot: u8, id_base: &str) -> Option<DeviceBattery> {
    for ch in channels {
        // 老设备电池寄存器 0x07：[level, next, status]
        if let Some([level_raw, _next, status]) = ch.request_register_read(slot, 0x07, REQUEST_MS)
        {
            if level_raw == 0xFF {
                continue; // 无电池
            }
            // 部分设备返回 0..=4 粗档位（近似折算），部分直接返回百分比
            let level = if level_raw <= 4 {
                match level_raw {
                    0 => 5,
                    1 => 20,
                    2 => 50,
                    _ => 90,
                }
            } else {
                level_raw.min(100)
            };
            let charging = status == 1 || status == 2;
            let mut db = build_device(ch, slot, id_base, Some(level), Some(charging));
            db.name = format!("罗技设备 #{slot}");
            db.kind = classify_kind(&db.name).to_string();
            return Some(db);
        }
    }
    None
}

/// 读取设备名称与类型（feature 0x0005），组装 DeviceBattery。
fn build_device(
    ch: &Channel,
    slot: u8,
    id_base: &str,
    level: Option<u8>,
    charging: Option<bool>,
) -> DeviceBattery {
    let mut name = format!("罗技设备 #{slot}");
    let mut kind = "other".to_string();

    if let Some(fidx) = ch.feature_index(slot, FID_DEVICE_TYPE_NAME) {
        if let Some(resp) = ch.request(slot, fidx, 0x00, &[], true, REQUEST_MS) {
            if let Some(type_byte) = resp.first() {
                kind = match type_byte {
                    0..=2 => "keyboard",
                    3..=5 => "mouse",
                    _ => "other",
                }
                .to_string();
            }
            let raw = &resp[1.min(resp.len())..];
            let text = String::from_utf8_lossy(raw)
                .trim_end_matches('\0')
                .trim()
                .to_string();
            if !text.is_empty() {
                kind = classify_kind(&text).to_string();
                name = text;
            }
        }
    }

    DeviceBattery {
        id: format!("hidpp:{id_base}:{slot}"),
        name,
        kind,
        level,
        charging,
        connected: true,
        transport: TRANSPORT_24G.to_string(),
        source: "hidpp".to_string(),
        protocol_status: STATUS_OK.to_string(),
        updated_at: DeviceBattery::now_ms(),
    }
}

impl From<hidapi::HidError> for ProbeError {
    fn from(e: hidapi::HidError) -> Self {
        ProbeError::Io(e.to_string())
    }
}
