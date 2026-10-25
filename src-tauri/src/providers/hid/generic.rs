//! 未知厂商设备通用只读启发式探测：只 get_feature_report，绝不写入。
//! 同一位置两轮（间隔 250ms）一致的 4..=100 值且形态像电量才采纳；
//! 候选不唯一则放弃，不猜测。

use std::collections::HashMap;
use std::time::Duration;

use hidapi::HidApi;

use crate::model::{
    classify_kind, DeviceBattery, STATUS_OK, TRANSPORT_BLUETOOTH, TRANSPORT_UNKNOWN,
};
use crate::providers::hid::{HidDev, ProbeError};

const REPORT_IDS: u8 = 0x10;
const SECOND_ROUND_DELAY: Duration = Duration::from_millis(250);

/// 返回 Ok(Some(device)) 表示启发式命中；Ok(None) 表示无法稳定读出。
pub fn probe(dev: &HidDev) -> Result<Option<DeviceBattery>, ProbeError> {
    let api = HidApi::new().map_err(|e| ProbeError::Io(e.to_string()))?;
    let path = std::ffi::CString::from_vec_with_nul(dev.path.clone())
        .map_err(|_| ProbeError::Io("invalid device path".into()))?;
    let hid = api
        .open_path(&path)
        .map_err(|e| ProbeError::Io(format!("open failed: {e}")))?;
    let _ = hid.set_blocking_mode(false);

    let first = scan_candidates(&hid);
    std::thread::sleep(SECOND_ROUND_DELAY);
    let second = scan_candidates(&hid);

    // 两轮一致的候选才算稳定
    let mut stable: Vec<((u8, usize), u8)> = Vec::new();
    for (key, v1) in &first {
        if let Some(v2) = second.get(key) {
            if v1.abs_diff(*v2) <= 1 {
                stable.push((*key, (*v1 + *v2) / 2));
            }
        }
    }

    // 候选不唯一 → 无法确定哪个是电量，宁可放弃
    if stable.len() != 1 {
        return Ok(None);
    }
    let level = stable[0].1.min(100);

    let name = dev.display_name(&format!("{:04x}:{:04x}", dev.vid, dev.pid));
    // id 基于 VID/PID（+序列号），不含 HID 路径：换 USB 口后身份保持稳定
    let id = match dev.serial.trim() {
        s if !s.is_empty() => format!("generic-hid:{:04x}-{:04x}-{s}", dev.vid, dev.pid),
        _ => format!("generic-hid:{:04x}-{:04x}", dev.vid, dev.pid),
    };
    Ok(Some(DeviceBattery {
        id,
        name,
        kind: classify_kind(&dev.product).to_string(),
        level: Some(level),
        charging: None,
        connected: true,
        transport: if dev.bus_is_bluetooth() {
            TRANSPORT_BLUETOOTH
        } else {
            TRANSPORT_UNKNOWN // 无法确定传输类型，不冒充 2.4G/星闪
        }
        .to_string(),
        source: "generic-hid".to_string(),
        protocol_status: STATUS_OK.to_string(),
        updated_at: DeviceBattery::now_ms(),
    }))
}

type Candidates = HashMap<(u8, usize), u8>;

fn scan_candidates(hid: &hidapi::HidDevice) -> Candidates {
    let mut out = Candidates::new();
    for rid in 1..=REPORT_IDS {
        // hidapi 2.6：report id 写入 buf[0]，单参数调用
        let mut buf = [0u8; 64];
        buf[0] = rid;
        if let Ok(n) = hid.get_feature_report(&mut buf) {
            if n < 4 {
                continue;
            }
            for i in 1..n {
                let v = buf[i];
                if (4..=100).contains(&v) && looks_like_battery(&buf[..n], i) {
                    out.insert((rid, i), v);
                }
            }
        }
    }
    out
}

fn looks_like_battery(report: &[u8], i: usize) -> bool {
    // 前一字节是常见充电标志 0x01
    (i > 0 && report[i - 1] == 0x01)
        // 后随电压高字节（mV 量级 < 0x0F）
        || (i + 1 < report.len() && report[i + 1] <= 0x0F)
        // 位于报文尾部
        || i + 1 == report.len()
}
