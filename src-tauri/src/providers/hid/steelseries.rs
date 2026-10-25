//! SteelSeries 无线耳机与鼠标电量协议（VID 0x1038）。
//! 来源：HaloBattery providers/steelseries.py（含真机验证）+ HeadsetControl +
//! Linux 驱动 hid-steelseries-arctis.c 的回显校验。三族交换：
//!
//! 1) Nova 系耳机（接收器 usage page 0xFFC0:0x0001）：写 [0x00,0xB0]，
//!    应答以 0xB0 开头，按 PID 家族解析（nova7 / nova5 / 7+ / GameBuds）。
//! 2) 无线鼠标（同一 0xFFC0 集合）：Rival 3 族用 [0x00,0xAA,0x01]（应答回显
//!    0xAA）；Aerox 3/5/9 族用接收器形态的电量查询 [0x00,0xD2]（应答回显
//!    0xD2，档位字节 bit7 为充电标志，1..=21 档或 >21 直读百分比）。
//!    链路随鼠标闲置休眠，应答只在唤醒后到达 → 本轮无应答即无读数。
//! 3) 经典 Arctis（1 / 7 / 9 / Pro Wireless / Nova Pro 底座）：各型号独立的
//!    请求与应答布局，只有回显或文档值吻合的应答才被采信。

use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::model::{DeviceBattery, STATUS_OK, TRANSPORT_24G};
use crate::providers::hid::{HidDev, ProbeError};

/// 配置集合（Nova / 鼠标家族）：usage page 0xFFC0 / usage 0x0001
const VENDOR_PAGE: u16 = 0xFFC0;
const VENDOR_USAGE: u16 = 0x0001;
const TIMEOUT: Duration = Duration::from_millis(1000);
const READ_SLICE: Duration = Duration::from_millis(100);

/// 稳定设备 id：仅 VID/PID（同一接收器的多个集合可能序列号不一致，拆组会导致重复卡片）
pub fn device_id(interfaces: &[HidDev]) -> String {
    format!("steelseries:{:04x}-{:04x}", interfaces[0].vid, interfaces[0].pid)
}

