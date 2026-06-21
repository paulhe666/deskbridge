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

const APP_VERSION = '1.5.4';
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
    edge: 'Screen edge',
    left: 'Left',
    right: 'Right',
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
    edge: '屏幕边缘',
    left: '左侧',
    right: '右侧',
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

  return (
    <main className="app-shell">
      <section className="hero-card">
        <div className="brand-row">
          <div className="app-mark">B</div>
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
                <select value={config.language} onChange={(event) => patchConfig({ language: event.target.value })}>
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
