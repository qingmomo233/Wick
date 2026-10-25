//! 8BitDo 手柄（D-input 模式，VID 0x2DC8）电量协议。
//! 来源：HaloBattery providers/eightbitdo.py（SDL 增强报告布局，真机验证）。
//!
//! 手柄普通输入报告不含电量，只有 SDL / 游戏（Steam）切换出的「增强报告」才有：
//! 蓝牙 report id 0x01（Pro 3 恒为 0x04），第 14 字节低 7 位 = 百分比、
//! bit7 = 充电。只读监听，绝不发送切换用的 feature report——切换并非无害
//! （HaloBattery #101：切换后 DirectInput 游戏失去输入直至手柄重启）。
//! 本轮没监听到带电量的报告就没有读数；只探测蓝牙链路（USB 直连多为充电线 = 有线）。

use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::model::{DeviceBattery, STATUS_OK, TRANSPORT_BLUETOOTH};
use crate::providers::hid::{HidDev, ProbeError};

const BATTERY_BYTE: usize = 14;
/// 增强报告以 ~85Hz 流出，普通报告仅在操作时发送——空窗口即「未被切换」
const LISTEN_WINDOW: Duration = Duration::from_millis(400);
const MAX_REPORTS: usize = 16;

fn known(pid: u16) -> Option<&'static str> {
    Some(match pid {
        0x6000 | 0x6100 => "8BitDo SF30 Pro",
        0x6001 | 0x6101 => "8BitDo SN30 Pro",
        0x6003 | 0x6006 => "8BitDo Pro 2",
        0x6009 => "8BitDo Pro 3",
        _ => return None,
    })
}

/// 增强报告 id：Pro 3 双传输恒为 0x04，其余蓝牙 0x01（本模块只走蓝牙）
fn enhanced_report_id(pid: u16) -> u8 {
    match pid {
        0x6009 => 0x04,
        _ => 0x01,
    }
}

pub fn device_id(interfaces: &[HidDev]) -> String {
    let d = &interfaces[0];
    match d.serial.trim() {
        s if !s.is_empty() => format!("8bitdo:{:04x}-{:04x}-{s}", d.vid, d.pid),
        _ => format!("8bitdo:{:04x}-{:04x}", d.vid, d.pid),
    }
}

pub fn probe(interfaces: &[HidDev]) -> Result<Option<DeviceBattery>, ProbeError> {
    let first = &interfaces[0];
    // USB 直连多为充电线（有线），不在监控范围
    if !first.bus_is_bluetooth() {
        return Ok(None);
    }
    let Some(name) = known(first.pid) else {
        return Ok(None);
    };
    let rid = enhanced_report_id(first.pid);
    let api = HidApi::new().map_err(|e| ProbeError::Io(e.to_string()))?;

    // 手柄集合（usage 0x01:0x04/0x05）优先
    let mut candidates: Vec<&HidDev> = interfaces.iter().collect();
    candidates.sort_by_key(|d| {
        if d.usage_page == 0x01 && (d.usage == 0x04 || d.usage == 0x05) {
            0
        } else {
            1
        }
    });
    for iface in candidates.iter().take(2) {
        let Ok(path) = std::ffi::CString::from_vec_with_nul(iface.path.clone()) else {
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            continue;
        };
        if dev.set_blocking_mode(false).is_err() {
            continue;
        }
        let deadline = Instant::now() + LISTEN_WINDOW;
        let mut buf = [0u8; 64];
        let mut count = 0usize;
        while Instant::now() < deadline && count < MAX_REPORTS {
            match dev.read(&mut buf) {
                Ok(n) if n > BATTERY_BYTE => {
                    count += 1;
                    if buf[0] != rid {
                        continue;
                    }
                    let b = buf[BATTERY_BYTE];
                    let level = b & 0x7F;
                    // 0 是普通报告的零填充而非空电（手柄在运行，0% 会误报低电量）
                    if level == 0 || level > 100 {
                        continue;
                    }
                    return Ok(Some(DeviceBattery {
                        id: device_id(interfaces),
                        name: name.to_string(),
                        kind: "gamepad".to_string(),
                        level: Some(level),
                        charging: Some(b & 0x80 != 0),
                        connected: true,
                        transport: TRANSPORT_BLUETOOTH.to_string(),
                        source: "vendor-hid".to_string(),
                        protocol_status: STATUS_OK.to_string(),
                        updated_at: DeviceBattery::now_ms(),
                    }));
                }
                _ => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
    Ok(None)
}
