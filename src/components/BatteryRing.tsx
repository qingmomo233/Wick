type Props = {
  level: number | null;
  charging: boolean | null;
  unsupported?: boolean;
};

const R = 30;
const CIRC = 2 * Math.PI * R;

function levelClass(level: number): string {
  if (level < 20) return 'critical';
  if (level < 50) return 'low';
  return 'good';
}

export default function BatteryRing({ level, charging, unsupported }: Props) {
  const hasLevel = level !== null;
  const pct = hasLevel ? Math.max(0, Math.min(100, level)) : 0;
  const noData = unsupported || !hasLevel;
  const dash = noData ? CIRC * 0.75 : (pct / 100) * CIRC;
  const cls = noData ? 'unsupported' : levelClass(pct);

  return (
    <div className={`ring ${hasLevel && level < 20 && !unsupported ? 'pulsing' : ''}`}>
      <svg viewBox="0 0 76 76" width="76" height="76">
        <circle
          className={`ring-track ${noData ? 'ring-track-dashed' : ''}`}
          cx="38" cy="38" r={R}
          fill="none"
        />
        <circle
          className={`ring-arc ring-arc-${cls}`}
          cx="38" cy="38" r={R}
          fill="none"
          strokeDasharray={`${dash} ${CIRC}`}
          transform="rotate(-90 38 38)"
          strokeLinecap={noData ? 'butt' : 'round'}
        />
      </svg>
      <div className="ring-center">
        {hasLevel ? (
          <>
            <span className={`ring-num ring-num-${cls}`}>{pct}</span>
            <span className="ring-unit">%</span>
          </>
        ) : (
          <span className="ring-unknown">--</span>
        )}
      </div>
      {charging === true && hasLevel && !unsupported && <span className="ring-charging" title="充电中">⚡</span>}
    </div>
  );
}
