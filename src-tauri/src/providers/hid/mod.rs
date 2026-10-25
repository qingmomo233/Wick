//! 厂商 HID 协议层（2.4G / 星闪接收器 / 蓝牙直连 HID 设备）。
//! 调度：VID/PID 注册表命中 → 专用解析器（罗技 / ATK / Razer / SteelSeries /
//! Corsair / HyperX / 8BitDo）；未命中 → 通用只读启发式（绝不写入未知设备）；
//! 读不出电量 → 不显示（只监控无线设备）。
//! 并发：hidapi 为阻塞 IO，一律 spawn_blocking + tokio timeout；
//! 同一物理设备组内串行探测，避免并发读写接收器导致应答串扰。

pub mod atk_vxe;
pub mod corsair;
pub mod eightbitdo;
pub mod generic;
pub mod hidpp_logi;
pub mod hyperx;
pub mod razer;
pub mod registry;
pub mod steelseries;

use std::time::Duration;

use hidapi::{BusType, HidApi};

use crate::model::{DeviceBattery, TRANSPORT_24G};

/// 每次协议调用的兜底超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// 罗技接收器需要顺序扫描 6 个配对槽位，放宽为整体 8 秒（单请求仍 ≤350ms）。
const RECEIVER_TIMEOUT: Duration = Duration::from_secs(8);
/// ATK 家族：被动监听 + 双轮主动查询 ×2 接口，整体放宽到 5 秒。
const ATK_TIMEOUT: Duration = Duration::from_secs(5);
/// Razer：已知 PID 单 tid 即命中；未知 PID 逐 tid 扫描（短窗口）。
const RAZER_TIMEOUT: Duration = Duration::from_secs(6);
/// SteelSeries：经典 Arctis 最多 3 集合 × 2 次问答。
const STEELSERIES_TIMEOUT: Duration = Duration::from_secs(7);
/// Corsair：bragi 需先跑会话（Virtuoso SE），首问 5s + 复问 2.5s + 充电 1s。
const CORSAIR_TIMEOUT: Duration = Duration::from_secs(12);
/// HyperX：Cloud III S 最多 2 集合 × 2s 读窗，其余型号几条 1s 命令。
const HYPERX_TIMEOUT: Duration = Duration::from_secs(6);
/// 设备枚举超时。
const ENUM_TIMEOUT: Duration = Duration::from_secs(3);

pub struct ProbeOutcome {
    pub devices: Vec<DeviceBattery>,
    /// (id, 展示名)：协议探测超时 / IO 错误的已知设备 → 前端 toast
    pub failures: Vec<(String, String)>,
    /// 本轮枚举到的全部 (vid, pid)，小写 "vid:pid"（供设备资料库判断接收器在场）
    pub present_vidpids: Vec<String>,
}

#[derive(thiserror::Error, Debug)]
pub enum ProbeError {
    #[error("protocol request timed out")]
    Timeout,
    #[error("hid io error: {0}")]
    Io(String),
}

/// 单个 HID 接口（top-level collection）的自持有快照。
/// hidapi 的 `DeviceInfo` 借用 `HidApi`，跨 await 持有不便，因此拷贝字段。
#[derive(Clone, Debug)]
pub struct HidDev {
    pub vid: u16,
    pub pid: u16,
    /// 含结尾 NUL 的设备路径字节
    pub path: Vec<u8>,
    pub product: String,
    pub manufacturer: String,
    pub serial: String,
    pub usage_page: u16,
    pub usage: u16,
    pub bus_type: BusType,
    /// USB 接口号（Windows 可靠；Linux 取自 sysfs，取不到为 -1），仅作探测排序
    pub interface: i32,
}

impl HidDev {
    pub fn is_vendor_page(&self) -> bool {
        (0xFF00..=0xFFFF).contains(&self.usage_page)
    }

    pub fn bus_is_bluetooth(&self) -> bool {
        matches!(self.bus_type, BusType::Bluetooth)
    }

    pub fn display_name(&self, fallback: &str) -> String {
        let p = self.product.trim();
        if !p.is_empty() {
            return p.to_string();
        }
        let m = self.manufacturer.trim();
        if !m.is_empty() {
            return format!("{m} {fallback}");
        }
        format!("HID {fallback}")
    }
}

/// 每组探测的产出（并行任务间传递）。
struct GroupResult {
    devices: Vec<DeviceBattery>,
    failures: Vec<(String, String)>,
}

impl GroupResult {
    fn empty() -> Self {
        GroupResult {
            devices: Vec::new(),
            failures: Vec::new(),
        }
    }
}

