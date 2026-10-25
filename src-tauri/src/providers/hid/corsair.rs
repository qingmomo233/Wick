//! Corsair 无线耳机与鼠标电量协议（VID 0x1B1C）。三族交换：
//!
//! 1) 经典 Void 族（HeadsetControl 真机验证）：usage page 0xFFC5:0x0001 集合上
//!    写 [0xC9,0x64]，5 字节应答 [2]=电量（bit7 为麦克风标志），
//!    [4]=状态（0 离线 / 1·2 用电池 / 4·5 充电）。
//! 2) Void v2 / Virtuoso Max / HS80 Max（HaloBattery 移植 HeadSetControl）：
//!    65 字节帧 [0]=report id 0 [1]=0x02 [2]=endpoint（08 接收器 09 耳机）
//!    [3]=0x02 [4]=command；先「固件查询 + 双心跳」唤醒深睡耳机（无声弹出，
//!    不切软件模式），再电量命令 0x0F → 应答 [4..6] 小端 0..=1000，÷10 为百分比。
//! 3) bragi / SlipStream（Dark Core RGB Pro SE 接收器、Virtuoso SE/XT）：
//!    usage page 0xFF42:0x0001，get-property 帧 route 0x09（配对设备）、
//!    cmd 0x02、属性 0x0F 电量（十分之一百分比小端）/ 0x10 充电（1 充电 2 用电池）。
//!    应答晚到（实测可达 ~4.7s），需时间窗等待；两次读数一致才采信
//!    （iCUE / SignalRGB 共用通道时旧属性应答会被误读成电量）。
//!    Virtuoso 族只收 64 字节裸帧且带额外 0x02 前缀，Dark Core 收 65 字节 report-id-0 帧。

use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::model::{DeviceBattery, STATUS_OK, TRANSPORT_24G};
use crate::providers::hid::{HidDev, ProbeError};

const READ_TIMEOUT_MS: i32 = 500;
/// bragi 首问等待窗（实测应答最慢 ~4.7s）
const BRAGI_WINDOW: Duration = Duration::from_secs(5);
const BRAGI_CONFIRM_WINDOW: Duration = Duration::from_millis(2500);

/// 稳定设备 id：仅 VID/PID（接收器与充电线共用产品名，靠 PID 区分已足够）
pub fn device_id(interfaces: &[HidDev]) -> String {
    format!("corsair:{:04x}-{:04x}", interfaces[0].vid, interfaces[0].pid)
}

pub fn probe(interfaces: &[HidDev]) -> Result<Option<DeviceBattery>, ProbeError> {
    let pid = interfaces[0].pid;
    let api = HidApi::new().map_err(|e| ProbeError::Io(e.to_string()))?;
    let reading = match pid {
        // Void v2 / Virtuoso Max / HS80 Max
        0x2A08 | 0x2A02 | 0x0A97 => v2_probe(&api, interfaces)
            .map(|r| (v2_name(pid).to_string(), "headset", r)),
        // bragi：Dark Core 鼠标 + Virtuoso SE/XT 耳机（只读接收器，有线 PID 不列）
        0x1B7F => bragi_probe(&api, interfaces, false)
            .map(|r| ("Corsair Dark Core RGB Pro SE".to_string(), "mouse", r)),
        0x0A40 | 0x0A3E => bragi_probe(&api, interfaces, true)
            .map(|r| ("Corsair Virtuoso RGB Wireless SE".to_string(), "headset", r)),
        0x0A64 => bragi_probe(&api, interfaces, true)
            .map(|r| ("Corsair Virtuoso RGB Wireless XT".to_string(), "headset", r)),
        // 经典 Void 族（HeadsetControl 列表；接收器名即产品字符串）
        _ if is_legacy_void_pid(pid) => legacy_probe(&api, interfaces)
            .map(|r| (fallback_name(interfaces, "Corsair Void"), "headset", r)),
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

fn v2_name(pid: u16) -> &'static str {
    match pid {
        0x2A08 => "Corsair Void v2 Wireless",
        0x2A02 => "Corsair Virtuoso Max Wireless",
        _ => "Corsair HS80 Max Wireless",
    }
}

fn fallback_name(interfaces: &[HidDev], fallback: &str) -> String {
    interfaces[0].display_name(fallback)
}

/// HeadsetControl corsair_void 设备的 PID 列表（Void / Void Pro / Elite / HS 系列）
fn is_legacy_void_pid(pid: u16) -> bool {
    matches!(
        pid,
        0x1B27 | 0x0A14 | 0x0A16 | 0x0A17 | 0x0A1D | 0x0A1A | 0x1B2A | 0x1B23 | 0x1B29 | 0x0A55
            | 0x0A51 | 0x0A52 | 0x0A38 | 0x0A4F | 0x0A2B | 0x0A75 | 0x0A56
    )
}

// ---------- 经典 Void 族 ----------

fn legacy_probe(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    let iface = interfaces
        .iter()
        .find(|d| d.usage_page == 0xFFC5 && d.usage == 0x0001)?;
    let path = std::ffi::CString::from_vec_with_nul(iface.path.clone()).ok()?;
    let dev = api.open_path(&path).ok()?;
    dev.write(&[0xC9, 0x64]).ok()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        match dev.read_timeout(&mut buf, 100) {
            Ok(n) if n >= 5 => {
                let status = buf[4];
                if status == 0 {
                    return None; // 耳机未连接
                }
                let charging = status == 4 || status == 5;
                if charging {
                    return None; // 插线充电 = 有线
                }
                let level = buf[2] & 0x7F; // bit7 是麦克风抬起标志
                if level <= 100 {
                    return Some((level, false));
                }
                return None;
            }
            Ok(_) | Err(_) => continue,
        }
    }
    None
}

