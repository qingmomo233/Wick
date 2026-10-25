import type { DeviceBattery, FilterKey } from '../types';

type Chip = { key: FilterKey; label: string };

/** 星闪（nearlink）在展示层并入「蓝牙」 */
const CHIPS: Chip[] = [
  { key: 'all', label: '全部' },
  { key: 'bluetooth', label: '蓝牙' },
  { key: '2.4g', label: '2.4G' },
];

function countFor(devices: DeviceBattery[], key: FilterKey): number {
  if (key === 'all') return devices.length;
  if (key === 'bluetooth') {
    return devices.filter((d) => d.transport === 'bluetooth' || d.transport === 'nearlink').length;
  }
  return devices.filter((d) => d.transport === key).length;
}

export default function FilterChips({
  devices,
  active,
  onChange,
}: {
  devices: DeviceBattery[];
  active: FilterKey;
  onChange: (k: FilterKey) => void;
}) {
  return (
    <nav className="chips">
      {CHIPS.map((c) => {
        const n = countFor(devices, c.key);
        return (
          <button
            key={c.key}
            className={`chip ${active === c.key ? 'chip-active' : ''}`}
            onClick={() => onChange(c.key)}
          >
            {c.label}
            <span className="chip-count">{n}</span>
          </button>
        );
      })}
    </nav>
  );
}
