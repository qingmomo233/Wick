import type { AppSettings, DeviceBattery } from './types';

export async function fetchDevices(): Promise<DeviceBattery[]> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<DeviceBattery[]>('get_devices');
}

export async function requestRefresh(): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('refresh_now');
}

/** 移除设备：从「已添加」列表删除并隐藏（设备消失后重连会再次自动添加）。 */
export async function removeDevice(id: string): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('remove_device', { id });
}

/** 手动排序（id 列表，顶在前）。 */
export async function reorderDevices(order: string[]): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('reorder_devices', { order });
}

/** 窗口置顶开关。 */
export async function setWindowAlwaysOnTop(top: boolean): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().setAlwaysOnTop(top);
}

export async function onDevicesUpdated(cb: (devices: DeviceBattery[]) => void): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  const un = await listen<DeviceBattery[]>('devices-updated', (e) => cb(e.payload));
  return un;
}

export async function onProbeFailed(cb: (name: string) => void): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  const un = await listen<string>('protocol-probe-failed', (e) => cb(e.payload));
  return un;
}

export async function minimizeWindow(): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().minimize();
}

/** 关闭按钮：Rust 侧按设置分流（最小化到托盘 / 退出程序）。 */
export async function closeApp(): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('close_app');
}

export async function fetchSettings(): Promise<AppSettings> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<AppSettings>('get_settings');
}

export async function saveSettings(settings: AppSettings): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('set_settings', { settings });
}