pub async fn collect(last: &[DeviceBattery]) -> ProbeOutcome {
    let devices = match tokio::time::timeout(
        ENUM_TIMEOUT,
        tauri::async_runtime::spawn_blocking(enumerate),
    )
    .await
    {
        Ok(Ok(devices)) => devices,
        Ok(Err(e)) => {
            eprintln!("[wick] warn: hid enumerate join error: {e}");
            Vec::new()
        }
        Err(_) => {
            eprintln!("[wick] warn: hid enumerate timeout");
            Vec::new()
        }
    };
    if devices.is_empty() {
        return ProbeOutcome {
            devices: Vec::new(),
            failures: Vec::new(),
            present_vidpids: Vec::new(),
        };
    }

    let groups = group_by_device(devices);
    let mut out = ProbeOutcome {
        devices: Vec::new(),
        failures: Vec::new(),
        present_vidpids: groups
            .iter()
            .map(|((vid, pid, _), _)| format!("{vid:04x}:{pid:04x}"))
            .collect(),
    };

    // 各物理设备组之间并行探测（组内串行，避免同一接收器并发读写串扰）
    let mut handles: Vec<tauri::async_runtime::JoinHandle<GroupResult>> = Vec::new();
    for (_key, interfaces) in groups {
        let plan = registry::lookup(interfaces[0].vid, interfaces[0].pid);
        match plan {
            registry::Plan::Logitech => {
                let last = last.to_vec();
                handles.push(tauri::async_runtime::spawn(collect_logitech(interfaces, last)));
            }
            registry::Plan::AtkFamily => {
                let last = last.to_vec();
                handles.push(tauri::async_runtime::spawn(collect_atk(interfaces, last)));
            }
            registry::Plan::Razer => {
                let last = last.to_vec();
                handles.push(tauri::async_runtime::spawn(collect_razer(interfaces, last)));
            }
            registry::Plan::SteelSeries => {
                let last = last.to_vec();
                handles.push(tauri::async_runtime::spawn(collect_steelseries(interfaces, last)));
            }
            registry::Plan::Corsair => {
                let last = last.to_vec();
                handles.push(tauri::async_runtime::spawn(collect_corsair(interfaces, last)));
            }
            registry::Plan::HyperX => {
                let last = last.to_vec();
                handles.push(tauri::async_runtime::spawn(collect_hyperx(interfaces, last)));
            }
            registry::Plan::EightBitDo => {
                handles.push(tauri::async_runtime::spawn(collect_eightbitdo(interfaces)));
            }
            registry::Plan::Unknown => {
                handles.push(tauri::async_runtime::spawn(collect_generic(interfaces)));
            }
        }
    }
    for handle in handles {
        match handle.await {
            Ok(result) => {
                out.devices.extend(result.devices);
                out.failures.extend(result.failures);
            }
            Err(e) => eprintln!("[wick] warn: hid group task join error: {e}"),
        }
    }

    // 同 id 去重（同型号双接收器可能被串到一组）：优先保留有电量读数的
    let mut deduped: Vec<DeviceBattery> = Vec::new();
    for d in out.devices {
        match deduped.iter_mut().find(|e| e.id == d.id) {
            Some(existing) => {
                if existing.level.is_none() && d.level.is_some() {
                    *existing = d;
                }
            }
            None => deduped.push(d),
        }
    }
    out.devices = deduped;
    out
}

// ---------- 各计划的具体调度 ----------

async fn collect_logitech(interfaces: Vec<HidDev>, last: Vec<DeviceBattery>) -> GroupResult {
    let mut out = GroupResult::empty();
    let vendor: Vec<HidDev> = interfaces
        .iter()
        .filter(|d| d.is_vendor_page())
        .cloned()
        .collect();
    if vendor.is_empty() {
        // 无厂商自定义接口：普通罗技有线键盘/鼠标（蓝牙直连的由系统蓝牙通道覆盖），跳过
        return out;
    }
    let vid = vendor[0].vid;
    let pid = vendor[0].pid;
    let receiver = vendor.iter().any(|d| is_receiver_product(&d.product));
    let label = vendor[0].display_name("Logitech");
    let id_prefix = format!("hidpp:{vid:04x}-{pid:04x}:");

    let result = if receiver {
        timeout_blocking(RECEIVER_TIMEOUT, {
            let vendor = vendor.clone();
            move || hidpp_logi::probe_receiver(&vendor)
        })
        .await
    } else {
        timeout_blocking(PROBE_TIMEOUT, {
            let vendor = vendor.clone();
            move || hidpp_logi::probe_direct(&vendor)
        })
        .await
    };

    match result {
        Ok(probe) => match probe {
            hidpp_logi::GroupProbe::Devices(devices) => out.devices.extend(devices),
            // 直连设备探测无结果（USB 有线 / 无电池）：按需求不显示
            hidpp_logi::GroupProbe::Nothing => {}
            hidpp_logi::GroupProbe::IoError(e) => {
                eprintln!("[wick] warn: hidpp open failed: {e}");
                out.failures.push((
                    format!("hidpp:{vid:04x}-{pid:04x}"),
                    format!("{label}（无法打开 HID 接口，Linux 下请检查 udev 权限）"),
                ));
                keep_last_devices(&mut out, &last, &id_prefix);
            }
        },
        Err(_) => {
            // 接收器整体超时（设备可能深睡）：保留上次读数，首次出现才提示
            eprintln!("[wick] warn: hidpp probe timeout on {label}");
            out.failures.push((id_prefix.clone(), format!("{label} 接收器")));
            keep_last_devices(&mut out, &last, &id_prefix);
        }
    }
    out
}

