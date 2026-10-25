use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::model::{DeviceBattery, STATUS_OK};
use crate::providers;
use crate::state::AppState;

/// 单次整体采集的超时上限（各协议调用另有更短的独立超时）。
const COLLECT_TIMEOUT: Duration = Duration::from_secs(14);
/// 低电量阈值（百分比）。
const LOW_LEVEL: u8 = 20;
/// 同一设备两次低电量通知的最小间隔：1 小时。
const NOTIFY_COOLDOWN_MS: i64 = 60 * 60 * 1000;

pub async fn run(app: AppHandle) {
    loop {
        let round_start = std::time::Instant::now();
        collect_and_publish(&app).await;
        // 每轮从设置读取检测间隔（2..=60 秒）。周期从轮起算：
        // 采集快则补足间隔，采集慢（深睡探测可达数秒）则只留最短缓冲
        let interval = app
            .state::<AppState>()
            .settings
            .lock()
            .map(|s| s.poll_interval_secs.clamp(2, 60))
            .unwrap_or(5);
        let sleep = Duration::from_secs(interval as u64)
            .saturating_sub(round_start.elapsed())
            .max(Duration::from_millis(500));
        eprintln!(
            "[wick] collect round done in {:.1}s, next in {:.1}s",
            round_start.elapsed().as_secs_f32(),
            sleep.as_secs_f32()
        );
        tokio::time::sleep(sleep).await;
    }
}

pub async fn collect_and_publish(app: &AppHandle) {
    collect_impl(app, false).await;
}

/// 手动刷新：重载设备资料库、清除隐藏名单（全部重新检测），强制推送最新列表。
pub async fn refresh(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Ok(mut d) = state.device_db.lock() {
        *d = crate::device_db::load(app);
    }
    if let Ok(mut known) = state.known.lock() {
        if !known.removed.is_empty() {
            known.removed.clear();
            crate::known::save(app, &known);
        }
    }
    collect_impl(app, true).await;
}

async fn collect_impl(app: &AppHandle, force: bool) {
    // 采集锁：防止轮询与手动刷新并发读写同一接收器
    let state = app.state::<AppState>();
    let _guard = state.collect_lock.lock().await;
    // provider 需要原始 id 空间的上次结果（keep-last 链）
    let raw = {
        state
            .raw_last
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    };
    let outcome = match tokio::time::timeout(COLLECT_TIMEOUT, providers::collect_all(&raw)).await {
        Ok(outcome) => outcome,
        Err(_) => {
            eprintln!("[wick] warn: overall collect timeout, keep last snapshot");
            return;
        }
    };
    // 更新原始快照
    if let Ok(mut r) = state.raw_last.lock() {
        *r = outcome.devices.clone();
    }
    // 首轮强制推送：webview 就绪前首轮事件可能丢失，后续轮次按 diff 推送
    let force = force || !state.emitted_once.swap(true, std::sync::atomic::Ordering::Relaxed);

    let devices = merge_with_known(app, outcome.devices, &outcome.present_vidpids);

    let state = app.state::<AppState>();

    // 1) 探测失败 → 前端 toast（只对「新失败」发事件，避免每 5 秒刷屏）
    if let Ok(mut map) = state.probe_failed.lock() {
        let prev = map.clone();
        map.clear();
        for (id, name) in &outcome.probe_failures {
            if map.insert(id.clone(), name.clone()).is_none() && !prev.contains_key(id) {
                let _ = app.emit("protocol-probe-failed", name.clone());
            }
        }
    }

    // 2) 与上次结果 diff，有变化才推送（手动刷新强制推送并刷新时间戳）
    let changed = match state.last_devices.lock() {
        Ok(mut last) => {
            let diff = last.len() != devices.len()
                || last.iter().zip(devices.iter()).any(|(a, b)| !a.same_reading(b));
            let changed = force || diff;
            if changed {
                *last = devices.clone();
            }
            changed
        }
        Err(_) => force,
    };
    if changed {
        let _ = app.emit("devices-updated", &devices);
    }

    // 3) 低电量通知（1 小时去重，仅针对在线设备）
    send_low_battery_notifications(app, &devices);
}

