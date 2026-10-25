//! Razer 无线设备电量协议（VID 0x1532）。
//! 来源：HaloBattery providers/razer.py（Windows 真机验证）+ OpenRazer 驱动，
//! 三套协议并存，按 PID 分派：
//!
//! 1) 主协议（绝大多数无线键鼠 / 接收器）：90 字节 feature report（report id 0），
//!    与 Synapse / OpenRazer 同源。请求 [1]=transaction id、[5]=0x02、
//!    [6]=command class（0x07 电源）、[7]=command id（0x80 电量 0..255、
//!    0x84 充电），[88] = 字节 2..87 的 XOR；应答 [0]=status（0x02 OK、
//!    0x01 busy、0x04 设备未应答、0x05 不支持）、[6][7] 回显命令、[9]=值。
//! 2) BlackShark V2 Pro 2020（0x0528）：0xFF00 集合上 64 字节 feature report
//!    （report id 0xFF，OpenRazer issue #1280 抓包）。
//! 3) BlackShark V2 Pro 2023 / Barracuda Pro（0x0555/0x0556/0x0557/0x053A）：
//!    "PA" 输出报告帧（report id 0x02 / 0x01），需先发 remote-on 唤醒链路。
//!
//! 有线过滤：插线充电的键鼠带标准键盘接口（Usage Page 0x07），跳过；
//! 已知的有线 PID（如 052E）也不探测。

use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::model::{classify_kind, DeviceBattery, STATUS_OK, TRANSPORT_24G};
use crate::providers::hid::{HidDev, ProbeError};

// ---------- 主协议常量 ----------

const REPORT_LEN: usize = 90;
const CLASS_POWER: u8 = 0x07;
const CMD_BATTERY: u8 = 0x80;
const CMD_CHARGING: u8 = 0x84;
const STATUS_OK_BYTE: u8 = 0x02;
/// 设备未应答（关机 / 深睡）——接收器在场，是正常状态而非错误
const STATUS_NO_REPLY: u8 = 0x04;

/// transaction id 尝试顺序：已知 PID 用表内首选，未知设备按此顺序扫描
const TRANSACTION_IDS: [u8; 5] = [0x1F, 0x3F, 0xFF, 0x9F, 0x08];
/// 单条命令的应答等待窗口
const QUERY_WINDOW: Duration = Duration::from_millis(1000);
/// 同一请求最多补发次数（应答被 Synapse 等 RGB 软件的报文顶掉时）
const RESENDS: usize = 3;

// ---------- PA 协议常量（BlackShark 2023 / Barracuda）----------

const PA_CMD_BATTERY: u8 = 0x21;
const PA_CMD_CHARGING: u8 = 0x2A;
const PA_REPLY_TIMEOUT: Duration = Duration::from_millis(150);
const PA_ATTEMPTS: usize = 3;

// ---------- BlackShark 2020 常量 ----------

const BS2020_REQUEST_PREFIX: [u8; 9] = [0xFF, 0x0A, 0x00, 0xFD, 0x04, 0x12, 0xF1, 0x02, 0x05];
const BS2020_REPLY_PREFIX: [u8; 9] = [0xFF, 0x0F, 0x05, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05];
/// 无新应答时接收器保留的旧载荷头（电池字节是过期值，不可用）
const BS2020_STALE_PREFIX: [u8; 9] = [0xFF, 0x01, 0x00, 0xFE, 0x12, 0x04, 0x1F, 0x08, 0x05];

