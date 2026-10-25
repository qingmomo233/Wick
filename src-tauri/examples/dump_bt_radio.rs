//! 诊断：枚举蓝牙无线电层面记住的设备（含真实连接状态 fConnected）。
//! 用于验证「已配对但未连接」设备能否与「正在连接」区分开。

use windows::Win32::Devices::Bluetooth::{
    BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindFirstRadio,
    BluetoothFindNextDevice, BluetoothFindRadioClose, BLUETOOTH_DEVICE_INFO,
    BLUETOOTH_DEVICE_SEARCH_PARAMS, BLUETOOTH_FIND_RADIO_PARAMS,
};
use windows::Win32::Foundation::HANDLE;

fn main() {
    unsafe {
        let radio_params =
            BLUETOOTH_FIND_RADIO_PARAMS { dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32 };
        let mut radio_handle = HANDLE::default();
        let radio_iter = match BluetoothFindFirstRadio(&radio_params, &mut radio_handle) {
            Ok(it) => it,
            Err(_) => {
                eprintln!("no bluetooth radio");
                return;
            }
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
        let dev_iter = match BluetoothFindFirstDevice(&search, &mut info) {
            Ok(it) => it,
            Err(e) => {
                eprintln!("no devices: {e}");
                let _ = BluetoothFindRadioClose(radio_iter);
                return;
            }
        };
        loop {
            let name_len = info.szName.iter().position(|c| *c == 0).unwrap_or(info.szName.len());
            let name = String::from_utf16_lossy(&info.szName[..name_len]);
            let mac = info.Address.Anonymous.ullLong; // 大端：直接格式化即 AA:BB:.. 顺序
            println!(
                "{:012X} | conn={} remembered={} auth={} | {}",
                mac,
                info.fConnected.as_bool(),
                info.fRemembered.as_bool(),
                info.fAuthenticated.as_bool(),
                name
            );
            if BluetoothFindNextDevice(dev_iter, &mut info).is_err() {
                break;
            }
        }
        let _ = BluetoothFindDeviceClose(dev_iter);
        // 只枚举第一个无线电足够（笔记本内置蓝牙）
        let _ = BluetoothFindRadioClose(radio_iter);
    }
}
