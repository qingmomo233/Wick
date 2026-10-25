pub mod bluetooth;
pub mod hid;

use crate::model::DeviceBattery;

pub struct CollectOutcome {
    pub devices: Vec<DeviceBattery>,
    /// (设备 id, 展示名)：协议探测超时 / IO 错误的已知设备
    pub probe_failures: Vec<(String, String)>,
    /// 本轮枚举到的接收器 VID:PID（"vid:pid"）
    pub present_vidpids: Vec<String>,
}

pub async fn collect_all(last: &[DeviceBattery]) -> CollectOutcome {
    let (bt, hid_outcome) = tokio::join!(bluetooth::collect(), hid::collect(last));
    let devices = merge(bt, hid_outcome.devices);
    CollectOutcome {
        devices,
        probe_failures: hid_outcome.failures,
        present_vidpids: hid_outcome.present_vidpids,
    }
}

/// 蓝牙 provider 与 HID provider 可能看到同一台设备（蓝牙+接收器双链路的
/// 键鼠）。按归一化名称去重，优先保留「有电量读数」的一方，平手优先蓝牙
/// （蓝牙卡片仅在链路真实在连时存在，代表设备当前活跃的连接方式）。
fn merge(bt: Vec<DeviceBattery>, hid: Vec<DeviceBattery>) -> Vec<DeviceBattery> {
    if bt.is_empty() {
        return hid;
    }
    let bt_names: Vec<String> = bt.iter().map(|d| normalize_name(&d.name)).collect();
    let mut out = bt;
    for d in hid {
        let name = normalize_name(&d.name);
        let clash_idx = bt_names.iter().position(|n| *n == name);
        match clash_idx {
            Some(i) => {
                let keep = &out[i];
                if keep.level.is_none() && d.level.is_some() {
                    out[i] = d;
                }
            }
            None => out.push(d),
        }
    }
    out
}

pub(crate) fn normalize_name(n: &str) -> String {
    n.to_lowercase().replace([' ', '-', '_'], "")
}

