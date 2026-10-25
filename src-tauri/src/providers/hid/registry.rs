//! VID/PID → 协议解析器注册表。
//!
//! 2.4G / 星闪接收器的电量协议没有业界统一标准，全部为厂商私有 HID 协议。
//! 这里维护「已知协议」映射；未登记的设备走通用只读启发式探测，
//! 失败则如实标记 `unsupported`。
//!
//! 扩展方式：在 `lookup` 中登记 VID（必要时细化到 PID），并实现对应解析器。

/// 对某组 VID/PID 采用的探测计划。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// 罗技（HID++，覆盖 Unifying / Bolt / Lightspeed 接收器及直连设备）
    Logitech,
    /// ATK / VXE / VGN 家族（含 NearLink 星闪接收器，VID 0x373B）
    AtkFamily,
    /// 已知品牌的接收器，但尚未实现其私有协议 → 如实显示「协议未适配」
    KnownVendor(&'static str),
    /// 未登记 VID
    Unknown,
}

pub fn lookup(vid: u16, _pid: u16) -> Plan {
    match vid {
        // Logitech（含 Unifying/Bolt/Lightspeed/Naano 接收器与直连键鼠）
        0x046D => Plan::Logitech,
        // ATK / VXE / VGN（Compx），已验证型号：
        //   373b:1115 ATK A9 Plus、373b:10c9 Nearlink Mouse Dongle（星闪）、
        //   373b:11fe Wireless mouse 8k NANO dongle（真机实测）
        0x373B => Plan::AtkFamily,
        // Compx 另一批次接收器 VID（真机实测 3554:fa09 "2.4G Wireless Receiver"，
        // 厂商 usage page 0xFF02/0xFF04 与 0x373B 家族一致）
        0x3554 => Plan::AtkFamily,
        // 以下品牌已有公开的电量协议实现（可参考 HaloBattery 等项目），暂未接入
        0x1532 => Plan::KnownVendor("Razer"),
        0x1038 => Plan::KnownVendor("SteelSeries"),
        0x1B1C => Plan::KnownVendor("Corsair"),
        0x2DC8 => Plan::KnownVendor("8BitDo"),
        0x03F0 => Plan::KnownVendor("HP/HyperX"),
        _ => Plan::Unknown,
    }
}

/// 已知「接收器 VID/PID → 实际外设产品名」映射。
/// 接收器的 HID 产品字符串是通用 dongle 名（如 "Wireless mouse 8k NANO dongle-L"），
/// 不是用户认知中的设备名；电量协议命令里也不含设备名，故维护此映射。
/// 扩展方式：在此登记新 PID（可用 `cargo run --example list_hid` 查询接收器 PID）。
pub fn product_name(vid: u16, pid: u16) -> Option<&'static str> {
    match (vid, pid) {
        // 用户实测：该 dongle 连接 ATK A9 Ultra Max 2.0 Tri-Mode（星闪三模）
        (0x373B, 0x11FE) => Some("ATK A9 Ultra Max 2.0 Tri-Mode"),
        // atk-tool 文档中的已知设备
        (0x373B, 0x1115) => Some("ATK A9 Plus"),
        (0x373B, 0x10C9) => Some("ATK 星闪鼠标"),
        // 用户实测：3554:fa09 为 MCHOSE G75 键盘的 2.4G 接收器（与蓝牙名一致，
        // 便于与蓝牙卡片按名去重合并）
        (0x3554, 0xFA09) => Some("MCHOSE G75 5.0"),
        _ => None,
    }
}
