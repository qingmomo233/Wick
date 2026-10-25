import type { Transport } from '../types';

/**
 * 展示层将星闪（nearlink）与经典蓝牙合并为「蓝牙」
 * （二者同为设备直连无线协议，仅协议栈不同；数据层仍保留区分）。
 */
const LABEL: Record<Transport, string> = {
  bluetooth: '蓝牙',
  nearlink: '蓝牙',
  '2.4g': '2.4G',
  unknown: '未知',
};

export default function TransportBadge({ transport }: { transport: Transport }) {
  const key = transport === 'nearlink' ? 'bluetooth' : transport;
  return <span className={`badge badge-${key === '2.4g' ? '24g' : key}`}>{LABEL[transport]}</span>;
}
