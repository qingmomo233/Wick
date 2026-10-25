//! 设备资料库：`devices/*.json`（用户可编辑），把同一物理设备的不同链路
//! 身份（2.4G 接收器名 / 蓝牙广播名 / VID:PID）归并为一张卡片。
//!
//! 文件名即设备展示名；字段：kind（mouse/keyboard/headset/...）、
//! aliases（其他名称，如蓝牙广播名）、vendor_ids（专属接收器 "vid:pid"）。
//! 首次启动自动生成已知设备与示例；手动刷新时重新加载。

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct DeviceInfo {
    #[serde(skip)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub vendor_ids: Vec<String>,
}

/// 内置默认资料（用户实测硬件），文件夹不存在时写出
const DEFAULTS: [(&str, &str); 3] = [
    (
        "ATK A9 Ultra Max 2.0 Tri-Mode.json",
        r#"{ "kind": "mouse", "aliases": ["ATK A9 ULTRA MAX 2.0"], "vendor_ids": ["373b:11fe"] }"#,
    ),
    (
        "AJ159 NL.json",
        r#"{ "kind": "mouse", "aliases": [], "vendor_ids": [] }"#,
    ),
    (
        "MCHOSE G75 5.0.json",
        r#"{ "kind": "keyboard", "aliases": [], "vendor_ids": ["3554:fa09"] }"#,
    ),
];

const README: &str = r#"每个 .json 文件描述一台设备，文件名即展示名。字段：
  kind       mouse | keyboard | headset | gamepad | speaker | phone | other
  aliases    该设备的其他名称（蓝牙广播名等，用于归并同一设备的不同链路）
  vendor_ids 该设备专属接收器的 "vid:pid"（小写十六进制；接收器插着即认为设备走 2.4G）
示例（新增设备时复制修改文件名与内容）：
  { "kind": "mouse", "aliases": ["它的蓝牙名"], "vendor_ids": ["046d:c52b"] }
修改后点击软件内刷新按钮即可生效（无需重启）。
"#;

fn dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri::Manager;
    app.path().app_config_dir().ok().map(|d| d.join("devices"))
}

pub fn load(app: &tauri::AppHandle) -> Vec<DeviceInfo> {
    let Some(dir) = dir(app) else {
        return Vec::new();
    };
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
        for (file, body) in DEFAULTS {
            let _ = std::fs::write(dir.join(file), body);
        }
        let _ = std::fs::write(dir.join("README.txt"), README);
    }
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<DeviceInfo>(&text) {
            Ok(mut info) => {
                info.name = stem.to_string();
                out.push(info);
            }
            Err(err) => eprintln!("[wick] warn: device db {stem} parse failed: {err}"),
        }
    }
    out
}

/// 按名称（含别名）匹配设备
pub fn by_name<'a>(db: &'a [DeviceInfo], name: &str) -> Option<&'a DeviceInfo> {
    let n = crate::providers::normalize_name(name);
    db.iter().find(|d| {
        crate::providers::normalize_name(&d.name) == n
            || d.aliases
                .iter()
                .any(|a| crate::providers::normalize_name(a) == n)
    })
}