pub fn probe(interfaces: &[HidDev]) -> Result<Option<DeviceBattery>, ProbeError> {
    let pid = interfaces[0].pid;
    let api = HidApi::new().map_err(|e| ProbeError::Io(e.to_string()))?;
    let reading = match pid {
        // ---- Nova 系耳机（0xB0 交换）----
        0x22A1 | 0x2202 | 0x227E | 0x2206 | 0x2258 | 0x229E | 0x22AD | 0x22A4 | 0x22A5
        | 0x223A | 0x22A9 | 0x227A | 0x220A | 0x22A7 | 0x2298 => {
            nova_probe(&api, interfaces, parse_nova7(pid_discrete(pid)))
                .map(|r| (nova_name(pid).unwrap_or("SteelSeries Arctis"), "headset", r))
        }
        0x2232 | 0x2253 | 0x2269 | 0x226D => {
            nova_probe(&api, interfaces, parse_nova5)
                .map(|r| (nova_name(pid).unwrap_or("SteelSeries Arctis"), "headset", r))
        }
        0x220E | 0x2212 | 0x2216 | 0x2236 => {
            nova_probe(&api, interfaces, parse_7plus)
                .map(|r| (nova_name(pid).unwrap_or("SteelSeries Arctis"), "headset", r))
        }
        0x230A | 0x2317 => nova_probe(&api, interfaces, parse_gamebuds)
            .map(|r| (nova_name(pid).unwrap_or("SteelSeries Arctis"), "headset", r)),
        // ---- 无线鼠标（0xAA / 0xD2 交换）----
        0x1830 | 0x1872 => mouse_probe(&api, interfaces, false)
            .map(|r| ("SteelSeries Rival 3 Wireless", "mouse", r)),
        0x1838 => mouse_probe(&api, interfaces, true)
            .map(|r| ("SteelSeries Aerox 3 Wireless", "mouse", r)),
        0x1878 => mouse_probe(&api, interfaces, true)
            .map(|r| ("SteelSeries Aerox 3 Wireless CS2 Dragon Lore", "mouse", r)),
        0x1852 => mouse_probe(&api, interfaces, true)
            .map(|r| ("SteelSeries Aerox 5 Wireless", "mouse", r)),
        0x185C => mouse_probe(&api, interfaces, true)
            .map(|r| ("SteelSeries Aerox 5 Wireless Destiny 2 Edition", "mouse", r)),
        0x1860 => mouse_probe(&api, interfaces, true)
            .map(|r| ("SteelSeries Aerox 5 Wireless Diablo IV Edition", "mouse", r)),
        0x1858 => mouse_probe(&api, interfaces, true)
            .map(|r| ("SteelSeries Aerox 9 Wireless", "mouse", r)),
        0x1874 => mouse_probe(&api, interfaces, true)
            .map(|r| ("SteelSeries Aerox 9 Wireless WOW Edition", "mouse", r)),
        // ---- 经典 Arctis ----
        0x12B3 | 0x12B6 | 0x12D7 | 0x12D5 => {
            classic_probe(&api, interfaces, 0xFF43, &[3], arctis1)
                .map(|r| (classic_name(pid), "headset", r))
        }
        0x12AD => classic_probe(&api, interfaces, 0, &[5], arctis7_2018)
            .map(|r| (classic_name(pid), "headset", r)),
        0x1260 | 0x1252 => classic_probe(&api, interfaces, 0, &[5], arctis7)
            .map(|r| (classic_name(pid), "headset", r)),
        0x12C2 | 0x12C4 => classic_probe(&api, interfaces, 0, &[0], arctis9)
            .map(|r| (classic_name(pid), "headset", r)),
        0x1290 => classic_probe(&api, interfaces, 0, &[0], pro_wireless)
            .map(|r| (classic_name(pid), "headset", r)),
        0x12E0 | 0x12E5 => classic_probe(&api, interfaces, 0, &[3, 4], nova_pro)
            .map(|r| (classic_name(pid), "headset", r)),
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

fn nova_name(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x2202 | 0x22A1 | 0x227E => "SteelSeries Arctis Nova 7",
        0x2206 | 0x2258 | 0x229E | 0x22AD => "SteelSeries Arctis Nova 7x",
        0x22A4 | 0x22A5 => "SteelSeries Arctis Nova 7X",
        0x223A | 0x22A9 => "SteelSeries Arctis Nova 7 Diablo IV",
        0x227A => "SteelSeries Arctis Nova 7 WoW Edition",
        0x220A | 0x22A7 | 0x2298 => "SteelSeries Arctis Nova 7P",
        0x2232 => "SteelSeries Arctis Nova 5",
        0x2253 => "SteelSeries Arctis Nova 5X",
        0x2269 => "SteelSeries Arctis Nova 3P Wireless",
        0x226D => "SteelSeries Arctis Nova 3X Wireless",
        0x220E => "SteelSeries Arctis 7+",
        0x2212 => "SteelSeries Arctis 7+ PS5",
        0x2216 => "SteelSeries Arctis 7+ Xbox",
        0x2236 => "SteelSeries Arctis 7+ Destiny",
        0x230A => "SteelSeries Arctis GameBuds",
        0x2317 => "SteelSeries Arctis GameBuds X",
        _ => return None,
    })
}

fn classic_name(pid: u16) -> &'static str {
    match pid {
        0x12B3 => "SteelSeries Arctis 1 Wireless",
        0x12B6 => "SteelSeries Arctis 1 Wireless Xbox",
        0x12D7 => "SteelSeries Arctis 7X",
        0x12D5 => "SteelSeries Arctis 7P",
        0x12AD => "SteelSeries Arctis 7 (2019)",
        0x1260 => "SteelSeries Arctis 7",
        0x1252 => "SteelSeries Arctis Pro (2019)",
        0x12C2 | 0x12C4 => "SteelSeries Arctis 9",
        0x1290 => "SteelSeries Arctis Pro Wireless",
        0x12E0 => "SteelSeries Arctis Nova Pro Wireless",
        0x12E5 => "SteelSeries Arctis Nova Pro Wireless X",
        _ => "SteelSeries Arctis",
    }
}

