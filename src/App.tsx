import { useEffect, useMemo, useRef, useState } from 'react';
import type { AppSettings, DeviceBattery, FilterKey } from './types';
import { DEFAULT_SETTINGS } from './types';
import {
  fetchDevices,
  requestRefresh,
  removeDevice,
  reorderDevices,
  setWindowAlwaysOnTop,
  saveSettings,
  onDevicesUpdated,
  onProbeFailed,
  onWindowResized,
  isWindowMaximized,
  startWindowDrag,
  minimizeWindow,
  closeApp,
  fetchSettings,
} from './api';
import DeviceCard from './components/DeviceCard';
import FilterChips from './components/FilterChips';
import Toasts, { type ToastItem } from './components/Toasts';
import SettingsModal from './components/SettingsModal';

function matchesFilter(d: DeviceBattery, f: FilterKey): boolean {
  if (f === 'all') return true;
  if (f === 'bluetooth') {
    return d.transport === 'bluetooth' || d.transport === 'nearlink';
  }
  return d.transport === f;
}

/** 长按激活拖拽排序的延迟 */
const DRAG_DELAY_MS = 420;

export default function App() {
  const [devices, setDevices] = useState<DeviceBattery[]>([]);
  const [filter, setFilter] = useState<FilterKey>('all');
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshedAt, setRefreshedAt] = useState(0);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [showSettings, setShowSettings] = useState(false);
  const [alwaysOnTop, setAlwaysOnTop] = useState(false);

  // 拖拽排序状态
  const [dragOrder, setDragOrder] = useState<string[] | null>(null);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const dragOrderRef = useRef<string[] | null>(null);
  const drag = useRef<{ id: string; pointerId: number; timer: number; active: boolean; startY: number; el: HTMLElement | null } | null>(null);

  useEffect(() => {
    let disposed = false;
    const unsubs: Array<() => void> = [];
    // 先注册事件监听，再拉取初始数据：否则首轮推送可能落在监听注册前丢失
    void (async () => {
      const un1 = await onDevicesUpdated((d) => setDevices(d)).catch(() => null);
      if (un1) unsubs.push(un1);
      const un2 = await onProbeFailed((name) => {
        setToasts((prev) => [
          ...prev.slice(-2),
          { id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`, text: `设备「${name}」协议探测失败` },
        ]);
      }).catch(() => null);
      if (un2) unsubs.push(un2);
      if (disposed) return;
      try {
        setDevices(await fetchDevices());
      } catch {
        /* 忽略：等首轮推送 */
      }
      try {
        const s = await fetchSettings();
        if (!disposed) {
          setSettings(s);
          setAlwaysOnTop(s.always_on_top);
        }
      } catch {
        /* 忽略 */
      }
    })();
    return () => {
      disposed = true;
      unsubs.forEach((un) => un());
    };
  }, []);

  useEffect(() => {
    if (toasts.length === 0) return;
    const t = window.setTimeout(() => setToasts((p) => p.slice(1)), 3200);
    return () => window.clearTimeout(t);
  }, [toasts]);

  // 背景透明度：只作用于背景层，不影响 UI 组件
  useEffect(() => {
    const clamped = Math.min(1, Math.max(0.2, settings.bg_opacity || 1));
    document.documentElement.style.setProperty('--bg-opacity', String(clamped));
  }, [settings.bg_opacity]);

  // 记住窗口尺寸：调整结束后（800ms 防抖）持久化，下次启动由后端恢复。
  // 直接从后端取最新设置合并，避免与设置面板的保存互相覆盖
  useEffect(() => {
    let timer: number | undefined;
    let disposed = false;
    let un: (() => void) | null = null;
    void onWindowResized((pw, ph) => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        void (async () => {
          if (await isWindowMaximized().catch(() => false)) return;
          const scale = window.devicePixelRatio || 1;
          const w = Math.max(360, Math.round(pw / scale));
          const h = Math.max(480, Math.round(ph / scale));
          try {
            const s = await fetchSettings();
            if (s.window_width === w && s.window_height === h) return;
            await saveSettings({ ...s, window_width: w, window_height: h });
          } catch {
            /* 忽略：下次调整再存 */
          }
        })();
      }, 800);
    })
      .then((u) => {
        if (disposed) u();
        else un = u;
      })
      .catch(() => null);
    return () => {
      disposed = true;
      window.clearTimeout(timer);
      un?.();
    };
  }, []);

  const filtered = useMemo(
    () => devices.filter((d) => matchesFilter(d, filter)),
    [devices, filter],
  );

  // 拖拽中的预览顺序覆盖显示顺序
  const displayList = useMemo(() => {
    if (!dragOrder) return filtered;
    const idx = (id: string) => dragOrder.indexOf(id);
    return [...filtered].sort((a, b) => idx(a.id) - idx(b.id));
  }, [filtered, dragOrder]);

  const lastUpdated = useMemo(() => {
    const ts = devices.reduce((m, d) => Math.max(m, d.updated_at), refreshedAt);
    return ts > 0 ? new Date(ts).toLocaleTimeString('zh-CN', { hour12: false }) : '--:--:--';
  }, [devices, refreshedAt]);

  const onRefresh = async () => {
    setRefreshing(true);
    try {
      await requestRefresh();
      setRefreshedAt(Date.now());
    } catch {
      setRefreshing(false);
      return;
    }
    setRefreshing(false);
  };

  const onRemove = (id: string) => {
    void removeDevice(id).catch(() => undefined);
  };

  // ---------- 长按拖拽排序 ----------

  const startDrag = (e: React.PointerEvent, id: string, el: HTMLElement) => {
    if (e.button !== 0 || drag.current) return;
    drag.current = { id, pointerId: e.pointerId, timer: 0, active: false, startY: e.clientY, el };
    drag.current.timer = window.setTimeout(() => {
      const d = drag.current;
      if (!d) return;
      d.active = true;
      d.el?.setPointerCapture(d.pointerId);
      setDraggingId(id);
      const order = devices.map((x) => x.id);
      dragOrderRef.current = order;
      setDragOrder(order);
    }, DRAG_DELAY_MS);
  };

  const onDragMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    if (!d.active) {
      // 长按等待期间移动超过阈值 → 视为普通操作，取消拖拽
      if (Math.abs(e.clientY - d.startY) > 8) {
        window.clearTimeout(d.timer);
        drag.current = null;
      }
      return;
    }
    const target = document
      .elementFromPoint(e.clientX, e.clientY)
      ?.closest('[data-card-id]') as HTMLElement | null;
    const targetId = target?.dataset.cardId;
    if (!targetId || targetId === d.id) return;
    setDragOrder((prev) => {
      if (!prev) return prev;
      const from = prev.indexOf(d.id);
      const to = prev.indexOf(targetId);
      if (from < 0 || to < 0) return prev;
      const next = [...prev];
      next.splice(to, 0, ...next.splice(from, 1));
      dragOrderRef.current = next;
      return next;
    });
  };

  const endDrag = () => {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    window.clearTimeout(d.timer);
    if (!d.active) return;
    d.el?.releasePointerCapture(d.pointerId);
    setDraggingId(null);
    const order = dragOrderRef.current;
    dragOrderRef.current = null;
    setDragOrder(null);
    if (order) void reorderDevices(order).catch(() => undefined);
  };

  // ---------- 置顶 ----------

  const onTogglePin = async () => {
    const next = !alwaysOnTop;
    setAlwaysOnTop(next);
    try {
      await setWindowAlwaysOnTop(next);
      const nextSettings = { ...settings, always_on_top: next };
      setSettings(nextSettings);
      await saveSettings(nextSettings);
    } catch {
      setAlwaysOnTop(!next);
    }
  };

  // 顶栏拖拽：仅左键单击拖动；双击不动作（需求：双击不允许最大化）
  const onTitlebarMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0 || e.detail !== 1) return;
    if ((e.target as HTMLElement).closest('.win-controls')) return;
    void startWindowDrag();
  };

  return (
    <div className="app">
      <header className="titlebar" onMouseDown={onTitlebarMouseDown}>
        <div className="brand">
          <span className="brand-dot" />
          <span className="brand-name">Wick</span>
        </div>
        <div className="win-controls">
          <button
            className="win-btn win-btn-icon"
            title="设置"
            onClick={() => setShowSettings(true)}
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
              <circle cx="12" cy="12" r="3.2" />
              <path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.5-2.4 1a7 7 0 0 0-2-1.2L14.1 3h-4l-.4 2.6a7 7 0 0 0-2 1.2l-2.4-1-2 3.5 2 1.5A7 7 0 0 0 5 12c0 .4 0 .8.1 1.2l-2 1.5 2 3.5 2.4-1a7 7 0 0 0 2 1.2l.4 2.6h4l.4-2.6a7 7 0 0 0 2-1.2l2.4 1 2-3.5-2-1.5c.1-.4.1-.8.1-1.2Z" />
            </svg>
          </button>
          <button
            className={`win-btn win-btn-icon ${alwaysOnTop ? 'win-btn-active' : ''}`}
            title={alwaysOnTop ? '取消窗口置顶' : '窗口置顶'}
            onClick={() => void onTogglePin()}
          >
            {alwaysOnTop ? (
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
                <path d="M12 17v5" />
                <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1z" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
                <path d="M12 17v5" />
                <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1z" />
                <path d="m2 2 20 20" />
              </svg>
            )}
          </button>
          <button className="win-btn" title="最小化" onClick={() => void minimizeWindow()}>—</button>
          <button className="win-btn win-close" title="关闭" onClick={() => void closeApp()}>✕</button>
        </div>
      </header>

      <FilterChips devices={devices} active={filter} onChange={setFilter} />

      <main className="content">
        {displayList.length === 0 ? (
          <div className="empty">
            <svg className="empty-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.4">
              <path d="M7 7V5a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v2" />
              <rect x="4" y="7" width="16" height="13" rx="2.5" />
              <path d="M9.5 11.5h5M9.5 15h3" />
            </svg>
            <p className="empty-title">
              {devices.length === 0 ? '未发现无线设备' : '该分类下暂无设备'}
            </p>
            <p className="empty-hint">
              {devices.length === 0
                ? '连接蓝牙设备，或插入 2.4G / 星闪接收器后点击刷新'
                : '试试切换到「全部」分类'}
            </p>
          </div>
        ) : (
          <div className="grid">
            {displayList.map((d) => (
              <div
                key={d.id}
                data-card-id={d.id}
                className={`card-wrap ${draggingId === d.id ? 'card-dragging' : ''}`}
                onPointerDown={(e) => startDrag(e, d.id, e.currentTarget)}
                onPointerMove={onDragMove}
                onPointerUp={endDrag}
                onPointerCancel={endDrag}
              >
                <DeviceCard device={d} onRemove={onRemove} />
              </div>
            ))}
          </div>
        )}
      </main>

      <footer className="footer">
        <button
          className={`refresh-btn ${refreshing ? 'spinning' : ''}`}
          title="立即刷新"
          onClick={() => void onRefresh()}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M20 11a8 8 0 1 0-2.34 5.66" />
            <path d="M20 5v6h-6" />
          </svg>
        </button>
        <span className="footer-time">更新于 {lastUpdated}</span>
      </footer>

      {showSettings && (
        <SettingsModal
          settings={settings}
          onPreview={setSettings}
          onClose={() => setShowSettings(false)}
        />
      )}

      <Toasts items={toasts} />
    </div>
  );
}
