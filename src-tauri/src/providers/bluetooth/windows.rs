//! Windows 蓝牙电量：SetupAPI 读取 PnP 设备 DEVPKEY 电池属性（毫秒级；
//! PowerShell Get-PnpDeviceProperty 在部分机器单次 20 秒，不可用）。
//! 属性键 {104EA319-6EE2-4701-BD47-8DDBF425BBE5}：pid 2 = 电量 %，pid 4 = 充电。

use std::time::Duration;

use serde::Deserialize;

use crate::model::{
    classify_kind, DeviceBattery, STATUS_OK, STATUS_UNSUPPORTED, TRANSPORT_BLUETOOTH,
};

#[derive(Deserialize, Debug, Clone)]
pub(crate) struct BtEntry {
    /// 设备实例路径（诊断用，聚合后不参与展示）
    pub inst: String,
    pub mac: String,
    pub name: String,
    pub level: Option<i64>,
    pub charging: Option<i64>,
    /// 电量来源优先级：2=GATT 电池服务子节点 1=设备根节点 0=其他服务节点。
    /// 聚合时高优先级的读数覆盖低优先级（根节点可能是旧缓存）。
    pub rank: u8,
    /// BTHLE\DEV_ 根节点（LE 身份设备；真实连接状态由 WinRT 查询，节点会残留）
    pub le_root: bool,
}

pub async fn collect() -> Vec<DeviceBattery> {
    tokio::time::timeout(Duration::from_secs(10), tauri::async_runtime::spawn_blocking(run))
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default()
}

/// LE 设备真实连接状态（WinRT BluetoothLEDevice.ConnectionStatus）。
/// BTHLE 根节点在设备切走后仍会残留并带冻结电量，节点在场 ≠ 在连；
/// 只有这里能拿到权威判定。API 失败时回退为「在场即已连接」。
#[cfg(target_os = "windows")]
fn le_connected(mac: &str) -> bool {
    use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothLEDevice};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    let Ok(addr) = u64::from_str_radix(mac, 16) else {
        return true;
    };
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let Ok(op) = BluetoothLEDevice::FromBluetoothAddressAsync(addr) else {
            return true;
        };
        let Ok(dev) = op.get() else {
            return true;
        };
        matches!(dev.ConnectionStatus(), Ok(status) if status == BluetoothConnectionStatus::Connected)
    }
}

#[cfg(target_os = "windows")]
fn run() -> Vec<DeviceBattery> {
    let entries = setupapi::query_bluetooth_entries();
    let table = radio_connected_table();
    build_devices(entries, &table)
}

#[cfg(not(target_os = "windows"))]
fn run() -> Vec<DeviceBattery> {
    Vec::new()
}

/// 经典蓝牙真实连接状态表（MAC 大写 -> fConnected）。
/// 无线电 API 失败时返回空表（调用方回退为「节点存在即已连接」旧行为）。
#[cfg(target_os = "windows")]
fn radio_connected_table() -> std::collections::HashMap<String, bool> {
    use std::collections::HashMap;
    use windows::Win32::Devices::Bluetooth::{
        BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindFirstRadio,
        BluetoothFindNextDevice, BluetoothFindRadioClose, BLUETOOTH_DEVICE_INFO,
        BLUETOOTH_DEVICE_SEARCH_PARAMS, BLUETOOTH_FIND_RADIO_PARAMS,
    };
    use windows::Win32::Foundation::HANDLE;

    let mut table = HashMap::new();
    unsafe {
        let radio_params = BLUETOOTH_FIND_RADIO_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
        };
        let mut radio_handle = HANDLE::default();
        let Ok(radio_iter) = BluetoothFindFirstRadio(&radio_params, &mut radio_handle) else {
            eprintln!("[wick] warn: bluetooth radio not found");
            return table;
        };
        let search = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: true.into(),
            fReturnRemembered: true.into(),
            fReturnUnknown: true.into(),
            fReturnConnected: true.into(),
            fIssueInquiry: false.into(),
            cTimeoutMultiplier: 1,
            hRadio: radio_handle,
        };
        let mut info = BLUETOOTH_DEVICE_INFO {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
            ..Default::default()
        };
        if let Ok(dev_iter) = BluetoothFindFirstDevice(&search, &mut info) {
            loop {
                let mac = format!("{:012X}", info.Address.Anonymous.ullLong);
                table.insert(mac, info.fConnected.as_bool());
                if BluetoothFindNextDevice(dev_iter, &mut info).is_err() {
                    break;
                }
            }
            let _ = BluetoothFindDeviceClose(dev_iter);
        } else {
            eprintln!("[wick] warn: BluetoothFindFirstDevice failed");
        }
        let _ = BluetoothFindRadioClose(radio_iter);
    }
    table
}