/// Nova 7 原始固件上报 0..=4 离散档位（0/25/50/75/100），2026 固件与 Gen2 直读百分比
fn pid_discrete(pid: u16) -> bool {
    matches!(pid, 0x2202 | 0x2206 | 0x22A4 | 0x223A | 0x227A)
}

// ---------- Nova 交换 ----------

fn nova_probe(
    api: &HidApi,
    interfaces: &[HidDev],
    parse: impl Fn(&[u8]) -> Option<(u8, bool)>,
) -> Option<(u8, bool)> {
    nova_exchange(api, interfaces).and_then(|r| parse(&r))
}

/// 在 0xFFC0:0x0001 集合上写 [0x00,0xB0]，读以 0xB0 开头的应答。
fn nova_exchange(api: &HidApi, interfaces: &[HidDev]) -> Option<Vec<u8>> {
    let iface = interfaces
        .iter()
        .find(|d| d.usage_page == VENDOR_PAGE && d.usage == VENDOR_USAGE)?;
    let path = std::ffi::CString::from_vec_with_nul(iface.path.clone()).ok()?;
    let dev = api.open_path(&path).ok()?;
    dev.write(&[0x00, 0xB0]).ok()?;
    let deadline = Instant::now() + TIMEOUT;
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        match dev.read_timeout(&mut buf, READ_SLICE.as_millis() as i32) {
            Ok(n) if n > 0 && buf[0] == 0xB0 => return Some(buf[..n.max(4)].to_vec()),
            Ok(_) | Err(_) => continue,
        }
    }
    None
}

/// Nova 7：`b0 <link> <battery> <status>`；link 0x03 = 已连接（耳机离开后
/// battery 字节保留旧值，只有 link 字节能识破），status 01/02 = 充电。
fn parse_nova7(discrete: bool) -> impl Fn(&[u8]) -> Option<(u8, bool)> {
    move |r| {
        if r.len() < 4 || r[0] != 0xB0 || r[1] != 0x03 || r[3] == 0x00 {
            return None;
        }
        let level = if discrete {
            r[2].min(4) * 25
        } else {
            r[2].min(100)
        };
        Some((level, r[3] == 0x01 || r[3] == 0x02))
    }
}

/// Nova 5 / 3P：`b0 <status> <?> <battery> <charging>`；status 0x02 = 耳机离线。
fn parse_nova5(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 5 || r[0] != 0xB0 || r[1] == 0x02 {
        return None;
    }
    Some((r[3].min(100), r[4] == 0x01))
}

/// Arctis 7+：`b0 <status> <level 0..4> <charging>`；status 0x01 = 耳机离线。
fn parse_7plus(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 4 || r[0] != 0xB0 || r[1] == 0x01 {
        return None;
    }
    Some((r[2].min(4) * 25, r[3] == 0x01))
}

/// GameBuds：`b0 <?> <?> <左状态> <右状态> <左%> <右%>`；状态 0x03 = 出盒，
/// 在盒中的耳机上报 0% 不算数，取出盒耳机的较低值。
fn parse_gamebuds(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 7 || r[0] != 0xB0 {
        return None;
    }
    let mut levels: [Option<u8>; 2] = [None, None];
    for i in 0..2 {
        if r[3 + i] == 0x03 {
            levels[i] = Some(r[5 + i]);
        }
    }
    match (levels[0], levels[1]) {
        (Some(l), Some(r)) => Some((l.min(r).min(100), false)),
        (Some(l), None) | (None, Some(l)) => Some((l.min(100), false)),
        (None, None) => None,
    }
}

// ---------- 鼠标交换 ----------

