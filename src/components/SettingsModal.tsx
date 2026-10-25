import { useState } from 'react';
import type { AppSettings } from '../types';
import { saveSettings } from '../api';

type Props = {
  settings: AppSettings;
  /** 预览回调（透明度等即时生效，不落盘） */
  onPreview: (s: AppSettings) => void;
  onClose: () => void;
};

export default function SettingsModal({ settings, onPreview, onClose }: Props) {
  const [local, setLocal] = useState<AppSettings>(settings);
  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'error'>('idle');

  /** 立即持久化（开关、按钮类设置） */
  const commit = async (next: AppSettings) => {
    setLocal(next);
    onPreview(next);
    setSaveState('saving');
    try {
      await saveSettings(next);
      setSaveState('idle');
    } catch {
      setSaveState('error');
    }
  };

  /** 仅本地预览（拖动中不落盘） */
  const preview = (next: AppSettings) => {
    setLocal(next);
    onPreview(next);
  };

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div className="settings-panel" onClick={(e) => e.stopPropagation()}>
        <div className="settings-header">
          <span className="settings-title">设置</span>
          <button className="win-btn" title="关闭" onClick={onClose}>✕</button>
        </div>

        <div className="settings-row">
          <div className="settings-label">
            <div className="settings-name">开机自启</div>
          </div>
          <label className="switch">
            <input
              type="checkbox"
              checked={local.autostart}
              onChange={(e) => void commit({ ...local, autostart: e.target.checked })}
            />
            <span className="slider" />
          </label>
        </div>

        <div className="settings-row settings-row-col">
          <div className="settings-label">
            <div className="settings-name">点击关闭按钮时</div>
          </div>
          <div className="segmented">
            <button
              className={`seg-btn ${local.close_action === 'minimize' ? 'seg-active' : ''}`}
              onClick={() => void commit({ ...local, close_action: 'minimize' })}
            >
              最小化到托盘
            </button>
            <button
              className={`seg-btn ${local.close_action === 'quit' ? 'seg-active' : ''}`}
              onClick={() => void commit({ ...local, close_action: 'quit' })}
            >
              退出程序
            </button>
          </div>
        </div>

        <div className="settings-row settings-row-col">
          <div className="settings-label">
            <div className="settings-name">检测间隔</div>
          </div>
          <div className="opacity-row">
            <input
              className="opacity-slider"
              type="range"
              min={2}
              max={60}
              step={1}
              value={local.poll_interval_secs}
              onChange={(e) => preview({ ...local, poll_interval_secs: Number(e.target.value) })}
              onPointerUp={() => void commit(local)}
              onKeyUp={() => void commit(local)}
            />
            <span className="opacity-value">{local.poll_interval_secs} 秒</span>
          </div>
        </div>

        <div className="settings-row settings-row-col">
          <div className="settings-label">
            <div className="settings-name">背景不透明度</div>
          </div>
          <div className="opacity-row">
            <input
              className="opacity-slider"
              type="range"
              min={0.2}
              max={1}
              step={0.05}
              value={local.bg_opacity}
              onChange={(e) => preview({ ...local, bg_opacity: Number(e.target.value) })}
              onPointerUp={() => void commit(local)}
              onKeyUp={() => void commit(local)}
            />
            <span className="opacity-value">{Math.round(local.bg_opacity * 100)}%</span>
          </div>
        </div>

        {saveState !== 'idle' && (
          <div className={`settings-footer ${saveState === 'error' ? 'settings-error' : ''}`}>
            {saveState === 'saving' ? '保存中…' : '保存失败，请重试'}
          </div>
        )}
      </div>
    </div>
  );
}