/// 与「已添加设备」库合并。
/// 1) 资料库身份归并：db 命中的设备用稳定 id/名称/类型；同一设备多链路同时
///    在线时蓝牙优先（代表当前活跃连接方式）
/// 2) 资料库合成：专属接收器在场但本轮无读数 → 2.4G 在线无读数卡
/// 3) 入库/离线合成：被移除且仍在线 → 隐藏；无读数的新设备 → 不显示不入库；
///    库内未扫到且有读数 → 离线卡片；无读数且未扫到 → 清除
fn merge_with_known(
    app: &AppHandle,
    scan: Vec<DeviceBattery>,
    present_vidpids: &[String],
) -> Vec<DeviceBattery> {
    let state = app.state::<AppState>();
    let Ok(mut known) = state.known.lock() else {
        return scan;
    };
    let db: Vec<crate::device_db::DeviceInfo> = state
        .device_db
        .lock()
        .map(|d| d.clone())
        .unwrap_or_default();
    let mut store_changed = false;

    // ---- 身份归并 ----
    let is_bt = |x: &DeviceBattery| x.transport == "bluetooth" || x.transport == "nearlink";
    let mut reps: Vec<DeviceBattery> = Vec::new();
    for mut d in scan {
        if let Some(info) = crate::device_db::by_name(&db, &d.name) {
            d.id = format!("db:{}", info.name);
            d.name = info.name.clone();
            if !info.kind.is_empty() {
                d.kind = info.kind.clone();
            }
        }
        match reps.iter_mut().find(|r| r.id == d.id) {
            Some(r) => {
                let replace = (is_bt(&d) && !is_bt(r))
                    || (is_bt(&d) == is_bt(r) && r.level.is_none() && d.level.is_some());
                if replace {
                    *r = d;
                }
            }
            None => reps.push(d),
        }
    }

    // ---- 资料库合成：接收器在场但本轮无读数（如键盘在锁写接收器上）----
    for info in &db {
        let id = format!("db:{}", info.name);
        if reps.iter().any(|r| r.id == id) {
            continue;
        }
        if !info.vendor_ids.iter().any(|v| present_vidpids.contains(v)) {
            continue;
        }
        reps.push(DeviceBattery {
            id,
            name: info.name.clone(),
            kind: if info.kind.is_empty() { "other".into() } else { info.kind.clone() },
            level: None,
            charging: None,
            connected: true,
            transport: "2.4g".to_string(),
            source: "device-db".to_string(),
            protocol_status: STATUS_OK.to_string(),
            updated_at: DeviceBattery::now_ms(),
        });
    }

    // 解除隐藏：被移除的设备一旦从系统中消失即可重新自动添加
    let scan_ids: Vec<String> = reps.iter().map(|d| d.id.clone()).collect();
    let before = known.removed.len();
    known.removed.retain(|id| scan_ids.contains(id));
    if known.removed.len() != before {
        store_changed = true;
    }

    // 仍在线的被移除设备不显示、不入库
    let visible: Vec<DeviceBattery> = reps
        .into_iter()
        .filter(|d| !known.removed.contains(&d.id))
        .collect();

    // 本轮扫到的设备更新入库并进入展示列表
    let mut shown: Vec<DeviceBattery> = Vec::new();
    for d in visible {
        match known.devices.iter_mut().find(|e| e.id == d.id) {
            Some(entry) => {
                let mut merged = d;
                // Windows 蓝牙电量缓存时有时无：本轮未读到但历史有 → 保留上次读数防跳变
                if merged.source == "windows-bt"
                    && merged.level.is_none()
                    && entry.level.is_some()
                {
                    merged.level = entry.level;
                    merged.charging = entry.charging;
                    merged.protocol_status = STATUS_OK.to_string();
                }
                // 资料库合成卡（设备在 2.4G、本轮无应答）：上次读数同样来自 2.4G
                // 时保留（跨链路的旧值不串用），updated_at 保留真实读取时间
                if merged.source == "device-db"
                    && merged.level.is_none()
                    && entry.level.is_some()
                    && entry.transport == "2.4g"
                {
                    merged.level = entry.level;
                    merged.charging = entry.charging;
                    merged.updated_at = entry.updated_at;
                }
                // 新名字是通用回退名（蓝牙设备 xx）而历史有真名 → 保留真名
                if merged.name.starts_with("蓝牙设备") && !entry.name.starts_with("蓝牙设备") {
                    merged.name = entry.name.clone();
                }
                if !entry.same_reading(&merged) {
                    store_changed = true;
                }
                *entry = merged.clone();
                shown.push(merged);
            }
            None => {
                // 有读数，或资料库声明的设备（接收器在场）才有资格进列表
                if d.level.is_some() || d.source == "device-db" {
                    known.order.insert(0, d.id.clone()); // 新设备置顶
                    known.devices.push(d.clone());
                    store_changed = true;
                    shown.push(d);
                }
            }
        }
    }

    // 同名跨链路清理：同一设备切换连接方式或蓝牙地址轮换会产生不同 id；
    // 该设备以某条链路在线时，清掉同名其他 id 的历史条目
    let online_names: Vec<String> = shown
        .iter()
        .map(|d| crate::providers::normalize_name(&d.name))
        .collect();
    let shown_ids: Vec<String> = shown.iter().map(|d| d.id.clone()).collect();
    known.devices.retain(|stored| {
        if shown_ids.contains(&stored.id) {
            return true;
        }
        if online_names.contains(&crate::providers::normalize_name(&stored.name)) {
            store_changed = true;
            return false;
        }
        true
    });

    // 合成离线卡片（仅有读数的设备）；无读数且本轮未扫到 → 从库中清除
    let mut cleaned: Vec<DeviceBattery> = Vec::new();
    for stored in known.devices.drain(..) {
        if shown_ids.contains(&stored.id) {
            cleaned.push(stored);
        } else if stored.level.is_some() {
            let mut offline = stored.clone();
            offline.connected = false;
            shown.push(offline);
            cleaned.push(stored);
        } else {
            store_changed = true; // 无读数且已消失 → 从库中删除
        }
    }
    known.devices = cleaned;
    let stored_ids: Vec<String> = known.devices.iter().map(|d| d.id.clone()).collect();
    known.order.retain(|id| stored_ids.contains(id));

    // 按 order 排序（顶在前）；不在 order 中的兜底放最后
    shown.sort_by_key(|d| known.order.iter().position(|id| *id == d.id).unwrap_or(usize::MAX));

    if store_changed {
        crate::known::save(app, &known);
    }
    shown
}

