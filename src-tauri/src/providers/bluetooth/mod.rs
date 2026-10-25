#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{collect, debug_bth_nodes};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::collect;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
/// 其他平台暂不支持蓝牙采集。
pub async fn collect() -> Vec<crate::model::DeviceBattery> {
    Vec::new()
}
