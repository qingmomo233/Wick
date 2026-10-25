//! HyperX 无线耳机电量协议（HP VID 0x03F0，Cloud Flight 为 Kingston VID 0x0951）。
//! 来源：HaloBattery + HeadsetControl + HyperHeadset（均真机验证）。各型号独立交换：
//!
//! - Cloud II Wireless（0696 / 018b）：usage page 0xFF90:0x0303，52 字节
//!   `06 ff bb <cmd>`，应答 20 字节回显前缀；cmd 02 电量（[7]=%）、03 充电（[4]==1）。
//! - Cloud III / II Core（05b7 / 0c9d / 0795 / 0995）：usage page 0xFF13:0x0001，
//!   62 字节 `66 <cmd>`（部分接收器只收 feature report，write 失败自动降级）；
//!   cmd 89 电量（应答 [1] 回显或 0x0D，[4]=%，[2][3] 全零视为无读数）、
//!   8a 充电（应答 [1] 回显或 0x0C，[2]：1 充电 2 充满）。
//! - Cloud III S（02cc / 06be）：64 字节输出报告 `0c 02 03 01 00 <cmd>`，
//!   应答 `0c .. <cmd> <value>`；cmd 06 电量、48 充电；0x0D 通知报告也会
//!   带电量（[4]==1 → [5]=%）与充电（[4]==10 → [5]=状态）。
//! - Cloud Alpha Wireless（098d）：31 字节 `21 bb <cmd>`；03 连接（[3]==1 离线）、
//!   0c 充电（[3]==1）、0b 电量（[3]=%）。
//! - Cloud Alpha 2（08be）：usage page 0xFF13:0xFF00，`50 02` → `51 02 <level> <flags>`，
//!   flags bit7 充电。
//! - Cloud Flight（0951:16c4 / 1723）：`21 ff 05` → 应答 [3..5] 大端电压 mV，
//!   电压超过 0x100B 视为充电（插线），否则按放电曲线折算百分比。

use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::model::{DeviceBattery, STATUS_OK, TRANSPORT_24G};
use crate::providers::hid::{HidDev, ProbeError};

const READ_TIMEOUT_MS: i32 = 1000;

/// 稳定设备 id：仅 VID/PID（底座可能枚举为多个 USB 功能，序列号不稳定）
pub fn device_id(interfaces: &[HidDev]) -> String {
    format!("hyperx:{:04x}-{:04x}", interfaces[0].vid, interfaces[0].pid)
}

pub fn probe(interfaces: &[HidDev]) -> Result<Option<DeviceBattery>, ProbeError> {
    let pid = interfaces[0].pid;
    let api = HidApi::new().map_err(|e| ProbeError::Io(e.to_string()))?;
    let reading = match pid {
        0x0696 | 0x018B => cloud2_probe(&api, interfaces)
            .map(|r| ("HyperX Cloud II Wireless", "headset", r)),
        0x05B7 | 0x0C9D => cloud3_probe(&api, interfaces)
            .map(|r| ("HyperX Cloud III Wireless", "headset", r)),
        0x0795 | 0x0995 => cloud3_probe(&api, interfaces)
            .map(|r| ("HyperX Cloud II Core Wireless", "headset", r)),
        0x02CC | 0x06BE => cloud3s_probe(&api, interfaces)
            .map(|r| ("HyperX Cloud III S Wireless", "headset", r)),
        0x098D => alpha_probe(&api, interfaces)
            .map(|r| ("HyperX Cloud Alpha Wireless", "headset", r)),
        0x08BE => alpha2_probe(&api, interfaces)
            .map(|r| ("HyperX Cloud Alpha 2", "headset", r)),
        // Kingston VID 只认 Cloud Flight 两个 PID，其余（内存等）不产生卡片
        0x16C4 | 0x1723 if interfaces[0].vid == 0x0951 => flight_probe(&api, interfaces)
            .map(|r| ("HyperX Cloud Flight Wireless", "headset", r)),
        _ => None,
    };
    Ok(reading.map(|(name, kind, (level, charging))| DeviceBattery {
        id: device_id(interfaces),
        name: name.to_string(),
        kind: kind.to_string(),
        level: Some(level),
        charging: Some(charging),
        connected: true,
        transport: TRANSPORT_24G.to_string(),
        source: "vendor-hid".to_string(),
        protocol_status: STATUS_OK.to_string(),
        updated_at: DeviceBattery::now_ms(),
    }))
}