// ---------- Void v2 族 ----------

fn v2_write(dev: &hidapi::HidDevice, endpoint: u8, sub: u8, command: u8) -> bool {
    let mut frame = [0u8; 65];
    frame[1] = 0x02;
    frame[2] = endpoint;
    frame[3] = sub;
    frame[4] = command;
    dev.write(&frame).is_ok()
}

fn v2_probe(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    // 无 usage page 线索，优先 interface 4（HeadsetControl 实测），再试其余厂商集合
    let mut candidates: Vec<&HidDev> = interfaces.iter().collect();
    candidates.sort_by_key(|d| (d.interface != 4, if d.interface < 0 { 99 } else { d.interface }));
    candidates.truncate(3);
    for iface in candidates {
        let Ok(path) = std::ffi::CString::from_vec_with_nul(iface.path.clone()) else {
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            continue;
        };
        // 唤醒深睡耳机的最小握手（固件查询 + 接收器/耳机心跳）
        v2_write(&dev, 0x08, 0x02, 0x13);
        v2_write(&dev, 0x08, 0x02, 0x12);
        let mut buf = [0u8; 64];
        for _ in 0..4 {
            if dev.read_timeout(&mut buf, 30).is_err() {
                break;
            }
        }
        v2_write(&dev, 0x09, 0x02, 0x12);
        if dev.read_timeout(&mut buf, READ_TIMEOUT_MS).map(|n| n == 0).unwrap_or(true) {
            continue; // 耳机未应答（关机 / 深睡 / 非目标集合）
        }
        for _ in 0..4 {
            if dev.read_timeout(&mut buf, 30).is_err() {
                break;
            }
        }
        // 电量查询 ×3：接收器有时回配对 id 而非电量，读到合法值为止
        for _ in 0..3 {
            v2_write(&dev, 0x09, 0x02, 0x0F);
            if let Ok(n) = dev.read_timeout(&mut buf, READ_TIMEOUT_MS) {
                if n > 5 {
                    let vendor = (buf[4] as u16) | ((buf[5] as u16) << 8);
                    if (1..=1000).contains(&vendor) {
                        return Some(((vendor / 10) as u8, false));
                    }
                }
            }
        }
    }
    None
}

// ---------- bragi / SlipStream ----------

const BRAGI_PAGE: u16 = 0xFF42;
const ROUTE_DONGLE: u8 = 0x08;
const ROUTE_DEVICE: u8 = 0x09;
const ROUTE_CHILD: u8 = 0x01; // 应答回程路线
const CMD_GET: u8 = 0x02;
const PROP_BATTERY: u8 = 0x0F;
const PROP_CHARGE: u8 = 0x10;
const PROP_HEARTBEAT: u8 = 0x12;
const PROP_FIRMWARE: u8 = 0x13;

/// get-property 帧。`bare`（Virtuoso 族）：64 字节裸帧带 0x02 前缀；
/// 否则（Dark Core）：65 字节 report-id-0 帧，裸帧为回退。
fn bragi_send(dev: &hidapi::HidDevice, prop: u8, route: u8, bare: bool) -> bool {
    if bare {
        let mut frame = [0u8; 64];
        frame[0] = 0x02;
        frame[1] = route;
        frame[2] = CMD_GET;
        frame[3] = prop;
        return dev.write(&frame).is_ok();
    }
    let mut frame = [0u8; 65];
    frame[1] = route;
    frame[2] = CMD_GET;
    frame[3] = prop;
    if dev.write(&frame).is_ok() {
        return true;
    }
    dev.write(&frame[1..]).is_ok()
}

