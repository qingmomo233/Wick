//! 数据层验证：直接调用采集管线输出全部设备（不启动 UI）。
//! cargo run --example dump_devices

fn main() {
    let outcome =
        tauri::async_runtime::block_on(wick_lib::providers::collect_all(&[]));
    println!("---- {} devices ----", outcome.devices.len());
    for d in &outcome.devices {
        println!(
            "{:<28} | {:>4} | {:>5} | {:<9} | {}",
            d.name,
            format!("{:?}", d.level),
            format!("{:?}", d.charging),
            d.transport,
            d.protocol_status,
        );
    }
    if !outcome.probe_failures.is_empty() {
        println!("---- failures ----");
        for (id, name) in &outcome.probe_failures {
            println!("{id}: {name}");
        }
    }
}
