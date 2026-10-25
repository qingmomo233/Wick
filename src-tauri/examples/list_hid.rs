//! 真机调试：列出本机全部 HID 设备（VID/PID/usage/传输类型）。
//! 用法：cargo run --example list_hid

fn main() {
    let api = match hidapi::HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hidapi init failed: {e}");
            return;
        }
    };
    println!(
        "{:<10} {:<10} {:<8} {:<8} {:<10} product",
        "VID", "PID", "usagePg", "usage", "bus"
    );
    for d in api.device_list() {
        println!(
            "{:<10} {:<10} 0x{:04x}   {:<8} {:<10} {}",
            format!("0x{:04x}", d.vendor_id()),
            format!("0x{:04x}", d.product_id()),
            d.usage_page(),
            d.usage(),
            format!("{:?}", d.bus_type()),
            d.product_string().unwrap_or("")
        );
    }
}
