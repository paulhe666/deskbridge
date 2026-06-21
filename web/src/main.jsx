import React, { useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import {
  Activity,
  CheckCircle2,
  CircleStop,
  ExternalLink,
  Info,
  Keyboard,
  MonitorCog,
  Play,
  RefreshCw,
  Save,
  Settings,
  Sparkles,
  TerminalSquare,
  Wifi,
  X,
} from 'lucide-react';
import './style.css';

const APP_VERSION = '1.5.7';
const BRAND_ICON_DATA = 'data:image/webp;base64,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';
const AUTHOR = 'paulhe666';
const REPO_URL = 'https://github.com/paulhe666/deskbridge';
const RELEASES_URL = `${REPO_URL}/releases`;

const mappingOptions = [
  ['control', 'Control / Ctrl'],
  ['meta', 'Meta / Command / Win'],
  ['alt', 'Alt / Option'],
  ['disabled', 'Disabled'],
];

const advancedKeys = ['Shift', 'CapsLock', 'Esc', 'Backspace', 'Delete', 'Arrow keys'];

const copy = {
  en: {
    subtitle: 'Clean device sharing powered by React and Rust',
    stopped: 'Service stopped',
    running: 'Service running',
    loading: 'Loading',
    connection: 'Connection',
    runtime: 'Runtime',
    logs: 'Logs',
    mode: 'Mode',
    client: 'Client',
    server: 'Server',
    serverAddress: 'Server address',
    bindAddress: 'Bind address',
    edge: 'Remote device position',
    left: 'Remote on left',
    right: 'Remote on right',
    language: 'Language',
    commandPreview: 'Command preview',
    start: 'Start',
    stop: 'Stop',
    save: 'Save',
    refresh: 'Refresh',
    clear: 'Clear',
    noLogs: 'No logs yet.',
    settings: 'Settings',
    keyboard: 'Keyboard Mapping',
    about: 'About / Update',
    activeMappings: 'Active mappings',
    plannedKeys: 'Additional keys planned for future mapping',
    plannedNote: 'Shown here as a roadmap. These keys need protocol/router support before they can be saved safely.',
    version: 'Version',
    author: 'Maintainer',
    repo: 'Repository',
    checkUpdates: 'Check updates',
    manualUpdate: 'Manual update',
    autoUpdate: 'Auto update',
    autoUpdateText: 'Auto update is planned. Current builds open the release page for manual update.',
    updateHint: 'Open GitHub Releases to compare and download the latest installer.',
  },
  zh: {
    subtitle: '基于 React 与 Rust 的简洁设备共享工具',
    stopped: '服务已停止',
    running: '服务运行中',
    loading: '加载中',
    connection: '连接',
    runtime: '运行',
    logs: '日志',
    mode: '模式',
    client: '客户端',
    server: '服务端',
    serverAddress: '服务端地址',
    bindAddress: '监听地址',
    edge: '远端设备位置',
    left: '远端在左侧',
    right: '远端在右侧',
    language: '语言',
    commandPreview: '命令预览',
    start: '启动',
    stop: '停止',
    save: '保存',
    refresh: '刷新',
    clear: '清空',
    noLogs: '暂无日志。',
    settings: '设置',
    keyboard: '键盘映射',
    about: '关于 / 更新',
    activeMappings: '当前生效映射',
    plannedKeys: '后续计划支持的更多键位',
    plannedNote: '这些键位先作为路线图展示，真正生效前还需要协议和 KeyboardRouter 支持。',
    version: '版本号',
    author: '维护者',
    repo: '仓库',
    checkUpdates: '检查更新',
    manualUpdate: '手动更新',
    autoUpdate: '自动更新',
    autoUpdateText: '自动更新功能后续加入。当前版本会打开 Release 页面手动更新。',
    updateHint: '打开 GitHub Releases，对比并下载最新版安装包。',
  },
};

function BrandMark() {
  return (
    <svg viewBox="0 0 128 128" role="img" aria-hidden="true" className="brand-svg">
      <defs>
        <linearGradient id="brand-main" x1="22" y1="14" x2="106" y2="116" gradientUnits="userSpaceOnUse">
          <stop stopColor="#84f6c9" />
          <stop offset="0.48" stopColor="#28d9dc" />
          <stop offset="1" stopColor="#1678f2" />
        </linearGradient>
        <linearGradient id="brand-highlight" x1="30" y1="16" x2="96" y2="100" gradientUnits="userSpaceOnUse">
          <stop stopColor="#ecfff7" stopOpacity="0.96" />
          <stop offset="1" stopColor="#c8f6ff" stopOpacity="0.32" />
        </linearGradient>
      </defs>
      <path className="brand-shadow" d="M36 18H77C98 18 111 30 111 48C111 61 104 70 93 74C108 78 118 88 118 104C118 124 103 137 78 137H36V18Z" />
      <path d="M30 15H76C97 15 110 27 110 46C110 60 102 69 90 73C106 77 116 88 116 103C116 123 101 135 77 135H30V15Z" fill="none" stroke="url(#brand-main)" strokeWidth="11" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M43 31H74C87 31 95 38 95 49C95 60 87 67 74 67H43V31Z" fill="none" stroke="url(#brand-highlight)" strokeWidth="7" strokeLinejoin="round" />
      <path d="M43 83H78C92 83 100 90 100 102C100 114 91 121 77 121H43V83Z" fill="none" stroke="url(#brand-highlight)" strokeWidth="7" strokeLinejoin="round" />
      <path d="M44 66H84" stroke="url(#brand-main)" strokeWidth="9" strokeLinecap="round" />
      <path d="M47 52H79" stroke="#d8fff8" strokeWidth="5" strokeLinecap="round" opacity="0.92" />
      <path d="M49 101H79" stroke="#dffbff" strokeWidth="5" strokeLinecap="round" opacity="0.88" />
      <circle cx="64" cy="75" r="3.3" fill="#efffff" />
      <circle cx="76" cy="75" r="3.3" fill="#efffff" />
      <circle cx="88" cy="75" r="3.3" fill="#efffff" />
    </svg>
  );
}

function App() {
  const [state, setState] = useState(null);
  const [config, setConfig] = useState(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState('keyboard');

  async function command(name, args = {}) {
    setError('');
    const payload = await invoke(name, args);
    setState(payload);
    setConfig(payload.config);
    return payload;
  }

  async function refresh() {
    try {
      await command('get_state');
    } catch (err) {
      setError(String(err));
    }
  }

  async function save(nextConfig = config) {
    if (!nextConfig) return;
    setBusy(true);
    try {
      await command('save_config', { config: nextConfig });
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function action(name) {
    setBusy(true);
    try {
      await command(name);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, 1200);
    return () => clearInterval(timer);
  }, []);

  const commandPreview = state?.commandPreview || 'deskbridge';
  const logs = state?.logs || [];
  const running = Boolean(state?.running);
  const role = config?.role || 'client';
  const t = copy[config?.language === 'zh' ? 'zh' : 'en'];

  const statusText = useMemo(() => {
    if (!state) return t.loading;
    return running ? t.running : t.stopped;
  }, [state, running, t]);

  function patchConfig(patch) {
    setConfig((current) => ({ ...current, ...patch }));
  }

  function patchAndSave(patch) {
    setConfig((current) => {
      const next = { ...current, ...patch };
      save(next);
      return next;
    });
  }

  return (
    <main className="app-shell">
      <section className="hero-card">
        <div className="brand-row">
          <div className="app-mark image-mark" aria-label="Deskbridge icon">
            <BrandMark />
          </div>
          <div>
            <h1>Deskbridge</h1>
            <p>{t.subtitle}</p>
          </div>
        </div>
        <div className="hero-actions">
          <div className={`status-pill ${running ? 'online' : 'offline'}`}>
            <Activity size={16} />
            {statusText}
          </div>
          <button className="icon-button" title={t.settings} onClick={() => setSettingsOpen(true)}>
            <Settings size={19} />
          </button>
        </div>
      </section>

      {error && <div className="error-banner">{error}</div>}

      <section className="grid-layout">
        <Panel title={t.connection} icon={<Wifi size={18} />}>
          {!config ? (
            <Skeleton />
          ) : (
            <div className="form-grid">
              <label>
                {t.mode}
                <select value={role} onChange={(event) => patchConfig({ role: event.target.value })}>
                  <option value="client">{t.client}</option>
                  <option value="server">{t.server}</option>
                </select>
              </label>

              {role === 'client' ? (
                <label className="wide">
                  {t.serverAddress}
                  <input
                    value={config.server}
                    placeholder="192.168.1.10:24920"
                    onChange={(event) => patchConfig({ server: event.target.value })}
                  />
                </label>
              ) : (
                <>
                  <label className="wide">
                    {t.bindAddress}
                    <input
                      value={config.bind}
                      placeholder="0.0.0.0:24920"
                      onChange={(event) => patchConfig({ bind: event.target.value })}
                    />
                  </label>
                  <label>
                    {t.edge}
                    <select value={config.edge} onChange={(event) => patchConfig({ edge: event.target.value })}>
                      <option value="left">{t.left}</option>
                      <option value="right">{t.right}</option>
                    </select>
                  </label>
                </>
              )}

              <label>
                {t.language}
                <select value={config.language} onChange={(event) => patchAndSave({ language: event.target.value })}>
                  <option value="zh">中文</option>
                  <option value="en">English</option>
                </select>
              </label>
            </div>
          )}
        </Panel>

        <Panel title={t.runtime} icon={<MonitorCog size={18} />}>
          <div className="command-card">
            <span>{t.commandPreview}</span>
            <code>{commandPreview}</code>
          </div>
          <div className="button-row">
            <button className="primary" disabled={busy || running} onClick={() => action('start_service')}>
              <Play size={16} /> {t.start}
            </button>
            <button disabled={busy || !running} onClick={() => action('stop_service')}>
              <CircleStop size={16} /> {t.stop}
            </button>
            <button disabled={busy} onClick={() => save()}>
              <Save size={16} /> {t.save}
            </button>
            <button disabled={busy} onClick={refresh}>
              <RefreshCw size={16} /> {t.refresh}
            </button>
          </div>
        </Panel>


        <Panel className="logs-panel" title={t.logs} icon={<TerminalSquare size={18} />} action={
          <button className="ghost" onClick={() => action('clear_logs')}>{t.clear}</button>
        }>
          <div className="logs-box">
            {logs.length ? logs.map((line, index) => <div key={`${index}-${line}`}>{line}</div>) : <span className="muted">{t.noLogs}</span>}
          </div>
        </Panel>
      </section>

      {settingsOpen && (
        <SettingsModal
          t={t}
          tab={settingsTab}
          setTab={setSettingsTab}
          config={config}
          patchConfig={patchConfig}
          save={save}
          close={() => setSettingsOpen(false)}
        />
      )}
    </main>
  );
}

function SettingsModal({ t, tab, setTab, config, patchConfig, save, close }) {
  return (
    <div className="modal-backdrop" onClick={close}>
      <section className="settings-modal" onClick={(event) => event.stopPropagation()}>
        <header className="modal-header">
          <div>
            <p className="eyebrow">Deskbridge</p>
            <h2>{t.settings}</h2>
          </div>
          <button className="icon-button light" onClick={close}><X size={18} /></button>
        </header>

        <div className="tabs">
          <button className={tab === 'keyboard' ? 'active' : ''} onClick={() => setTab('keyboard')}>
            <Keyboard size={16} /> {t.keyboard}
          </button>
          <button className={tab === 'about' ? 'active' : ''} onClick={() => setTab('about')}>
            <Info size={16} /> {t.about}
          </button>
        </div>

        {tab === 'keyboard' ? (
          <div className="settings-body">
            <h3>{t.activeMappings}</h3>
            {config ? (
              <div className="form-grid compact">
                <MappingSelect label="Command" value={config.macCommandMapping} onChange={(value) => patchConfig({ macCommandMapping: value })} />
                <MappingSelect label="Control" value={config.macControlMapping} onChange={(value) => patchConfig({ macControlMapping: value })} />
                <MappingSelect label="Option" value={config.macOptionMapping} onChange={(value) => patchConfig({ macOptionMapping: value })} />
              </div>
            ) : <Skeleton />}
            <div className="planned-box">
              <h3>{t.plannedKeys}</h3>
              <p>{t.plannedNote}</p>
              <div className="key-grid">
                {advancedKeys.map((key) => <span key={key}>{key}</span>)}
              </div>
            </div>
            <button className="primary" onClick={() => save()}><Save size={16} /> {t.save}</button>
          </div>
        ) : (
          <div className="settings-body about-grid">
            <InfoRow label={t.version} value={APP_VERSION} />
            <InfoRow label={t.author} value={AUTHOR} />
            <InfoRow label={t.repo} value="paulhe666/deskbridge" />
            <div className="update-card">
              <div>
                <h3>{t.checkUpdates}</h3>
                <p>{t.updateHint}</p>
              </div>
              <a className="primary link-button" href={RELEASES_URL} target="_blank" rel="noreferrer">
                <ExternalLink size={16} /> {t.manualUpdate} <ExternalLink size={14} />
              </a>
            </div>
            <div className="update-card soft">
              <CheckCircle2 size={18} />
              <div>
                <h3>{t.autoUpdate}</h3>
                <p>{t.autoUpdateText}</p>
              </div>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}

function InfoRow({ label, value }) {
  return (
    <div className="info-row">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function Panel({ title, icon, children, action, className = '' }) {
  return (
    <section className={`panel ${className}`}>
      <header>
        <div className="panel-title">{icon}{title}</div>
        {action}
      </header>
      {children}
    </section>
  );
}

function Range({ label, value, min, max, step, onChange }) {
  return (
    <label className="range-row">
      <span>{label}</span>
      <input type="range" min={min} max={max} step={step} value={value} onChange={(event) => onChange(Number(event.target.value))} />
      <strong>{value}</strong>
    </label>
  );
}

function MappingSelect({ label, value, onChange }) {
  return (
    <label>
      {label}
      <select value={value} onChange={(event) => onChange(event.target.value)}>
        {mappingOptions.map(([optionValue, optionLabel]) => (
          <option key={optionValue} value={optionValue}>{optionLabel}</option>
        ))}
      </select>
    </label>
  );
}

function Skeleton() {
  return <div className="skeleton">Loading configuration...</div>;
}

createRoot(document.getElementById('root')).render(<App />);