/// 已知无线设备表：PID → (名称, 类型, 首选 transaction id)。
/// 名称与 tid 来自 OpenRazer 的 charge_level 驱动表（HaloBattery 交叉核对）。
fn known(pid: u16) -> Option<(&'static str, &'static str, u8)> {
    Some(match pid {
        0x0528 => ("Razer BlackShark V2 Pro (2020)", "headset", 0x3F),
        0x0555..=0x0557 => ("Razer BlackShark V2 Pro (2023)", "headset", 0x3F),
        0x053A => ("Razer Barracuda Pro", "headset", 0x3F),
        0x00A4 => ("Razer Mouse Dock Pro", "mouse", 0x1F),
        // 无线键盘（HyperSpeed 接收器）
        0x0290 | 0x0292 => ("Razer DeathStalker V2 Pro", "keyboard", 0x9F),
        0x0296 | 0x0298 => ("Razer DeathStalker V2 Pro TKL", "keyboard", 0x9F),
        0x0271 | 0x0258 => ("Razer BlackWidow V3 Mini HyperSpeed", "keyboard", 0x9F),
        0x02BA | 0x02B9 => ("Razer BlackWidow V4 Mini HyperSpeed", "keyboard", 0x9F),
        0x02D5 | 0x02D7 => ("Razer BlackWidow V4 Tenkeyless HyperSpeed", "keyboard", 0x9F),
        0x025A | 0x025C => ("Razer BlackWidow V3 Pro", "keyboard", 0x9F),
        // 无线鼠标
        0x001F => ("Razer Naga Epic", "mouse", 0xFF),
        0x0024 | 0x0025 => ("Razer Mamba (2012)", "mouse", 0xFF),
        0x0032 => ("Razer Ouroboros", "mouse", 0xFF),
        0x003E | 0x003F => ("Razer Naga Epic Chroma", "mouse", 0xFF),
        0x0044 | 0x0045 => ("Razer Mamba", "mouse", 0xFF),
        0x0059 | 0x005A => ("Razer Lancehead", "mouse", 0x3F),
        0x0062 => ("Razer Atheris", "mouse", 0x1F),
        0x006F | 0x0070 => ("Razer Lancehead Wireless", "mouse", 0x1F),
        0x0072 | 0x0073 => ("Razer Mamba Wireless", "mouse", 0x3F),
        0x0077 | 0x0080 => ("Razer Pro Click", "mouse", 0x1F),
        0x007A | 0x007B => ("Razer Viper Ultimate", "mouse", 0xFF),
        0x007C | 0x007D => ("Razer DeathAdder V2 Pro", "mouse", 0x3F),
        0x0083 => ("Razer Basilisk X HyperSpeed", "mouse", 0xFF),
        0x0086 | 0x0088 => ("Razer Basilisk Ultimate", "mouse", 0x1F),
        0x008F | 0x0090 => ("Razer Naga Pro", "mouse", 0x1F),
        0x0094 => ("Razer Orochi V2", "mouse", 0x1F),
        0x009A => ("Razer Pro Click Mini", "mouse", 0x1F),
        0x009C => ("Razer DeathAdder V2 X HyperSpeed", "mouse", 0x1F),
        0x009E | 0x009F => ("Razer Viper Mini SE", "mouse", 0x1F),
        0x00A5 | 0x00A6 => ("Razer Viper V2 Pro", "mouse", 0x1F),
        0x00A7 | 0x00A8 => ("Razer Naga V2 Pro", "mouse", 0x1F),
        0x00AA | 0x00AB => ("Razer Basilisk V3 Pro", "mouse", 0x1F),
        0x00AF | 0x00B0 => ("Razer Cobra Pro", "mouse", 0x1F),
        0x00B3 => ("Razer HyperPolling Dongle", "mouse", 0x1F),
        0x00B4 => ("Razer Naga V2 HyperSpeed", "mouse", 0x1F),
        0x00B6 | 0x00B7 => ("Razer DeathAdder V3 Pro", "mouse", 0x1F),
        0x00B8 => ("Razer Viper V3 HyperSpeed", "mouse", 0x1F),
        0x00B9 => ("Razer Basilisk V3 X HyperSpeed", "mouse", 0x1F),
        0x00BE | 0x00BF => ("Razer DeathAdder V4 Pro", "mouse", 0x1F),
        0x00C0 | 0x00C1 => ("Razer Viper V3 Pro", "mouse", 0x1F),
        0x00C2 | 0x00C3 => ("Razer DeathAdder V3 Pro", "mouse", 0x1F),
        0x00C4 | 0x00C5 => ("Razer DeathAdder V3 HyperSpeed", "mouse", 0x1F),
        0x00C7 | 0x00C8 => ("Razer Pro Click V2 Vertical Edition", "mouse", 0x1F),
        0x00CC | 0x00CD => ("Razer Basilisk V3 Pro 35K", "mouse", 0x1F),
        0x00D0 | 0x00D1 => ("Razer Pro Click V2", "mouse", 0x1F),
        0x00D3 | 0x00D4 => ("Razer Basilisk Mobile", "mouse", 0x1F),
        0x00D6 | 0x00D7 => ("Razer Basilisk V3 Pro 35K Phantom Green", "mouse", 0x1F),
        _ => return None,
    })
}

/// 未登记 PID 仅在名称暗示无线时探测（有线设备也会应答电量命令，但读数无意义）
const WIRELESS_WORDS: [&str; 8] = [
    "hyperspeed", "wireless", "receiver", "dongle", "dock", "blackshark", "barracuda", "nari",
];