// ---------- SetupAPI 查询 ----------

#[cfg(target_os = "windows")]
mod setupapi {
    use windows::core::GUID;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
        SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW, SetupDiGetDeviceRegistryPropertyW, DIGCF_ALLCLASSES,
        DIGCF_PRESENT, SP_DEVINFO_DATA, SPDRP_DEVICEDESC, SPDRP_FRIENDLYNAME,
    };
    use windows::Win32::Devices::Properties::{DEVPROPKEY, DEVPROPTYPE};
    
    // {104EA319-6EE2-4701-BD47-8DDBF425BBE5}
    const BATTERY_FMTID: GUID = GUID::from_values(
        0x104E_A319,
        0x6EE2,
        0x4701,
        [0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5],
    );

    /// DEVPKEY 键：pid 2 = 电量百分比，pid 4 = 充电状态
    fn battery_key(pid: u32) -> DEVPROPKEY {
        DEVPROPKEY {
            fmtid: BATTERY_FMTID,
            pid,
        }
    }

    unsafe fn instance_id(
        set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
        info: &SP_DEVINFO_DATA,
    ) -> String {
        let mut buf = [0u16; 512];
        if SetupDiGetDeviceInstanceIdW(set, info, Some(&mut buf), None).is_err() {
            return String::new();
        }
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }

    unsafe fn device_name(
        set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
        info: &SP_DEVINFO_DATA,
    ) -> String {
        let mut buf = [0u16; 512];
        // FriendlyName 是设备真名；DEVICEDESC 多为通用名「蓝牙 LE 设备」，仅作回退
        for prop in [SPDRP_FRIENDLYNAME, SPDRP_DEVICEDESC] {
            let bytes = std::slice::from_raw_parts_mut(
                buf.as_mut_ptr() as *mut u8,
                std::mem::size_of_val(&buf),
            );
            buf.fill(0);
            if SetupDiGetDeviceRegistryPropertyW(set, info, prop, None, Some(bytes), None).is_ok()
            {
                let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
                let name = String::from_utf16_lossy(&buf[..len]);
                if !name.trim().is_empty() {
                    return name;
                }
            }
        }
        String::new()
    }

    unsafe fn u32_property(
        set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
        info: &SP_DEVINFO_DATA,
        pid: u32,
    ) -> Option<u32> {
        let key = battery_key(pid);
        let mut prop_type = DEVPROPTYPE(0);
        let mut value = 0u32;
        let mut required = 0u32;
        let bytes = std::slice::from_raw_parts_mut(
            &mut value as *mut u32 as *mut u8,
            4,
        );
        let ok = SetupDiGetDevicePropertyW(
            set,
            info,
            &key,
            &mut prop_type,
            Some(bytes),
            Some(&mut required),
            0,
        );
        if ok.is_ok() {
            Some(value)
        } else {
            None
        }
    }

    pub fn query_bluetooth_entries() -> Vec<super::BtEntry> {
        unsafe {
            let set =
                match SetupDiGetClassDevsW(None, None, None, DIGCF_ALLCLASSES | DIGCF_PRESENT) {
                    Ok(s) => s,
                    Err(_) => return Vec::new(),
                };
            let mut entries = Vec::new();
            let mut index = 0u32;
            loop {
                let mut info = SP_DEVINFO_DATA {
                    cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                    ..Default::default()
                };
                if SetupDiEnumDeviceInfo(set, index, &mut info).is_err() {
                    break; // 枚举完毕或出错
                }
                index += 1;

                let inst = instance_id(set, &info);
                if !inst.starts_with("BTH") {
                    continue;
                }
                let Some(mac) = super::extract_mac(&inst) else {
                    continue;
                };
                let name = super::clean_name(&device_name(set, &info));

                // 每个节点都查电量属性（可能挂在根节点或 GATT 电池服务子节点），
                // rank 决定聚合优先级（180F 电池服务 > 根节点 > 其他）
                let rank: u8 = if inst.contains("{0000180F") {
                    2
                } else if inst.contains("\\DEV_") {
                    1
                } else {
                    0
                };
                // BTHLE\DEV_ 根节点标记 LE 身份（注意：设备切走后节点会残留，
                // 真实连接状态在 build_devices 里经 WinRT 查询）
                let le_root = inst.starts_with("BTHLE\\DEV_");
                let (level, charging) = (
                    u32_property(set, &info, 2).map(|v| v as i64),
                    u32_property(set, &info, 4).map(|v| v as i64),
                );

                entries.push(super::BtEntry {
                    inst,
                    mac,
                    name,
                    level,
                    charging,
                    rank,
                    le_root,
                });
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
            entries
        }
    }
}

