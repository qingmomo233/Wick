//! ATK / VXE / VGN 系列无线鼠标（含星闪接收器）电量协议。
//! 来源：atk-tool / libatk-rs 交叉验证 + 真机实测。
//! - 接收器 VID 0x373B / 0x3554（Compx），接口 Usage Page 0xFF02 / 0xFF04，Report ID 0x08
//! - 命令帧 16 字节 [cmd, status, addr_be(2), len, data(10), checksum]，
//!   checksum = 0x55 - (0x08 + 前 15 字节之和)（u8 回绕）
//! - 电量查询 cmd = 0x04；应答 buf[6] = %，buf[7] == 0x01 充电，buf[8..10] = mV 大端；
//!   status=1 的 ACK 帧电量全 0，需过滤
//! - 8K dongle（373b:11fe）写入需完整 65 字节；鼠标深睡时忽略查询 → 先被动监听再主动查询
//! - 星闪接收器以 USB HID 呈现、协议同源，仅按产品名区分 transport

use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::model::{DeviceBattery, STATUS_OK, TRANSPORT_24G, TRANSPORT_NEARLINK};
use crate::providers::hid::{is_nearlink, registry, HidDev, ProbeError};

const REPORT_ID: u8 = 0x08;
const CMD_QUERY_BATTERY: u8 = 0x04;
const FRAME_LEN: usize = 16;
/// 主动查询的读超时（外层任务另有兜底超时）。
const READ_TIMEOUT: Duration = Duration::from_millis(1400);
/// 被动监听窗口：确认链路是否有流量（鼠标活动）。
const LISTEN_WINDOW: Duration = Duration::from_millis(450);

/// 稳定设备 id：基于 VID/PID（+序列号）。不使用 HID 路径哈希——
/// 接收器换 USB 口后路径变化会导致「已添加设备」认成新设备产生重复卡片。
pub fn device_id(interfaces: &[HidDev]) -> Option<String> {
    interfaces.first().map(|d| match d.serial.trim() {
        s if !s.is_empty() => format!("atk:{:04x}-{:04x}-{s}", d.vid, d.pid),
        _ => format!("atk:{:04x}-{:04x}", d.vid, d.pid),
    })
}

// MCHOSE 锁写接收器（3554:fa09）：FF04 feature 0x06 的电量位是冻结缓存
// （键盘插电充满时为 100，拨掉用到 82 后仍返回 100），不可用。
// 唯一实时来源是设备经接收器转发时 FF02 上偶发的 0x08 电量帧（被动监听捕获）。

/// 逐个尝试厂商接口；返回 None 表示本轮未读到（设备深睡或协议未适配）。
/// 调用方应结合上一轮结果做「保留上次读数」处理。
pub fn probe(interfaces: &[HidDev]) -> Result<Option<DeviceBattery>, ProbeError> {
    let api = HidApi::new().map_err(|e| ProbeError::Io(e.to_string()))?;

    // 优先尝试已知 usage page（0xFF02 / 0xFF04），最多取 2 个接口控制总耗时
    let mut sorted: Vec<&HidDev> = interfaces.iter().collect();
    sorted.sort_by_key(|d| {
        let rank = match d.usage_page {
            0xFF02 => 0,
            0xFF04 => 1,
            _ => 2,
        };
        (rank, d.usage_page)
    });
    sorted.truncate(2);

    for iface in sorted {
        let path = match std::ffi::CString::from_vec_with_nul(iface.path.clone()) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let dev = match api.open_path(&path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!(
                    "[wick] warn: atk open {:04x}:{:04x} failed: {e}",
                    iface.vid, iface.pid
                );
                continue;
            }
        };
        if let Some((level, charging)) = query_battery(&dev).unwrap_or(None) {
            let nearlink = is_nearlink(&iface.product, &iface.manufacturer);
            let fallback = iface.display_name("ATK/VXE");
            let name = match registry::product_name(iface.vid, iface.pid) {
                Some(model) => model.to_string(),
                None => {
                    if nearlink && iface.product.trim().is_empty() {
                        "星闪无线设备".to_string()
                    } else {
                        fallback
                    }
                }
            };
            let id = device_id(interfaces).unwrap_or_else(|| {
                format!("atk:{:04x}-{:04x}", iface.vid, iface.pid)
            });
            return Ok(Some(DeviceBattery {
                id,
                name,
                kind: "mouse".to_string(),
                level: Some(level),
                charging: Some(charging), // buf[7]==0x01：充电中（有线），真机实测确认
                connected: true,
                transport: if nearlink { TRANSPORT_NEARLINK } else { TRANSPORT_24G }.to_string(),
                source: "vendor-hid".to_string(),
                protocol_status: STATUS_OK.to_string(),
                updated_at: DeviceBattery::now_ms(),
            }));
        }
    }
    Ok(None)
}

