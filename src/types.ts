/** 与 src-tauri/src/model.rs 中 DeviceBattery 严格一致 */
export interface DeviceBattery {
  id: string;
  name: string;
  kind: DeviceKind;
  level: number | null;
  charging: boolean | null;
  connected: boolean;
  transport: Transport;
  source: string;
  protocol_status: ProtocolStatus;
  updated_at: number;
}

export type DeviceKind =
  | 'headset'
  | 'mouse'
  | 'keyboard'
  | 'gamepad'
  | 'speaker'
  | 'phone'
  | 'other';

export type Transport = 'bluetooth' | 'nearlink' | '2.4g' | 'unknown';

export type ProtocolStatus = 'ok' | 'unsupported';

export type FilterKey = 'all' | 'bluetooth' | '2.4g';

/** 与 src-tauri/src/settings.rs 中 AppSettings 严格一致 */
export interface AppSettings {
  autostart: boolean;
  /** "minimize" | "quit" */
  close_action: string;
  /** 0.2..=1.0 应用背景不透明度（不影响 UI 组件） */
  bg_opacity: number;
  /** 2..=60 秒 */
  poll_interval_secs: number;
  /** 窗口置顶 */
  always_on_top: boolean;
  /** 记住的窗口尺寸（逻辑像素），null = 默认 */
  window_width: number | null;
  window_height: number | null;
}

export const DEFAULT_SETTINGS: AppSettings = {
  autostart: false,
  close_action: 'minimize',
  bg_opacity: 1.0,
  poll_interval_secs: 5,
  always_on_top: false,
  window_width: null,
  window_height: null,
};
