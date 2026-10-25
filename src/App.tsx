import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
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
  /** 长按等待期的按压反馈（轻微下压） */
  const [pressingId, setPressingId] = useState<string | null>(null);
  const dragOrderRef = useRef<string[] | null>(null);
  const drag = useRef<{
    id: string;
    pointerId: number;
    timer: number;
    active: boolean;
    startY: number;
    el: HTMLElement | null;
    /** 抓取点相对卡片左上角的偏移（浮卡跟随指针用） */
    offsetX: number;
    offsetY: number;
    /** 浮卡初始尺寸与位置（拾起时锁定） */
    width: number;
    height: number;
    homeX: number;
    homeY: number;
    lastX: number;
    lastY: number;
  } | null>(null);
  /** 浮空卡层（跟随指针，直接改 transform 不走 React 渲染） */
  const floatRef = useRef<HTMLDivElement>(null);

  // ---------- 换位 FLIP 动画 ----------
  // 顺序变化前记录各卡片位置，DOM 更新后把卡片从旧位置平滑滑到新位置
  // （时序与缓动参考 dnd-kit sortable：220ms / cubic-bezier(0.2, 0, 0, 1)）
  const flipRects = useRef<Map<string, DOMRect>>(new Map());
  const flipAnims = useRef<Map<string, Animation>>(new Map());
  /** FLIP 动画播放中的卡片 id：视觉位置未落定，不作为换位目标，
   *  否则换位判定会反复触发"换过去又换回来"的震荡 */
  const flipAnimating = useRef<Set<string>>(new Set());
  const reduceMotion = useMemo(
    () => typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches,
    [],
  );

  const captureFlip = () => {
    if (reduceMotion) return;
    flipRects.current.clear();
    document.querySelectorAll<HTMLElement>('[data-card-id]').forEach((el) => {
      if (el.dataset.cardId) flipRects.current.set(el.dataset.cardId, el.getBoundingClientRect());
    });
  };

  useLayoutEffect(() => {
    if (flipRects.current.size === 0 || reduceMotion) return;
    const rects = flipRects.current;
    flipRects.current = new Map();
    document.querySelectorAll<HTMLElement>('[data-card-id]').forEach((el) => {
      const id = el.dataset.cardId;
      if (!id || id === draggingId) return; // 占位卡不可见，无需动画（还要供落位取准确槽位）
      const prev = rects.get(id);
      if (!prev) return;
      const next = el.getBoundingClientRect();
      const dx = prev.left - next.left;
      const dy = prev.top - next.top;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
      flipAnims.current.get(id)?.cancel();
      const anim = el.animate(
        [
          { transform: `translate(${dx}px, ${dy}px)` },
          { transform: 'translate(0px, 0px)' },
        ],
        { duration: 220, easing: 'cubic-bezier(0.2, 0, 0, 1)' },
      );
      flipAnims.current.set(id, anim);
      flipAnimating.current.add(id);
      anim.onfinish = () => flipAnimating.current.delete(id);
      anim.oncancel = () => flipAnimating.current.delete(id);
    });
  }, [dragOrder, devices, draggingId]);

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

  // 浮卡层挂载：按拾起时锁定的尺寸/位置初始化
  useLayoutEffect(() => {
    const d = drag.current;
    const fl = floatRef.current;
    if (!draggingId || !d || !fl) return;
    fl.style.width = `${d.width}px`;
    fl.style.transform = `translate(${d.homeX}px, ${d.homeY}px)`;
  }, [draggingId]);

  const startDrag = (e: React.PointerEvent, id: string, el: HTMLElement) => {
    if (e.button !== 0 || drag.current) return;
    // 移除按钮上不启动拖拽（避免按压反馈闪烁 + 误触发长按）
    if ((e.target as HTMLElement).closest('.card-remove')) return;
    drag.current = {
      id,
      pointerId: e.pointerId,
      timer: 0,
      active: false,
      startY: e.clientY,
      el,
      offsetX: 0,
      offsetY: 0,
      width: 0,
      height: 0,
      homeX: 0,
      homeY: 0,
      lastX: e.clientX,
      lastY: e.clientY,
    };
    setPressingId(id);
    drag.current.timer = window.setTimeout(() => {
      const d = drag.current;
      if (!d?.el) return;
      const r = d.el.getBoundingClientRect();
      d.active = true;
      d.offsetX = d.lastX - r.left;
      d.offsetY = d.lastY - r.top;
      d.width = r.width;
      d.height = r.height;
      d.homeX = r.left;
      d.homeY = r.top;
      setPressingId(null);
      d.el.setPointerCapture(d.pointerId);
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
        setPressingId(null);
      }
      return;
    }
    d.lastX = e.clientX;
    d.lastY = e.clientY;
    // 浮卡跟随指针（直接写 transform，避免整列表重渲染）
    if (floatRef.current) {
      floatRef.current.style.transform = `translate(${e.clientX - d.offsetX}px, ${
        e.clientY - d.offsetY
      }px)`;
    }
    // 占位判定：浮卡中心落在哪张卡上，空槽就挪到哪里
    const fx = e.clientX - d.offsetX + d.width / 2;
    const fy = e.clientY - d.offsetY + d.height / 2;
    let hit: { el: HTMLElement; id: string } | null = null;
    for (const el of Array.from(document.querySelectorAll<HTMLElement>('[data-card-id]'))) {
      const cid = el.dataset.cardId;
      if (!cid || cid === d.id || flipAnimating.current.has(cid)) continue; // 动画中不参与判定
      const r = el.getBoundingClientRect();
      if (fx >= r.left && fx <= r.right && fy >= r.top && fy <= r.bottom) {
        hit = { el, id: cid };
        break;
      }
    }
    const prev = dragOrderRef.current;
    if (!hit || !prev) return;
    const r = hit.el.getBoundingClientRect();
    // 前/后判定：同一行比 x，跨行比 y（行主序网格）
    const before =
      Math.abs(fy - (r.top + r.height / 2)) <= r.height / 2
        ? fx < r.left + r.width / 2
        : fy < r.top + r.height / 2;
    const others = prev.filter((x) => x !== d.id);
    let k = others.indexOf(hit.id);
    if (!before) k += 1;
    const next = [...others.slice(0, k), d.id, ...others.slice(k)];
    if (next.join('\n') === prev.join('\n')) return; // 槽位未变
    captureFlip(); // DOM 更新前记录旧位置，渲染后播放让位滑动
    dragOrderRef.current = next;
    setDragOrder(next);
  };

  const endDrag = () => {
    const d = drag.current;
    drag.current = null;
    setPressingId(null);
    if (!d) return;
    window.clearTimeout(d.timer);
    if (!d.active) return;
    d.el?.releasePointerCapture(d.pointerId);
    const finish = () => {
      setDraggingId(null);
      const order = dragOrderRef.current;
      dragOrderRef.current = null;
      if (!order) {
        setDragOrder(null);
        return;
      }
      // 记录位置 → 提交顺序并立即采用返回值（避免回跳闪烁）
      captureFlip();
      void (async () => {
        try {
          const updated = await reorderDevices(order);
          setDevices(updated);
        } catch {
          /* 提交失败：回退显示当前列表顺序 */
        }
        setDragOrder(null);
      })();
    };
    // 落位动画：浮卡滑向自己的占位槽，到位后再交回真实卡片
    const fl = floatRef.current;
    let slot: HTMLElement | null = null;
    for (const el of Array.from(document.querySelectorAll<HTMLElement>('[data-card-id]'))) {
      if (el.dataset.cardId === d.id) slot = el;
    }
    if (fl && slot) {
      const r = slot.getBoundingClientRect();
      fl.classList.add('dropping');
      fl.style.transform = `translate(${r.left}px, ${r.top}px)`;
      window.setTimeout(finish, 160);
    } else {
      finish();
    }
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
                className={`card-wrap ${draggingId === d.id ? 'card-dragging' : ''} ${
                  pressingId === d.id ? 'card-pressing' : ''
                }`}
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

      {/* 拖拽中的浮空卡：跟随指针，原位置由隐藏的占位卡保留空槽 */}
      {draggingId && (
        <div ref={floatRef} className="drag-float">
          <DeviceCard device={devices.find((x) => x.id === draggingId)!} onRemove={onRemove} />
        </div>
      )}

      <Toasts items={toasts} />
    </div>
  );
}