/// get-property 应答的原始 16 位值；两种形态都回程路线 0x01：
/// Dark Core `01 02 00 <v LE>`、Virtuoso `01 01 02 00 <v LE>`（多一字节来源）。
/// 字段不全吻合（设备列表 / 通知 / 心跳帧）一律不采信。
fn bragi_parse_raw(reply: &[u8]) -> Option<u16> {
    let data = if reply.len() > 64 {
        &reply[1..]
    } else {
        reply
    };
    if data.len() < 5 || data[0] != ROUTE_CHILD {
        return None;
    }
    if data[1] == CMD_GET && data[2] == 0x00 {
        return Some((data[3] as u16) | ((data[4] as u16) << 8));
    }
    if data.len() >= 6 && data[1] == ROUTE_CHILD && data[2] == CMD_GET && data[3] == 0x00 {
        return Some((data[4] as u16) | ((data[5] as u16) << 8));
    }
    None
}

fn bragi_read(
    dev: &hidapi::HidDevice,
    window: Duration,
    accept: fn(u16) -> bool,
) -> Option<u16> {
    let deadline = Instant::now() + window;
    let mut buf = [0u8; 65];
    while Instant::now() < deadline {
        match dev.read_timeout(&mut buf, READ_TIMEOUT_MS) {
            Ok(n) if n > 0 => {
                if let Some(v) = bragi_parse_raw(&buf[..n]) {
                    if accept(v) {
                        return Some(v);
                    }
                }
            }
            Ok(_) | Err(_) => continue,
        }
    }
    None
}

fn bragi_probe(api: &HidApi, interfaces: &[HidDev], bare: bool) -> Option<(u8, bool)> {
    let mut candidates: Vec<&HidDev> = interfaces
        .iter()
        .filter(|d| d.usage_page == BRAGI_PAGE)
        .collect();
    if candidates.is_empty() {
        candidates = interfaces.iter().collect();
    }
    candidates.sort_by_key(|d| (d.usage != 0x0001, if d.interface < 0 { 99 } else { d.interface }));
    for iface in candidates.iter().take(2) {
        let Ok(path) = std::ffi::CString::from_vec_with_nul(iface.path.clone()) else {
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            continue;
        };
        let mut buf = [0u8; 65];
        for _ in 0..4 {
            if dev.read_timeout(&mut buf, 30).is_err() {
                break;
            }
        }
        // Virtuoso SE 需要先跑一遍会话（固件查询 + 双心跳）才应答
        if bare {
            for (prop, route) in [
                (PROP_FIRMWARE, ROUTE_DONGLE),
                (PROP_HEARTBEAT, ROUTE_DONGLE),
                (PROP_HEARTBEAT, ROUTE_DEVICE),
            ] {
                if !bragi_send(&dev, prop, route, bare) {
                    break;
                }
                let _ = bragi_read(&dev, Duration::from_millis(600), |_| false);
            }
        }
        if !bragi_send(&dev, PROP_BATTERY, ROUTE_DEVICE, bare) {
            continue; // 该集合发不进去 → 试下一个
        }
        let valid = |v: u16| (1..=1000).contains(&v);
        let Some(first) = bragi_read(&dev, BRAGI_WINDOW, valid) else {
            // 发送被接受但整窗无应答：协议端点已定位，设备关机 / 深睡，
            // 不再试其他集合（避免每轮多花一个读窗）
            break;
        };
        // 应答不回显属性号，别的软件可能把其他属性的应答顶上来 → 复问一次须一致
        let confirmed = if bragi_send(&dev, PROP_BATTERY, ROUTE_DEVICE, bare) {
            match bragi_read(&dev, BRAGI_CONFIRM_WINDOW, valid) {
                Some(second) => second == first,
                None => true, // 复问静默：采信单次读数
            }
        } else {
            true
        };
        if !confirmed {
            continue;
        }
        let level = (first / 10) as u8;
        if level > 100 {
            continue;
        }
        // Virtuoso 族可再问充电属性（1 充电 2 用电池）；充电 = 插线，超出无线范围
        if bare
            && bragi_send(&dev, PROP_CHARGE, ROUTE_DEVICE, bare)
            && bragi_read(&dev, Duration::from_secs(1), |v| v == 1 || v == 2) == Some(1)
        {
            return None;
        }
        return Some((level, false));
    }
    None
}