/// 稳定设备 id：仅 VID/PID，不含序列号——Synapse 会把部分集合重挂载为
/// 空序列号的虚拟设备，按序列号拆组会让同一台设备出现两张卡片。
pub fn device_id(interfaces: &[HidDev]) -> String {
    format!("razer:{:04x}-{:04x}", interfaces[0].vid, interfaces[0].pid)
}

/// 逐 PID 分派到三套协议之一。返回 None 表示本轮无读数（设备深睡 / 关机）。
pub fn probe(interfaces: &[HidDev]) -> Result<Option<DeviceBattery>, ProbeError> {
    let pid = interfaces[0].pid;
    // 有线过滤：插线状态的键鼠暴露标准键盘接口
    if interfaces.iter().any(|d| d.usage_page == 0x07) {
        return Ok(None);
    }
    let (name, kind, tid) = match known(pid) {
        Some((n, k, t)) => (n.to_string(), k.to_string(), Some(t)),
        None => {
            // 已知有线 PID 直接跳过（052E = BlackShark 2020 充电线，电量从接收器 0528 读）
            if pid == 0x052E {
                return Ok(None);
            }
            let product = format!(
                "{} {}",
                interfaces[0].product,
                interfaces[0].manufacturer
            )
            .to_lowercase();
            if !WIRELESS_WORDS.iter().any(|w| product.contains(w)) {
                return Ok(None);
            }
            (interfaces[0].display_name("Razer"), String::new(), None)
        }
    };
    let kind = if kind.is_empty() {
        classify_kind(&name).to_string()
    } else {
        kind
    };

    let api = HidApi::new().map_err(|e| ProbeError::Io(e.to_string()))?;
    let reading = match pid {
        0x0528 => probe_bs2020(&api, interfaces),
        0x0555..=0x0557 => {
            probe_pa(&api, interfaces, 0x02, 12).or_else(|| probe_core(&api, interfaces, tid))
        }
        0x053A => probe_pa(&api, interfaces, 0x01, 13),
        _ => probe_core(&api, interfaces, tid),
    };
    Ok(reading.map(|(level, charging)| DeviceBattery {
        id: device_id(interfaces),
        name,
        kind,
        level: Some(level),
        charging: Some(charging),
        connected: true,
        transport: TRANSPORT_24G.to_string(),
        source: "vendor-hid".to_string(),
        protocol_status: STATUS_OK.to_string(),
        updated_at: DeviceBattery::now_ms(),
    }))
}

// ---------- 主协议 ----------

/// 接口探测顺序：厂商页 / 通用桌面页优先，Windows 重挂载的集合（interface -1）垫底
fn rank_interfaces(interfaces: &[HidDev]) -> Vec<&HidDev> {
    let mut sorted: Vec<&HidDev> = interfaces.iter().collect();
    sorted.sort_by_key(|d| {
        let page_rank = if d.usage_page == 0x0001 || d.usage_page == 0xFF00 {
            0
        } else {
            1
        };
        (page_rank, if d.interface < 0 { 99 } else { d.interface })
    });
    sorted
}

fn probe_core(api: &HidApi, interfaces: &[HidDev], pref_tid: Option<u8>) -> Option<(u8, bool)> {
    // 未知 PID（无首选 tid）：短窗口扫描 + 最多 2 个接口，控制整组耗时
    let scan = pref_tid.is_none();
    let window = if scan {
        Duration::from_millis(350)
    } else {
        QUERY_WINDOW
    };
    let mut ranked = rank_interfaces(interfaces);
    if scan {
        ranked.truncate(2);
    }
    for iface in ranked {
        let Ok(path) = std::ffi::CString::from_vec_with_nul(iface.path.clone()) else {
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            continue;
        };
        // 主协议只用 feature report（GET/SET_REPORT），与阻塞模式无关
        let mut tids: Vec<u8> = Vec::new();
        if let Some(t) = pref_tid {
            tids.push(t);
        }
        for t in TRANSACTION_IDS {
            if !tids.contains(&t) {
                tids.push(t);
            }
        }
        let mut no_reply = false;
        for tid in tids {
            match query_power(&dev, tid, CMD_BATTERY, window) {
                // 该接口不收 feature report（send 失败）→ 换接口
                QueryOutcome::Unreachable => break,
                QueryOutcome::Value(raw) => {
                    let level = ((raw as u16 * 100 + 127) / 255) as u8;
                    let charging = matches!(
                        query_power(&dev, tid, CMD_CHARGING, window),
                        QueryOutcome::Value(v) if v != 0
                    );
                    return Some((level.min(100), charging));
                }
                // 设备关机 / 深睡：协议端口已找到，不再换 tid / 接口
                QueryOutcome::NoReply => {
                    no_reply = true;
                    break;
                }
                QueryOutcome::Foreign | QueryOutcome::Busy => continue,
            }
        }
        if no_reply {
            return None;
        }
    }
    None
}