/// 打开指定 usage 的集合（阻塞模式，配合 read_timeout 等待应答）
fn open_usage(api: &HidApi, interfaces: &[HidDev], page: u16, usage: u16) -> Option<hidapi::HidDevice> {
    let iface = interfaces
        .iter()
        .find(|d| d.usage_page == page && d.usage == usage)?;
    let path = std::ffi::CString::from_vec_with_nul(iface.path.clone()).ok()?;
    api.open_path(&path).ok()
}

fn read_reply(dev: &hidapi::HidDevice, _min_len: usize) -> Option<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_millis(READ_TIMEOUT_MS as u64);
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        match dev.read_timeout(&mut buf, 100) {
            Ok(n) if n > 0 => return Some(buf.to_vec()),
            Ok(_) | Err(_) => continue,
        }
    }
    None
}

// ---------- Cloud II Wireless ----------

fn cloud2_cmd(dev: &hidapi::HidDevice, cmd: u8) -> Option<Vec<u8>> {
    let mut req = [0u8; 52];
    req[0] = 0x06;
    req[1] = 0xFF;
    req[2] = 0xBB;
    req[3] = cmd;
    dev.write(&req).ok()?;
    std::thread::sleep(Duration::from_millis(100));
    let r = read_reply(dev, 20)?;
    (r.len() > 4 && r[0] == 0x06 && r[1] == 0xFF && r[2] == 0xBB && r[3] == cmd).then_some(r)
}

fn cloud2_probe(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    let dev = open_usage(api, interfaces, 0xFF90, 0x0303)?;
    let r = cloud2_cmd(&dev, 0x02)?;
    let level = *r.get(7)?;
    if level > 100 {
        return None;
    }
    let charging = cloud2_cmd(&dev, 0x03)
        .and_then(|r| r.get(4).copied())
        .map(|v| v == 1)
        .unwrap_or(false);
    if charging {
        return None; // 插线充电 = 有线
    }
    Some((level, false))
}

// ---------- Cloud III / Cloud II Core ----------

fn cloud3_cmd(dev: &hidapi::HidDevice, cmd: u8) -> Option<Vec<u8>> {
    let mut req = [0u8; 62];
    req[0] = 0x66;
    req[1] = cmd;
    if dev.write(&req).is_err() {
        // 部分接收器只收 feature report（Windows write 报 Incorrect function）
        dev.send_feature_report(&req).ok()?;
    }
    let r = read_reply(dev, 62)?;
    // [1] 为命令回显或对应应答 id
    let echo_ok = r.len() > 4 && r[0] == 0x66 && (r[1] == cmd || r[1] == cloud3_reply_id(cmd));
    echo_ok.then_some(r)
}

fn cloud3_reply_id(cmd: u8) -> u8 {
    match cmd {
        0x89 => 0x0D,
        0x8A => 0x0C,
        _ => 0,
    }
}

fn cloud3_probe(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    let dev = open_usage(api, interfaces, 0xFF13, 0x0001)?;
    let r = cloud3_cmd(&dev, 0x89)?;
    // 全零状态 = 尚无读数
    let level = *r.get(4)?;
    if level > 100 || (r.get(2) == Some(&0) && r.get(3) == Some(&0)) {
        return None;
    }
    let charging = matches!(
        cloud3_cmd(&dev, 0x8A).and_then(|r| r.get(2).copied()),
        Some(1) | Some(2)
    );
    if charging {
        return None; // 插线 / 充满 = 有线
    }
    Some((level, false))
}

// ---------- Cloud III S ----------