/// 手动拖拽排序：覆盖 order 列表并立即推送。
pub fn reorder_devices(app: &AppHandle, order: &[String]) {
    let state = app.state::<AppState>();
    let devices = {
        let Ok(mut known) = state.known.lock() else {
            return;
        };
        let ids: Vec<String> = known.devices.iter().map(|d| d.id.clone()).collect();
        let mut new_order: Vec<String> = order.iter().filter(|id| ids.contains(id)).cloned().collect();
        for id in ids {
            if !new_order.contains(&id) {
                new_order.push(id);
            }
        }
        known.order = new_order;
        crate::known::save(app, &known);
        let order = known.order.clone();
        match state.last_devices.lock() {
            Ok(mut last) => {
                last.sort_by_key(|d| order.iter().position(|id| *id == d.id).unwrap_or(usize::MAX));
                last.clone()
            }
            Err(_) => Vec::new(),
        }
    };
    let _ = app.emit("devices-updated", &devices);
}

/// 手动移除设备：从库中删除、加入隐藏名单、立即推送新列表。
pub fn remove_device(app: &AppHandle, id: &str) {
    let state = app.state::<AppState>();
    let devices = {
        let Ok(mut known) = state.known.lock() else {
            return;
        };
        known.devices.retain(|d| d.id != id);
        if !known.removed.iter().any(|r| r == id) {
            known.removed.push(id.to_string());
        }
        crate::known::save(app, &known);
        // 同步清理通知去重表
        if let Ok(mut notified) = state.low_notified_at.lock() {
            notified.remove(id);
        }
        // 立即更新推送列表（离线卡片随库一起消失；在线设备下轮起被隐藏）
        let removed = known.removed.clone();
        match state.last_devices.lock() {
            Ok(mut last) => {
                last.retain(|d| d.id != id && !removed.contains(&d.id));
                last.clone()
            }
            Err(_) => Vec::new(),
        }
    };
    let _ = app.emit("devices-updated", &devices);
}

fn send_low_battery_notifications(app: &AppHandle, devices: &[DeviceBattery]) {
    let state = app.state::<AppState>();
    let now = DeviceBattery::now_ms();

    // 先筛选出需要通知的设备（持锁期间不做任何 IO）
    let due: Vec<&DeviceBattery> = {
        let Ok(notified) = state.low_notified_at.lock() else {
            return;
        };
        devices
            .iter()
            .filter(|d| {
                d.protocol_status == STATUS_OK
                    && d.connected
                    && d.charging != Some(true)
                    && d.level.map(|l| l <= LOW_LEVEL).unwrap_or(false)
            })
            .filter(|d| now - notified.get(&d.id).copied().unwrap_or(0) >= NOTIFY_COOLDOWN_MS)
            .collect()
    };

    for d in due {
        if let Ok(mut notified) = state.low_notified_at.lock() {
            notified.insert(d.id.clone(), now);
        }
        let result = app
            .notification()
            .builder()
            .title("Wick 低电量提醒")
            .body(format!("「{}」电量仅 {}%，请及时充电。", d.name, d.level.unwrap_or(0)))
            .show();
        if let Err(e) = result {
            eprintln!("[wick] warn: notification failed: {e}");
        }
    }
}