#[cfg(target_os = "windows")]
/// 调试用原始节点（dump_bt 示例）：实例路径 / 名字 / 电量 / 充电 / rank / LE根
pub type BthNodeRow = (String, String, Option<i64>, Option<i64>, u8, bool);

#[cfg(target_os = "windows")]
pub fn debug_bth_nodes() -> Vec<BthNodeRow> {
    setupapi::query_bluetooth_entries()
        .into_iter()
        .map(|e| (e.inst, e.name, e.level, e.charging, e.rank, e.le_root))
        .collect()
}

// ---------- MAC 提取与名字清洗（跨平台纯逻辑，可单测） ----------

/// 从蓝牙设备 InstanceId 提取 MAC（覆盖三种节点格式）：
/// 1) `BTHENUM\DEV_<MAC>` / `BTHLE\DEV_<MAC>`（根节点，含设备真名与电量）
/// 2) `BTHLEDEVICE\{GUID}_DEV_..._<MAC>`（LE GATT 服务节点）
/// 3) `BTHENUM\{GUID}_VID&.._PID&..\..._<MAC>_C..`（经典蓝牙服务节点）
pub(crate) fn extract_mac(inst: &str) -> Option<String> {
    let is_hex12 = |s: &str| s.len() == 12 && s.chars().all(|c| c.is_ascii_hexdigit());

    // 分支 1：\DEV_<12hex> 后跟 '\' 或结尾
    if let Some(pos) = inst.find("\\DEV_") {
        let rest = &inst[pos + 5..];
        if rest.len() >= 12 && is_hex12(&rest[..12]) {
            let after = rest[12..].chars().next();
            if after.is_none() || after == Some('\\') {
                return Some(rest[..12].to_uppercase());
            }
        }
    }

    // 分支 2：BTHLEDEVICE\{GUID}_DEV_..._<MAC>（取最后一个 '_' 段）
    if inst.starts_with("BTHLEDEVICE\\{") {
        if let Some(idx) = inst.rfind('_') {
            let tail = inst[idx + 1..].split('\\').next().unwrap_or("");
            if is_hex12(tail) {
                return Some(tail.to_uppercase());
            }
        }
    }

    // 分支 3：BTHENUM\{GUID}_...&<MAC>_C（实测前缀是 '&'，"_<12hex>_C" 或 "&<12hex>_C"，
    // 需前导字符非十六进制，避免长十六进制串的尾部 12 位误匹配）
    if inst.starts_with("BTHENUM\\{") {
        let chars: Vec<char> = inst.chars().collect();
        for i in 0..chars.len() {
            if i + 13 >= chars.len() {
                break;
            }
            let prev_is_hex = i > 0 && chars[i - 1].is_ascii_hexdigit();
            if prev_is_hex {
                continue;
            }
            let seg: String = chars[i..i + 12].iter().collect();
            if is_hex12(&seg) && chars[i + 12] == '_' && chars[i + 13] == 'C' {
                return Some(seg.to_uppercase());
            }
        }
    }
    None
}

/// 服务类子节点名（Avrcp/Hands-Free/GATT 等）不是设备真名，置空。
/// 「设备真名 + 服务后缀」的子节点（如 "xxx Avrcp 传输"）先剥离后缀保留真名。
pub(crate) fn clean_name(raw: &str) -> String {
    let mut name = raw.trim().to_string();
    if name.is_empty() {
        return String::new();
    }
    for suffix in ["Avrcp 传输", "耳机音频网关服务", "Avrcp Transport"] {
        if name.ends_with(suffix) {
            name = name[..name.len() - suffix.len()].trim_end().to_string();
        }
    }
    if name.is_empty() {
        return String::new();
    }
    const SERVICE_TOKENS: [&str; 21] = [
        "Avrcp", "AVRCP", "Hands-Free", "Hands Free", "HFP", "A2DP", "GATT", "通用属性",
        "通用访问", "配置文件", "音频网关", "外围设备", "信息服务", "串行", "个人区域",
        "枚举器", "%1", "Device Information", "Attribute", "Service", "enumerator",
    ];
    if SERVICE_TOKENS.iter().any(|t| name.contains(t)) {
        return String::new();
    }
    name
}