async fn collect_atk(interfaces: Vec<HidDev>, last: Vec<DeviceBattery>) -> GroupResult {
    let mut out = GroupResult::empty();
    let vendor: Vec<HidDev> = interfaces
        .iter()
        .filter(|d| d.is_vendor_page())
        .cloned()
        .collect();
    if vendor.is_empty() {
        return out;
    }
    let expected_id = atk_vxe::device_id(&vendor);
    let result = timeout_blocking(ATK_TIMEOUT, {
        let vendor = vendor.clone();
        move || atk_vxe::probe(&vendor)
    })
    .await;
    match result {
        Ok(Ok(Some(device))) => out.devices.push(device),
        // 本轮无应答（深睡 / 锁写 / 未适配）：有历史读数则保留，从未读到 → 不出卡片
        _ => {
            if let Some(id) = &expected_id {
                if let Some(prev) = last.iter().find(|d| &d.id == id && d.level.is_some()) {
                    out.devices.push(prev.clone());
                }
            }
        }
    }
    out
}

/// Razer / SteelSeries / Corsair / HyperX 共用的调度骨架：专用解析器探测，
/// 无应答（设备深睡 / 关机，接收器在场属正常状态）保留上次读数，
/// 打开接口失败（多为 Linux udev 权限）提示一次。
async fn collect_vendor_family(
    interfaces: Vec<HidDev>,
    last: Vec<DeviceBattery>,
    timeout: Duration,
    brand: &str,
    probe: fn(&[HidDev]) -> Result<Option<DeviceBattery>, ProbeError>,
    id_of: fn(&[HidDev]) -> String,
) -> GroupResult {
    let mut out = GroupResult::empty();
    let id = id_of(&interfaces);
    let label = interfaces[0].display_name(brand);
    let result = timeout_blocking(timeout, {
        let interfaces = interfaces.clone();
        move || probe(&interfaces)
    })
    .await;
    let io_error = matches!(result, Ok(Err(ProbeError::Io(_))));
    match result {
        Ok(Ok(Some(device))) => out.devices.push(device),
        // 深睡保留最后读数（与 ATK 一致）；整体超时不提示——接收器在场而
        // 外设睡眠是常态，不是故障
        _ => {
            if let Some(prev) = last.iter().find(|d| d.id == id && d.level.is_some()) {
                out.devices.push(prev.clone());
            }
        }
    }
    // 打开接口失败（多为 Linux udev 权限）：提示一次
    if io_error {
        out.failures.push((
            id,
            format!("{label}（无法打开 HID 接口，Linux 下请检查 udev 权限）"),
        ));
    }
    out
}

async fn collect_razer(interfaces: Vec<HidDev>, last: Vec<DeviceBattery>) -> GroupResult {
    collect_vendor_family(interfaces, last, RAZER_TIMEOUT, "Razer", razer::probe, razer::device_id)
        .await
}

async fn collect_steelseries(interfaces: Vec<HidDev>, last: Vec<DeviceBattery>) -> GroupResult {
    collect_vendor_family(
        interfaces,
        last,
        STEELSERIES_TIMEOUT,
        "SteelSeries",
        steelseries::probe,
        steelseries::device_id,
    )
    .await
}

async fn collect_corsair(interfaces: Vec<HidDev>, last: Vec<DeviceBattery>) -> GroupResult {
    collect_vendor_family(
        interfaces,
        last,
        CORSAIR_TIMEOUT,
        "Corsair",
        corsair::probe,
        corsair::device_id,
    )
    .await
}

async fn collect_hyperx(interfaces: Vec<HidDev>, last: Vec<DeviceBattery>) -> GroupResult {
    collect_vendor_family(
        interfaces,
        last,
        HYPERX_TIMEOUT,
        "HyperX",
        hyperx::probe,
        hyperx::device_id,
    )
    .await
}