/// `aerox` = true 走 Aerox 的 0xD2 查询，false 走 Rival 3 的 0xAA 查询。
fn mouse_probe(api: &HidApi, interfaces: &[HidDev], aerox: bool) -> Option<(u8, bool)> {
    let iface = interfaces
        .iter()
        .find(|d| d.usage_page == VENDOR_PAGE && d.usage == VENDOR_USAGE)?;
    let path = std::ffi::CString::from_vec_with_nul(iface.path.clone()).ok()?;
    let dev = api.open_path(&path).ok()?;

    let mut req = [0u8; 64];
    if aerox {
        // Aerox：接收器把有线电量命令 0x92 加 0x40 标志位 → 00 d2
        req[1] = 0xD2;
    } else {
        // Rival 3：00 aa 01
        req[1] = 0xAA;
        req[2] = 0x01;
    }
    let echo = req[1];
    // 链路可能打盹，写三轮、每轮短读几次
    for _ in 0..3 {
        dev.write(&req).ok()?;
        for _ in 0..6 {
            let mut buf = [0u8; 64];
            match dev.read_timeout(&mut buf, 100) {
                Ok(n) if n > 0 => {
                    // 回显可能在 [0]（无 report id）或 [1]（report id 0 前缀）
                    let m = if buf[0] == 0x00 && n > 1 { 1 } else { 0 };
                    if n > m && buf[m] == echo {
                        if aerox {
                            return parse_aerox(&buf[m..n]);
                        }
                        return parse_rival3(&buf[m..n]);
                    }
                }
                _ => continue,
            }
        }
    }
    None
}

/// Rival 3：`aa <level> <?> <charging>`（steel-mouse 布局；rivalcfg 的无回显
/// 布局 `<level> <?> <charging>` 也接受——充电字节必须为 0/1 防杂报误读）。
fn parse_rival3(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() >= 4 && r[0] == 0xAA {
        let level = r[1];
        if level <= 100 {
            return Some((level, r[3] != 0));
        }
        return None;
    }
    if r.len() >= 3 && r[0] <= 100 && (r[2] == 0 || r[2] == 1) {
        return Some((r[0], r[2] == 1));
    }
    None
}

/// Aerox：`d2 <level>`；level 字节 bit7 = 充电，低 7 位 1..=21 为 21 档刻度
/// （(v-1)*5），>21 直接是百分比；0 表示鼠标关机/休眠而非空电。
fn parse_aerox(r: &[u8]) -> Option<(u8, bool)> {
    if r.len() < 2 || r[0] != 0xD2 {
        return None;
    }
    let b = r[1];
    let v = b & 0x7F;
    if v == 0 {
        return None;
    }
    let level = if v > 21 { v.min(100) } else { (v - 1) * 5 };
    Some((level, b & 0x80 != 0))
}

// ---------- 经典 Arctis 交换 ----------

/// 一次「写请求 + 等待被 accept 的应答」；page 为 0 表示接受任意厂商页集合。
fn ask(dev: &hidapi::HidDevice, request: &[u8], accept: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    dev.write(request).ok()?;
    let deadline = Instant::now() + TIMEOUT;
    let mut buf = [0u8; 64];
    while Instant::now() < deadline {
        match dev.read_timeout(&mut buf, READ_SLICE.as_millis() as i32) {
            Ok(n) if n > 0 && accept(&buf[..n]) => return Some(buf[..n].to_vec()),
            Ok(_) | Err(_) => continue,
        }
    }
    None
}

fn echo2(a: u8, b: u8) -> impl Fn(&[u8]) -> bool {
    move |r| r.len() >= 4 && r[0] == a && r[1] == b
}

