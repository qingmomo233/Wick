//! 诊断：对指定接收器做长时被动监听 + 只读 feature 扫描（不写入任何字节）。
//! cargo run --example dump_raw

use hidapi::HidApi;

fn main() {
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hidapi init failed: {e}");
            return;
        }
    };
    let targets: Vec<_> = api
        .device_list()
        .filter(|d| [0x373Bu16, 0x3554u16].contains(&d.vendor_id()))
        .filter(|d| (0xFF00..=0xFFFF).contains(&d.usage_page()))
        .collect();
    if targets.is_empty() {
        println!("no vendor interfaces");
        return;
    }

    for t in &targets {
        println!(
            "== {:04x}:{:04x} page=0x{:04x} usage={} ==",
            t.vendor_id(),
            t.product_id(),
            t.usage_page(),
            t.usage()
        );
        let Ok(dev) = api.open_path(t.path()) else {
            println!("  open failed");
            continue;
        };
        let _ = dev.set_blocking_mode(false);

        // 只读 feature 扫描
        for rid in 1u8..=0x20 {
            let mut buf = [0u8; 64];
            buf[0] = rid;
            if let Ok(n) = dev.get_feature_report(&mut buf) {
                println!("  feature[0x{rid:02x}] len={n} {:02x?}", &buf[..n.min(24)]);
            }
        }

        // 被动监听 input 报文
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        let mut count = 0usize;
        while std::time::Instant::now() < deadline {
            let mut buf = [0u8; 64];
            if let Ok(n) = dev.read(&mut buf) {
                if n > 0 {
                    count += 1;
                    println!("  input[{:5}] len={n} {:02x?}", count, &buf[..n.min(24)]);
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        println!("  listened {secs}s, {count} frames");
    }
}