enum QueryOutcome {
    /// 接口拒绝 feature report（非协议集合）
    Unreachable,
    /// 设备未应答（关机 / 深睡，status 0x04）或不支持（0x05）
    NoReply,
    /// status OK 但应答属于其他命令（Synapse 等 RGB 软件共用此集合）
    Foreign,
    /// 设备忙（0x01）到超时
    Busy,
    Value(u8),
}

fn query_power(dev: &hidapi::HidDevice, tid: u8, cmd: u8, window: Duration) -> QueryOutcome {
    let mut req = [0u8; REPORT_LEN];
    req[1] = tid;
    req[5] = 0x02;
    req[6] = CLASS_POWER;
    req[7] = cmd;
    let mut crc = 0u8;
    for &b in &req[2..88] {
        crc ^= b;
    }
    req[88] = crc;

    let send = |r: &[u8]| -> bool {
        let mut packet = Vec::with_capacity(1 + r.len());
        packet.push(0x00); // report id
        packet.extend_from_slice(r);
        dev.send_feature_report(&packet).is_ok()
    };
    if !send(&req) {
        return QueryOutcome::Unreachable;
    }

    let deadline = Instant::now() + window;
    let mut sends = 1usize;
    let mut foreign = 0usize;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(60));
        let mut buf = [0u8; REPORT_LEN + 1];
        buf[0] = 0x00;
        let Ok(n) = dev.get_feature_report(&mut buf) else {
            return QueryOutcome::Unreachable;
        };
        // Windows 应答带 report id 前缀（91 字节），Linux 不带（90 字节）
        let data: &[u8] = if n > REPORT_LEN {
            &buf[1..=REPORT_LEN]
        } else if n == REPORT_LEN {
            &buf[..REPORT_LEN]
        } else {
            continue;
        };
        let status = data[0];
        if data[6] == CLASS_POWER && data[7] == cmd {
            return match status {
                STATUS_OK_BYTE => QueryOutcome::Value(data[9]),
                STATUS_NO_REPLY | 0x05 => QueryOutcome::NoReply,
                _ => QueryOutcome::Busy,
            };
        }
        if status == STATUS_OK_BYTE {
            // 应答属于别的命令（RGB 软件的高频帧），隔几轮补发一次
            foreign += 1;
            if foreign % 4 == 0 && sends < RESENDS {
                if !send(&req) {
                    return QueryOutcome::Unreachable;
                }
                sends += 1;
            }
        }
    }
    if foreign > 0 {
        QueryOutcome::Foreign
    } else {
        QueryOutcome::Busy
    }
}

// ---------- PA 协议（BlackShark 2023 / Barracuda Pro）----------

/// `report_id`/`echo_off` 区分两代帧布局：BlackShark 2023 用 report id 0x02、
/// 应答命令回显在 [12]；Barracuda 用 0x01、回显在 [13] 且 'P','A' 前移两字节。
fn probe_pa(api: &HidApi, interfaces: &[HidDev], report_id: u8, echo_off: usize) -> Option<(u8, bool)> {
    let mut candidates: Vec<&HidDev> = interfaces.iter().collect();
    candidates.sort_by_key(|d| if d.is_vendor_page() { 0 } else { 1 });
    for iface in candidates.iter().take(2) {
        let Ok(path) = std::ffi::CString::from_vec_with_nul(iface.path.clone()) else {
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            continue;
        };
        let _ = dev.set_blocking_mode(false);
        let Some(level) = pa_query(&dev, report_id, echo_off, PA_CMD_BATTERY) else {
            continue;
        };
        if level > 100 {
            continue;
        }
        let charging = matches!(pa_query(&dev, report_id, echo_off, PA_CMD_CHARGING), Some(v) if v != 0);
        return Some((level.min(100), charging));
    }
    None
}