fn cloud3s_probe(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    // 请求发给第一个收得下 0x0C 输出报告的集合（厂商页优先）
    let mut candidates: Vec<&HidDev> = interfaces.iter().collect();
    candidates.sort_by_key(|d| if d.is_vendor_page() { 0 } else { 1 });
    for iface in candidates.iter().take(2) {
        let Ok(path) = std::ffi::CString::from_vec_with_nul(iface.path.clone()) else {
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            continue;
        };
        let mut req = [0u8; 64];
        req[0] = 0x0C;
        req[1] = 0x02;
        req[2] = 0x03;
        req[3] = 0x01; // 01 = 读值（00 为写值，绝不发送）
        req[5] = 0x06; // 电量
        if dev.write(&req).is_err() {
            continue;
        }
        // 应答是 0x0C 输入报告（[5]=命令 [6]=值），0x0D 通知（[4]==1 → [5]=%）也接受
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut buf = [0u8; 64];
        while Instant::now() < deadline {
            match dev.read_timeout(&mut buf, 100) {
                Ok(n) if n > 6 => {
                    if buf[0] == 0x0C && buf[5] == 0x06 {
                        let v = buf[6];
                        if (1..=100).contains(&v) {
                            return Some((v, false));
                        }
                        break;
                    }
                    if buf[0] == 0x0D && buf[4] == 1 {
                        let v = buf[5];
                        if (1..=100).contains(&v) {
                            return Some((v, false));
                        }
                    }
                }
                Ok(_) | Err(_) => continue,
            }
        }
    }
    None
}

// ---------- Cloud Alpha Wireless ----------

fn alpha_cmd(dev: &hidapi::HidDevice, cmd: u8) -> Option<Vec<u8>> {
    let mut req = [0u8; 31];
    req[0] = 0x21;
    req[1] = 0xBB;
    req[2] = cmd;
    dev.write(&req).ok()?;
    read_reply(dev, 4)
}

fn alpha_probe(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    // 无 usage 线索（HeadsetControl 未记录），取第一个厂商集合
    let iface = interfaces.iter().find(|d| d.is_vendor_page())?;
    let path = std::ffi::CString::from_vec_with_nul(iface.path.clone()).ok()?;
    let dev = api.open_path(&path).ok()?;
    // 连接检查：[3]==1 → 耳机离线
    if alpha_cmd(&dev, 0x03)?.get(3) == Some(&1) {
        return None;
    }
    // 充电检查：[3]==1 → 插线
    if alpha_cmd(&dev, 0x0C)?.get(3) == Some(&1) {
        return None;
    }
    let level = *alpha_cmd(&dev, 0x0B)?.get(3)?;
    if level > 100 {
        return None;
    }
    Some((level, false))
}

// ---------- Cloud Alpha 2 ----------

fn alpha2_probe(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    let dev = open_usage(api, interfaces, 0xFF13, 0xFF00)?;
    let mut req = [0u8; 64];
    req[0] = 0x50;
    req[1] = 0x02;
    dev.write(&req).ok()?;
    let r = read_reply(&dev, 8)?;
    // 应答 `51 02 <level> .. <flags>`；61 02 帧与电量无关，忽略
    if r.len() < 7 || r[0] != 0x51 || r[1] != 0x02 {
        return None;
    }
    let level = r[2];
    if level > 100 {
        return None;
    }
    if r[6] & 0x80 != 0 {
        return None; // 充电中 = 插线
    }
    Some((level, false))
}

// ---------- Cloud Flight ----------

/// 电压 → 百分比折算（HeadsetControl 参照 Logitech 放电曲线导出）
fn flight_estimate(voltage: u16) -> u8 {
    if voltage <= 3648 {
        return (voltage as u32 * 125 / 100000) as u8;
    }
    if voltage > 3975 {
        return 100;
    }
    let v = voltage as f64;
    let level = 0.00000002547505 * v.powi(4) - 0.0003900299 * v.powi(3) + 2.238321 * v.powi(2)
        - 5706.256 * v
        + 5452299.0;
    level.round().clamp(0.0, 100.0) as u8
}

fn flight_probe(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    let iface = interfaces.iter().find(|d| d.is_vendor_page())?;
    let path = std::ffi::CString::from_vec_with_nul(iface.path.clone()).ok()?;
    let dev = api.open_path(&path).ok()?;
    let mut req = [0u8; 20];
    req[0] = 0x21;
    req[1] = 0xFF;
    req[2] = 0x05;
    dev.write(&req).ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        match dev.read_timeout(&mut buf, 100) {
            Ok(n) if n == 15 || n == 20 => {
                let voltage = ((buf[3] as u16) << 8) | buf[4] as u16;
                if voltage > 0x100B {
                    return None; // 充电中 = 插线
                }
                if (3000..=4500).contains(&voltage) {
                    return Some((flight_estimate(voltage), false));
                }
            }
            Ok(_) | Err(_) => continue,
        }
    }
    None
}