fn classic_probe(
    api: &HidApi,
    interfaces: &[HidDev],
    page: u16,
    prefer_ifaces: &[i32],
    exchange: impl Fn(&hidapi::HidDevice) -> Option<(u8, bool)>,
) -> Option<(u8, bool)> {
    // 目标集合：指定 usage page（Arctis 1 族 0xFF43）；其余型号优先接口号匹配
    // （Windows 可靠），无接口线索时退回任意厂商页集合，靠回显校验兜底
    let match_iface = |d: &HidDev| prefer_ifaces.contains(&d.interface);
    let mut candidates: Vec<&HidDev> = interfaces
        .iter()
        .filter(|d| {
            if page != 0 {
                d.usage_page == page
            } else {
                match_iface(d) || d.is_vendor_page()
            }
        })
        .collect();
    candidates.sort_by_key(|d| if match_iface(d) { 0 } else { 1 });
    candidates.truncate(3); // 有界探测：非目标集合写不进 / 无应答，很快跳过
    for iface in candidates {
        let Ok(path) = std::ffi::CString::from_vec_with_nul(iface.path.clone()) else {
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            continue;
        };
        if let Some(r) = exchange(&dev) {
            return Some(r);
        }
    }
    None
}

/// Arctis 1 / 7X / 7P：`06 12 → 06 12 <status> <level>`；status 0x01 = 离线，无充电标志。
fn arctis1(dev: &hidapi::HidDevice) -> Option<(u8, bool)> {
    let r = ask(dev, &[0x06, 0x12], echo2(0x06, 0x12))?;
    if r[2] == 0x01 {
        return None;
    }
    Some((r[3].min(100), false))
}

/// Arctis 7 2019：先 `06 14` 查连接（[2] 须为 0x03），再 `06 18` 读电量。
fn arctis7_2018(dev: &hidapi::HidDevice) -> Option<(u8, bool)> {
    let link = ask(dev, &[0x06, 0x14], echo2(0x06, 0x14))?;
    if link[2] != 0x03 {
        return None;
    }
    let r = ask(dev, &[0x06, 0x18], echo2(0x06, 0x18))?;
    Some((r[2].min(100), false))
}

/// Arctis 7 / Pro 2019：`06 18 → 06 18 <level>`；耳机关机时读 0 → 无读数。
fn arctis7(dev: &hidapi::HidDevice) -> Option<(u8, bool)> {
    let r = ask(dev, &[0x06, 0x18], echo2(0x06, 0x18))?;
    if r[2] == 0 {
        return None;
    }
    Some((r[2].min(100), false))
}

/// Arctis 9：`00 20 → aa 01 <?> <raw> <charging>`；raw 0x64..0x9A 线性折算
/// 0..100%，非 0xAA 应答 = 耳机离线。
fn arctis9(dev: &hidapi::HidDevice) -> Option<(u8, bool)> {
    let r = ask(
        dev,
        &[0x00, 0x20],
        |r| r.len() >= 5 && (r[0] == 0xAA || r[0] == 0x55),
    )?;
    if r[0] != 0xAA || r[1] != 0x01 {
        return None;
    }
    let raw = r[3].clamp(0x64, 0x9A);
    Some(((raw - 0x64) * 100 / (0x9A - 0x64), r[4] == 0x01))
}

/// Arctis Pro Wireless：`41 aa → <state>`（0x02 离线 0x04 在线），
/// 再 `40 aa → <level 0..4>` ×25%；应答不回显，只认文档值。
fn pro_wireless(dev: &hidapi::HidDevice) -> Option<(u8, bool)> {
    let state = ask(
        dev,
        &[0x41, 0xAA],
        |r| !r.is_empty() && (r[0] == 0x02 || r[0] == 0x04),
    )?;
    if state[0] == 0x02 {
        return None;
    }
    let r = ask(dev, &[0x40, 0xAA], |r| !r.is_empty() && r[0] <= 4)?;
    Some((r[0] * 25, false))
}

/// Nova Pro Wireless 底座：`06 b0 → [6]=九档刻度(0..8) [15]=状态`；
/// 状态 01 离线 / 02 插线充电（有线，不显示）/ 08 用电池。
fn nova_pro(dev: &hidapi::HidDevice) -> Option<(u8, bool)> {
    let r = ask(
        dev,
        &[0x06, 0xB0],
        |r| r.len() >= 16 && matches!(r[15], 0x01 | 0x02 | 0x08),
    )?;
    if r[15] == 0x01 || r[15] == 0x02 || r[6] > 8 {
        return None;
    }
    Some((r[6] * 100 / 8, false))
}
