//! Linux 蓝牙电量：优先 BlueZ D-Bus（org.bluez.Battery1），
//! 回退 UPower D-Bus（BlueZ 会把支持电量上报的设备注册到 /org/bluez 路径下）。

use std::collections::HashMap;
use std::time::Duration;

use crate::model::{
    classify_kind, DeviceBattery, STATUS_OK, STATUS_UNSUPPORTED, TRANSPORT_BLUETOOTH,
};

type Props = HashMap<String, zbus::zvariant::OwnedValue>;

pub async fn collect() -> Vec<DeviceBattery> {
    tokio::time::timeout(Duration::from_secs(8), collect_inner())
        .await
        .unwrap_or_default()
}

async fn collect_inner() -> Vec<DeviceBattery> {
    match bluez_devices().await {
        Ok(devices) if !devices.is_empty() => devices,
        Ok(_) => upower_fallback().await,
        Err(e) => {
            eprintln!("[wick] warn: BlueZ D-Bus unavailable: {e}");
            upower_fallback().await
        }
    }
}

async fn bluez_devices() -> zbus::Result<Vec<DeviceBattery>> {
    use zbus::fdo::ObjectManagerProxy;

    let conn = zbus::Connection::system().await?;
    let manager = ObjectManagerProxy::new(&conn, "org.bluez", "/").await?;
    let objects = manager.get_managed_objects().await?;

    let mut out = Vec::new();
    for (_path, ifaces) in objects.iter() {
        let Some(device) = ifaces.get("org.bluez.Device1") else {
            continue;
        };
        let Some(name) = get_str(device, "Name") else {
            continue;
        };
        // 只输出已配对且当前在连的设备；未连接/仅配对的不显示
        if !get_bool(device, "Paired").unwrap_or(false) {
            continue;
        }
        if !get_bool(device, "Connected").unwrap_or(false) {
            continue;
        }
        let address = get_str(device, "Address").unwrap_or_default();
        let level = ifaces
            .get("org.bluez.Battery1")
            .and_then(|b| get_u8(b, "Percentage"));

        let kind = get_str(device, "Icon")
            .map(|icon| match icon.as_str() {
                "audio-headset" | "audio-headphones" | "audio-card" => "headset",
                "input-mouse" => "mouse",
                "input-keyboard" => "keyboard",
                "input-gaming" => "gamepad",
                "input-tablet" => "other",
                "phone" => "phone",
                _ => classify_kind(&name),
            })
            .unwrap_or_else(|| classify_kind(&name))
            .to_string();

        out.push(DeviceBattery {
            id: format!("bt-bluez:{}", address.replace(':', "").to_lowercase()),
            name: name.clone(),
            kind,
            level,
            charging: None,
            connected: true,
            transport: TRANSPORT_BLUETOOTH.to_string(),
            source: "bluez".to_string(),
            protocol_status: if level.is_some() {
                STATUS_OK.to_string()
            } else {
                STATUS_UNSUPPORTED.to_string()
            },
            updated_at: DeviceBattery::now_ms(),
        });
    }
    Ok(out)
}

/// UPower 回退：BlueZ 把带电量上报的蓝牙设备注册在 /org/bluez 前缀路径下。
async fn upower_fallback() -> Vec<DeviceBattery> {
    let result = upower_devices().await;
    match result {
        Ok(devices) => devices,
        Err(e) => {
            eprintln!("[wick] warn: UPower fallback failed: {e}");
            Vec::new()
        }
    }
}

async fn upower_devices() -> zbus::Result<Vec<DeviceBattery>> {
    use zbus::Proxy;

    let conn = zbus::Connection::system().await?;
    let upower = Proxy::new(
        &conn,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
    )
    .await?;
    let paths: Vec<zbus::zvariant::OwnedObjectPath> = upower
        .call_method("EnumerateDevices", &())
        .await?
        .body()
        .deserialize()?;

    let mut out = Vec::new();
    for path in paths {
        // zbus 4：读取任意接口属性需经 org.freedesktop.Properties 的 PropertiesProxy
        let props_proxy = zbus::fdo::PropertiesProxy::new(
            &conn,
            "org.freedesktop.UPower",
            path.clone(),
        )
        .await?;
        let iface = zbus::names::InterfaceName::from_static_str("org.freedesktop.UPower.Device")?;
        let props: Props = props_proxy.get_all(Some(iface).into()).await?;

        let native_path = props
            .get("NativePath")
            .and_then(get_str_from_value)
            .unwrap_or_default();
        if !native_path.starts_with("/org/bluez") {
            continue;
        }
        let model = props
            .get("Model")
            .and_then(get_str_from_value)
            .unwrap_or_else(|| "蓝牙设备".to_string());
        let percentage = props.get("Percentage").and_then(get_u8_from_value);
        // UPower DeviceState: 1=充电中 2=放电 4=已充满 5/6=等待
        let state = props.get("State").and_then(get_u32_from_value);
        let is_present = props.get("IsPresent").and_then(get_bool_from_value);
        let charging = state.map(|s| s == 1);

        out.push(DeviceBattery {
            id: format!("bt-upower:{}", native_path),
            name: model.clone(),
            kind: classify_kind(&model).to_string(),
            level: percentage,
            charging,
            connected: is_present.unwrap_or(true),
            transport: TRANSPORT_BLUETOOTH.to_string(),
            source: "bluez".to_string(),
            protocol_status: if percentage.is_some() {
                STATUS_OK.to_string()
            } else {
                STATUS_UNSUPPORTED.to_string()
            },
            updated_at: DeviceBattery::now_ms(),
        });
    }
    Ok(out)
}

// ---------- zvariant 属性取值小工具 ----------

fn get_str(props: &Props, key: &str) -> Option<String> {
    props.get(key).and_then(get_str_from_value)
}

fn get_bool(props: &Props, key: &str) -> Option<bool> {
    props.get(key).and_then(get_bool_from_value)
}

fn get_u8(props: &Props, key: &str) -> Option<u8> {
    props.get(key).and_then(get_u8_from_value)
}

fn get_str_from_value(v: &zbus::zvariant::OwnedValue) -> Option<String> {
    v.downcast_ref::<String>().ok()
}

fn get_bool_from_value(v: &zbus::zvariant::OwnedValue) -> Option<bool> {
    v.downcast_ref::<bool>().ok()
}

fn get_u8_from_value(v: &zbus::zvariant::OwnedValue) -> Option<u8> {
    match v.downcast_ref::<u8>() {
        Ok(u) => Some(u),
        Err(_) => v
            .downcast_ref::<u32>()
            .ok()
            .and_then(|u| u8::try_from(u).ok()),
    }
}

fn get_u32_from_value(v: &zbus::zvariant::OwnedValue) -> Option<u32> {
    v.downcast_ref::<u32>().ok()
}
