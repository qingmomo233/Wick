//! 真机调试：对新接入的品牌家族（Razer / SteelSeries / Corsair / HyperX / 8BitDo）
//! 执行一次电量协议探测，验证协议解析器。
//! 用法：cargo run --example probe_vendor

use hidapi::HidApi;
use wick_lib::providers::hid::{corsair, eightbitdo, hyperx, razer, registry, steelseries, HidDev};

const BRAND_VIDS: [u16; 7] = [0x1532, 0x1038, 0x1B1C, 0x03F0, 0x0951, 0x2DC8, 0x046D];

fn main() {
    let api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hidapi init failed: {e}");
            return;
        }
    };

    // 按 (vid, pid, serial) 聚合接口，与生产枚举一致
    let mut groups: Vec<(u16, u16, String, Vec<HidDev>)> = Vec::new();
    for d in api.device_list() {
        if !BRAND_VIDS.contains(&d.vendor_id()) {
            continue;
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
            interface: d.interface_number(),
        };
        let key = (dev.vid, dev.pid, dev.serial.clone());
        match groups.iter_mut().find(|(v, p, s, _)| (*v, *p, s.clone()) == key) {
            Some((_, _, _, v)) => v.push(dev),
            None => groups.push((key.0, key.1, key.2, vec![dev])),
        }
    }

    if groups.is_empty() {
        println!("no brand devices found");
        return;
    }

    for (vid, pid, _serial, ifaces) in &groups {
        println!(
            "\n{:04x}:{pid:04x} product='{}' interfaces={} (usage pages: {})",
            vid,
            ifaces[0].product,
            ifaces.len(),
            ifaces
                .iter()
                .map(|d| format!("{:04x}:{:04x}", d.usage_page, d.usage))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let result = match registry::lookup(*vid, *pid) {
            registry::Plan::Razer => razer::probe(ifaces),
            registry::Plan::SteelSeries => steelseries::probe(ifaces),
            registry::Plan::Corsair => corsair::probe(ifaces),
            registry::Plan::HyperX => hyperx::probe(ifaces),
            registry::Plan::EightBitDo => eightbitdo::probe(ifaces),
            plan => {
                println!("  plan {plan:?}: not probed by this example");
                continue;
            }
        };
        match result {
            Ok(Some(db)) => println!(
                "  SUCCESS name='{}' kind={} level={:?} charging={:?} transport={}",
                db.name, db.kind, db.level, db.charging, db.transport
            ),
            Ok(None) => println!("  no response (device asleep / off / not a wireless model)"),
            Err(e) => println!("  ERROR: {e}"),
        }
    }
}