/// 单命令 PA 查询：先发一帧 remote-on 唤醒打盹的射频链路，再正式查询。
/// 返回 Some(payload)——电量命令的 payload 是百分比，充电命令是 0/1。
fn pa_query(dev: &hidapi::HidDevice, report_id: u8, echo_off: usize, cmd: u8) -> Option<u8> {
    let remote = |on: bool| -> bool {
        let mut f = [0u8; 64];
        f[0] = report_id;
        f[1] = 0x80;
        f[2] = 0x07;
        if report_id == 0x02 {
            f[5] = 0x50; // 'P'
            f[6] = 0x41; // 'A'
            f[7] = 0x0E;
            f[9] = 0x02; // remote 类型
            f[10] = 0xE1;
            f[11] = u8::from(on);
        } else {
            f[3] = 0x50;
            f[4] = 0x41;
            f[5] = 0x0E;
            f[6] = 0x08;
            f[7] = 0x02;
            f[8] = 0xE1;
            f[9] = u8::from(on);
        }
        dev.write(&f).is_ok()
    };
    let query = |c: u8| -> bool {
        let mut f = [0u8; 64];
        f[0] = report_id;
        f[1] = 0x80;
        f[2] = 0x08;
        if report_id == 0x02 {
            f[5] = 0x50;
            f[6] = 0x41;
            f[7] = 0x08;
            f[9] = 0x03; // query 类型
            f[10] = c;
        } else {
            f[3] = 0x50;
            f[4] = 0x41;
            f[5] = 0x08;
            f[6] = 0x08;
            f[7] = 0x03;
            f[8] = c;
        }
        dev.write(&f).is_ok()
    };

    // 唤醒帧：remote-on 被接受说明该集合说 PA 协议
    drain(dev);
    if !remote(true) {
        return None;
    }
    std::thread::sleep(Duration::from_millis(35));

    let mut result: Option<u8> = None;
    for _ in 0..PA_ATTEMPTS {
        remote(true);
        if !query(cmd) {
            break;
        }
        let deadline = Instant::now() + PA_REPLY_TIMEOUT;
        while Instant::now() < deadline {
            let mut buf = [0u8; 64];
            match dev.read(&mut buf) {
                Ok(n) if n > 0 => {
                    // 应答可能带或不带 report id 前缀
                    let off = if buf[0] == report_id { 0 } else { 1 };
                    let c = echo_off + off;
                    if c + 3 < n && buf[c] == cmd && buf[c + 1] == 0x01 {
                        result = Some(buf[c + 3]);
                        break;
                    }
                }
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        if result.is_some() {
            break;
        }
    }
    let _ = remote(false);
    result
}

// ---------- BlackShark V2 Pro 2020 ----------

/// 布局（真机直读验证）：应答前缀后 [11]=电源状态、[12..13]=电压 mV 大端、
/// [14]=档位电量。充电中（状态 0x09）档位不可靠，只报充电不报电量；
/// 0x64/0x32/... 档位由固件给出，10% 一档。
fn probe_bs2020(api: &HidApi, interfaces: &[HidDev]) -> Option<(u8, bool)> {
    for iface in interfaces.iter().filter(|d| d.usage_page == 0xFF00) {
        let Ok(path) = std::ffi::CString::from_vec_with_nul(iface.path.clone()) else {
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            continue;
        };
        let mut req = [0u8; 64];
        req[..BS2020_REQUEST_PREFIX.len()].copy_from_slice(&BS2020_REQUEST_PREFIX);
        if dev.send_feature_report(&req).is_err() {
            continue;
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
            let mut buf = [0u8; 64];
            buf[0] = 0xFF;
            let Ok(n) = dev.get_feature_report(&mut buf) else {
                return None;
            };
            if n == 64 && buf[..9] == BS2020_REPLY_PREFIX {
                let state = buf[11];
                let millivolts = ((buf[12] as u16) << 8) | buf[13] as u16;
                if millivolts == 0 {
                    return None; // 耳机关机 / 超范围
                }
                let charging = state == 0x09;
                if charging {
                    return None; // 插线充电 = 有线，超出监控范围
                }
                let level = buf[14];
                if level <= 100 {
                    return Some((level, false));
                }
                return None;
            }
            if n == 64 && buf[..9] == BS2020_STALE_PREFIX {
                return None; // 无新应答（耳机关机）
            }
        }
    }
    None
}

fn drain(dev: &hidapi::HidDevice) {
    let mut buf = [0u8; 64];
    for _ in 0..32 {
        match dev.read(&mut buf) {
            Ok(n) if n > 0 => continue,
            _ => break,
        }
    }
}