async fn collect_eightbitdo(interfaces: Vec<HidDev>) -> GroupResult {
    // 只读监听，无应答即无卡片（手柄未被 Steam 切换时本就没有电量可读）
    let mut out = GroupResult::empty();
    let result = timeout_blocking(PROBE_TIMEOUT, {
        let interfaces = interfaces.clone();
        move || eightbitdo::probe(&interfaces)
    })
    .await;
    if let Ok(Ok(Some(device))) = result {
        out.devices.push(device);
    }
    out
}

async fn collect_generic(interfaces: Vec<HidDev>) -> GroupResult {
    let mut out = GroupResult::empty();
    let first = &interfaces[0];
    // I2C/SPI 为笔记本内置触摸板与键盘，不列入（非外设）
    if !matches!(first.bus_type, BusType::Usb | BusType::Bluetooth) {
        return out;
    }
    let bluetooth = first.bus_is_bluetooth();

    // 只对带厂商自定义接口的设备做只读启发式探测（绝不写未知设备）。
    // 无厂商接口的标准 HID 是普通有线键鼠（蓝牙直连设备由系统蓝牙通道覆盖）→ 跳过
    let target = interfaces.iter().find(|d| d.is_vendor_page()).cloned();
    let Some(target) = target else {
        return out;
    };
    // USB 上带厂商接口的可能是无线接收器，也可能是带 RGB 控制的有线键盘：
    // 有标准键盘接口（Usage Page 0x07）的按有线处理跳过；
    // 其余只有启发式稳定读出电量才显示（有电池读数的几乎只可能是无线外设）
    if !bluetooth && has_std_keyboard_page(&interfaces) {
        return out;
    }

    let result = timeout_blocking(PROBE_TIMEOUT, {
        let target = target.clone();
        move || generic::probe(&target)
    })
    .await;
    // USB 读到电量的未知厂商设备视为 2.4G 接收器；读不出则不显示
    if let Ok(Ok(Some(mut device))) = result {
        if !bluetooth {
            device.transport = TRANSPORT_24G.to_string();
        }
        out.devices.push(device);
    }
    out
}

/// 保留上一轮中 id 匹配前缀且带有电量读数的设备。
fn keep_last_devices(out: &mut GroupResult, last: &[DeviceBattery], id_prefix: &str) {
    for d in last {
        if d.id.starts_with(id_prefix) && d.level.is_some() {
            out.devices.push(d.clone());
        }
    }
}

// ---------- 工具 ----------

/// 组内是否含标准键盘接口（Usage Page 0x07）。
/// 带标准键盘接口的 USB 设备是有线键盘，不是 2.4G 接收器。
fn has_std_keyboard_page(interfaces: &[HidDev]) -> bool {
    interfaces.iter().any(|d| d.usage_page == 0x07)
}

pub(crate) fn is_nearlink(product: &str, manufacturer: &str) -> bool {
    let p = format!("{} {}", product, manufacturer).to_lowercase();
    p.contains("nearlink") || p.contains("星闪") || p.contains("near link")
}

fn is_receiver_product(product: &str) -> bool {
    let p = product.to_lowercase();
    ["unifying", "bolt", "lightspeed", "receiver", "nano"].iter().any(|k| p.contains(k))
}

fn enumerate() -> Vec<HidDev> {
    let api = match HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            eprintln!("[wick] warn: hidapi init failed: {e}");
            return Vec::new();
        }
    };
    api.device_list()
        .map(|d| HidDev {
            vid: d.vendor_id(),
            pid: d.product_id(),
            path: d.path().to_bytes_with_nul().to_vec(),
            product: d.product_string().unwrap_or("").to_string(),
            manufacturer: d.manufacturer_string().unwrap_or("").to_string(),
            serial: d.serial_number().unwrap_or("").to_string(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            bus_type: d.bus_type(),
            interface: d.interface_number(),
        })
        .collect()
}

/// 按 (vid, pid, serial) 聚合同一物理设备的多个接口，保持枚举顺序。
fn group_by_device(devices: Vec<HidDev>) -> Vec<((u16, u16, String), Vec<HidDev>)> {
    let mut groups: Vec<((u16, u16, String), Vec<HidDev>)> = Vec::new();
    for d in devices {
        let key = (d.vid, d.pid, d.serial.clone());
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(d),
            None => groups.push((key, vec![d])),
        }
    }
    groups
}

/// 在异步上下文中以超时运行阻塞探测。
async fn timeout_blocking<T, F>(timeout: Duration, f: F) -> Result<T, ProbeError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match tokio::time::timeout(timeout, tauri::async_runtime::spawn_blocking(f)).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(ProbeError::Io(format!("join error: {e}"))),
        Err(_) => Err(ProbeError::Timeout),
    }
}