// ---------- 聚合与模型转换 ----------

fn build_devices(entries: Vec<BtEntry>, table: &std::collections::HashMap<String, bool>) -> Vec<DeviceBattery> {
    if entries.is_empty() {
        return Vec::new();
    }
    // 同一设备（同 MAC）多节点按字段合并：电量/充电取有读数的节点，
    // 名字取「真名 > 通用名（蓝牙 LE 设备/Bluetooth）> 空」中最好的
    fn name_rank(n: &str) -> u8 {
        let t = n.trim();
        if t.is_empty() {
            0
        } else if t.contains("蓝牙") || t.to_lowercase().contains("bluetooth") {
            1
        } else {
            2
        }
    }
    // 过滤垃圾服务子节点（无电量、无真名、非根节点，如手机 GATT / HID-over-GATT），
    // 否则会制造一堆「蓝牙设备 xxxxxx」永久离线卡
    let entries: Vec<BtEntry> = entries
        .into_iter()
        .filter(|e| e.level.is_some() || e.rank == 1 || name_rank(&e.name) == 2)
        .collect();
    if entries.is_empty() {
        return Vec::new();
    }
    // 连接门控（按 MAC 判定，保证同设备全部节点同进同出）：
    // LE 身份（BTHLE 前缀节点）查 WinRT 真实连接状态——根节点在设备切走后
    // 仍会残留；经典蓝牙查无线电层 fConnected。
    // 两套 API 都失败（表为空）时回退旧行为：节点存在即已连接。
    let table_empty = table.is_empty();
    let mut conn_cache: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let entries: Vec<BtEntry> = entries
        .into_iter()
        .filter(|e| {
            if table_empty {
                return true;
            }
            *conn_cache
                .entry(e.mac.clone())
                .or_insert_with(|| {
                    if e.inst.starts_with("BTHLE") {
                        le_connected(&e.mac)
                    } else {
                        table.get(&e.mac).copied().unwrap_or(false)
                    }
                })
        })
        .collect();
    if entries.is_empty() {
        return Vec::new();
    }
    let mut grouped: Vec<BtEntry> = Vec::new();
    for e in entries {
        match grouped.iter_mut().find(|g| g.mac == e.mac) {
            Some(existing) => {
                // 电量取优先级最高（rank 最大）的读数；rank 同时记录当前读数来源
                if e.level.is_some() && (existing.level.is_none() || e.rank > existing.rank) {
                    existing.level = e.level;
                    existing.charging = e.charging;
                    existing.rank = e.rank;
                } else if existing.charging.is_none() {
                    existing.charging = e.charging;
                }
                if name_rank(&e.name) > name_rank(&existing.name) {
                    existing.name = e.name;
                }
            }
            None => grouped.push(e),
        }
    }

    let now = DeviceBattery::now_ms();
    let mut out: Vec<DeviceBattery> = Vec::new();
    for e in grouped {
        let level = e.level.filter(|l| (0..=100).contains(l)).map(|l| l as u8);
        let charging = e.charging.filter(|c| *c == 0 || *c == 1).map(|c| c == 1);
        let name = if e.name.trim().is_empty() {
            format!("蓝牙设备 {}", &e.mac[..6.min(e.mac.len())])
        } else {
            e.name.clone()
        };
        // 同名去重：手机等设备经经典蓝牙与 LE 两条链路呈现为不同 MAC 的两个节点
        if let Some(existing) = out
            .iter_mut()
            .find(|d| d.name.to_lowercase().replace([' ', '-', '_'], "") == name.to_lowercase().replace([' ', '-', '_'], ""))
        {
            if existing.level.is_none() && level.is_some() {
                existing.level = level;
                existing.charging = charging;
                existing.protocol_status = STATUS_OK.to_string();
            }
            continue;
        }
        out.push(DeviceBattery {
            id: format!("bt-win:{}", e.mac.to_lowercase()),
            name: name.clone(),
            kind: classify_kind(&name).to_string(),
            level,
            charging,
            connected: true,
            transport: TRANSPORT_BLUETOOTH.to_string(),
            source: "windows-bt".to_string(),
            protocol_status: if level.is_some() {
                STATUS_OK.to_string()
            } else {
                // 设备在但 Windows 未暴露其电量（无标准电量上报）→ 如实标记
                STATUS_UNSUPPORTED.to_string()
            },
            updated_at: now,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_mac_formats() {
        assert_eq!(
            extract_mac(r"BTHLE\DEV_DB57C7716058\7&7E593D0&1&DB57C7716058"),
            Some("DB57C7716058".into())
        );
        assert_eq!(
            extract_mac(r"BTHENUM\DEV_882F92C5212E\7&1E9ADD38&1&BLUETOOTHDEVICE_882F92C5212E"),
            Some("882F92C5212E".into())
        );
        assert_eq!(
            extract_mac(r"BTHLEDEVICE\{0000180A-0000-1000-8000-00805F9B34FB}_DEV_VID&023554_PID&FA07_REV&6701_DB57C7716058\8&253E0746&0&000A"),
            Some("DB57C7716058".into())
        );
        assert_eq!(
            extract_mac(r"BTHENUM\{0000110E-0000-1000-8000-00805F9B34FB}_VID&000102B0_PID&0000\7&1E9ADD38&1&A090B55D4D8A_C00000000"),
            Some("A090B55D4D8A".into())
        );
        assert_eq!(extract_mac(r"BTH\MS_BTHLE\6&12139909&0&3"), None);
    }

    #[test]
    fn clean_names() {
        assert_eq!(
            clean_name("REDMI Buds 8 青春版 Avrcp 传输"),
            "REDMI Buds 8 青春版"
        );
        assert_eq!(clean_name("Device Information Service"), "");
        assert_eq!(clean_name("MCHOSE G75 5.0"), "MCHOSE G75 5.0");
    }

    #[test]
    fn aggregate_by_mac() {
        let entries = vec![
            BtEntry { inst: String::new(), mac: "AABBCCDDEEFF".into(), name: String::new(), level: Some(82), charging: None, rank: 1, le_root: false },
            BtEntry { inst: String::new(), mac: "AABBCCDDEEFF".into(), name: "MCHOSE G75 5.0".into(), level: Some(82), charging: None, rank: 0, le_root: false },
            BtEntry { inst: String::new(), mac: "AABBCCDDEEFF".into(), name: "Generic Access Profile".into(), level: None, charging: None, rank: 0, le_root: false },
        ];
        let out = build_devices(entries, &std::collections::HashMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "MCHOSE G75 5.0");
        assert_eq!(out[0].level, Some(82));
    }

    #[test]
    fn battery_service_node_overrides_stale_root() {
        // 根节点 82%（旧缓存）应被 GATT 电池服务节点（rank 2）的 40% 覆盖
        let entries = vec![
            BtEntry { inst: String::new(), mac: "112233445566".into(), name: "REDMI Buds".into(), level: Some(82), charging: None, rank: 1, le_root: false },
            BtEntry { inst: String::new(), mac: "112233445566".into(), name: String::new(), level: Some(40), charging: Some(1), rank: 2, le_root: false },
        ];
        let out = build_devices(entries, &std::collections::HashMap::new());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].level, Some(40));
        assert_eq!(out[0].charging, Some(true));
        assert_eq!(out[0].name, "REDMI Buds");
    }

    #[test]
    fn disconnected_classic_device_hidden() {
        // 已配对但未连接（无线电层 fConnected=false）→ 不生成卡片
        let entries = vec![
            BtEntry { inst: String::new(), mac: "882F92C5212E".into(), name: "MooYu233".into(), level: None, charging: None, rank: 1, le_root: false },
            BtEntry { inst: String::new(), mac: "4142CEF89B5E".into(), name: "TWS-895B".into(), level: Some(100), charging: None, rank: 0, le_root: false },
            BtEntry { inst: String::new(), mac: "A090B55D4D8A".into(), name: "REDMI Buds".into(), level: Some(55), charging: None, rank: 1, le_root: false },
        ];
        let mut table = std::collections::HashMap::new();
        table.insert("882F92C5212E".to_string(), false);
        table.insert("4142CEF89B5E".to_string(), false);
        table.insert("A090B55D4D8A".to_string(), true);
        let out = build_devices(entries, &table);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "REDMI Buds");
    }

    #[test]
    fn le_root_with_empty_table_falls_back_to_present() {
        // 无线电表为空（API 失败回退）→ LE 根节点存在即显示，不查 WinRT
        let entries = vec![
            BtEntry { inst: String::new(), mac: "DB57C7716058".into(), name: "MCHOSE G75 5.0".into(), level: Some(82), charging: None, rank: 1, le_root: true },
        ];
        let out = build_devices(entries, &std::collections::HashMap::new());
        assert_eq!(out.len(), 1);
        assert!(out[0].connected);
    }
}
