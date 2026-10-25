import type { DeviceBattery, Transport } from '../types';
import BatteryRing from './BatteryRing';
import TransportBadge from './TransportBadge';

const KIND_LABEL: Record<string, string> = {
  headset: '耳机',
  mouse: '鼠标',
  keyboard: '键盘',
  gamepad: '手柄',
  speaker: '音箱',
  phone: '手机',
  other: '设备',
};

type Props = {
  device: DeviceBattery;
  /** 右下角红色按钮：移除设备（从已添加列表删除并隐藏） */
  onRemove: (id: string) => void;
};

export default function DeviceCard({ device, onRemove }: Props) {
  const unsupported = device.protocol_status !== 'ok';
  const offline = !device.connected;
  return (
    <article className={`card ${unsupported ? 'card-unsupported' : ''} ${offline ? 'card-offline' : ''}`}>
      <div className="card-ring">
        <BatteryRing
          level={device.protocol_status === 'ok' ? device.level : null}
          charging={device.charging}
          unsupported={unsupported}
        />
      </div>
      <div className="card-info">
        <div className="card-name" title={device.name}>{device.name}</div>
        <div className="card-meta">
          <TransportBadge transport={device.transport as Transport} />
          <span className="kind-label">{KIND_LABEL[device.kind] ?? '设备'}</span>
          {device.charging === true && <span className="charging-badge" title="充电中">⚡</span>}
          {offline && (
            <span className="offline-badge" title="设备休眠或未连接，保留上次读数">离线</span>
          )}
        </div>
      </div>
      {unsupported && <div className="card-unsupported-tag">协议未适配</div>}
      <button
        className="card-remove"
        title="移除设备（断开后重连会自动恢复）"
        onPointerDown={(e) => e.stopPropagation()}
        onClick={() => onRemove(device.id)}
      >
        ✕
      </button>
    </article>
  );
}
