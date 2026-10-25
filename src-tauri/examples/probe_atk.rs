//! 真机调试：对系统中的 ATK/Compx 家族接收器（VID 0x373B / 0x3554）
//! 执行一次电量协议探测，验证协议解析器。
//! 用法：cargo run --example probe_atk

use hidapi::HidApi;
use wick_lib::providers::hid::atk_vxe;
use wick_lib::providers::hid::HidDev;

const ATK_VIDS: [u16; 2] = [0x373B, 0x3554];

fn main() {
    let api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hidapi init failed: {e}");
            return;
        }
    };

    let mut groups: Vec<(u16, u16, Vec<HidDev>)> = Vec::new();
    for d in api.device_list() {
        if !ATK_VIDS.contains(&d.vendor_id()) {
            continue;
        }
        if !(0xFF00..=0xFFFF).contains(&d.usage_page()) {
            continue; // 只取厂商自定义集合
        }
        let dev = HidDev {
            vid: d.vendor_id(),
            pid: d.product_id(),
            path: d.path().to_bytes_with_nul().to_vec(),
            product: d.product_string().unwrap_or("").to_string(),
            manufacturer: d.manufacturer_string().unwrap_or("").to_string(),
            serial: d.serial_number().unwrap_or("").to_string(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            bus_type: d.bus_type(),
        };
        println!(
            "found vendor interface {:04x}:{:04x} usage_page=0x{:04x} usage={} product='{}'",
            dev.vid, dev.pid, dev.usage_page, dev.usage, dev.product
        );
        match groups.iter_mut().find(|(v, p, _)| *v == dev.vid && *p == dev.pid) {
            Some((_, _, v)) => v.push(dev),
            None => groups.push((dev.vid, dev.pid, vec![dev])),
        }
    }

    if groups.is_empty() {
        println!("no ATK/Compx vendor interfaces found");
        return;
    }

    // 模拟生产轮询：连续 10 轮，验证活跃窗口命中 + 稳定性
    for round in 1..=10 {
        for (vid, pid, ifaces) in &groups {
            match atk_vxe::probe(ifaces) {
                Ok(Some(db)) => println!(
                    "[round {round}] SUCCESS {vid:04x}:{pid:04x} name='{}' level={:?} charging={:?} transport={}",
                    db.name, db.level, db.charging, db.transport
                ),
                Ok(None) => println!("[round {round}] {vid:04x}:{pid:04x} no response"),
                Err(e) => println!("[round {round}] {vid:04x}:{pid:04x} ERROR: {e}"),
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}