/// 电量读取：先被动监听，再主动查询。
fn query_battery(dev: &hidapi::HidDevice) -> Result<Option<(u8, bool)>, ProbeError> {
    let _ = dev.set_blocking_mode(false);

    // 1) 被监听窗口内若恰好出现电量帧，直接采用（零写入）
    if let Some(hit) = listen_battery_frame(dev, LISTEN_WINDOW) {
        return Ok(Some(hit));
    }

    // 2) 主动查询；首写常用于唤醒省电中的鼠标，300ms 无应答补发一次
    drain(dev);
    let mut frame = [0u8; FRAME_LEN];
    frame[0] = CMD_QUERY_BATTERY;
    let sum: u16 = REPORT_ID as u16 + frame[..FRAME_LEN - 1].iter().map(|b| *b as u16).sum::<u16>();
    frame[FRAME_LEN - 1] = 0x55u8.wrapping_sub(sum as u8);
    let mut packet = Vec::with_capacity(1 + FRAME_LEN);
    packet.push(REPORT_ID);
    packet.extend_from_slice(&frame);
    if write_command(dev, &packet).is_err() {
        // 锁写接收器（3554:fa09）：命令发不出去，退化为纯被动监听到超时
        // （设备活跃时接收器会转发实时电量帧）
        return Ok(listen_battery_frame(dev, READ_TIMEOUT));
    }

    let deadline = Instant::now() + READ_TIMEOUT;
    let start = Instant::now();
    let mut resent = false;
    while Instant::now() < deadline {
        if !resent && start.elapsed() >= Duration::from_millis(300) {
            let _ = write_command(dev, &packet);
            resent = true;
        }
        if let Some(hit) = try_parse_read(dev) {
            return Ok(Some(hit));
        }
        std::thread::sleep(Duration::from_millis(4));
    }
    Ok(None)
}

/// 非阻塞读一帧并按电量格式解析；无数据或非电量帧返回 None（不等待）。
fn try_parse_read(dev: &hidapi::HidDevice) -> Option<(u8, bool)> {
    let mut buf = [0u8; 64];
    let n = match dev.read(&mut buf) {
        Ok(n) if n >= 10 => n,
        _ => return None,
    };
    // 应答可能带或不带 ReportID 前缀
    let f: &[u8] = if buf[0] == CMD_QUERY_BATTERY {
        &buf[..n]
    } else if buf.get(1) == Some(&CMD_QUERY_BATTERY) {
        &buf[1..n]
    } else {
        return None;
    };
    if f.len() < 9 {
        return None;
    }
    let level = f[5];
    if level > 100 {
        return None; // 非电量应答
    }
    let charging = f[6] == 0x01;
    let millivolts = ((f[7] as u16) << 8) | f[8] as u16;
    // 过滤 ACK 帧：电量/电压均为 0 的应答不是真实读数
    if millivolts == 0 && !charging {
        return None;
    }
    Some((level, charging))
}

/// 在给定时长内被动等待电量帧。
fn listen_battery_frame(dev: &hidapi::HidDevice, dur: Duration) -> Option<(u8, bool)> {
    let deadline = Instant::now() + dur;
    while Instant::now() < deadline {
        if let Some(hit) = try_parse_read(dev) {
            return Some(hit);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

/// 真机实测（373b:11fe 8K dongle）：需要完整 65 字节写入才应答；
/// 老型号（373b:1115/10c9）17 字节即可，65 失败时回退。
fn write_command(dev: &hidapi::HidDevice, packet: &[u8]) -> Result<(), ProbeError> {
    let mut padded = packet.to_vec();
    padded.resize(65, 0);
    if let Err(e65) = dev.write(&padded) {
        if let Err(e17) = dev.write(packet) {
            // 常见持续状态（该接口无兼容长度的输出报告），不刷日志；
            // 结果由调用方以「协议未适配」卡片如实呈现
            return Err(ProbeError::Io(format!("write failed: {e65} / {e17}")));
        }
    }
    Ok(())
}

/// 非阻塞地清空待读的旧报文。
fn drain(dev: &hidapi::HidDevice) {
    let mut buf = [0u8; 64];
    for _ in 0..8 {
        match dev.read(&mut buf) {
            Ok(n) if n > 0 => continue,
            _ => break,
        }
    }
}
