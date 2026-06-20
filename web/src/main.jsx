import React, { useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import {
  Activity,
  CircleStop,
  ClipboardList,
  MonitorCog,
  Play,
  RefreshCw,
  Settings,
  TerminalSquare,
  Wifi,
} from 'lucide-react';
import './style.css';

const mappingOptions = [
  ['control', 'Control / Ctrl'],
  ['meta', 'Meta / Command / Win'],
  ['alt', 'Alt / Option'],
  ['disabled', 'Disabled'],
];

function App() {
  const [state, setState] = useState(null);
  const [config, setConfig] = useState(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

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

  const statusText = useMemo(() => {
    if (!state) return 'Loading';
    return running ? 'Service running' : 'Service stopped';
  }, [state, running]);

  function patchConfig(patch) {
    setConfig((current) => ({ ...current, ...patch }));
  }

  return (
    <main className="app-shell">
      <section className="hero-card">
        <div className="brand-row">
          <div className="app-mark">D</div>
          <div>
            <h1>Deskbridge</h1>
            <p>React frontend · Rust backend · Tauri command API</p>
          </div>
        </div>
        <div className={`status-pill ${running ? 'online' : 'offline'}`}>
          <Activity size={16} />
          {statusText}
        </div>
      </section>

      {error && <div className="error-banner">{error}</div>}

      <section className="grid-layout">
        <Panel title="Connection" icon={<Wifi size={18} />}>
          {!config ? (
            <Skeleton />
          ) : (
            <div className="form-grid">
              <label>
                Mode
                <select value={role} onChange={(event) => patchConfig({ role: event.target.value })}>
                  <option value="client">Client</option>
                  <option value="server">Server</option>
                </select>
              </label>

              {role === 'client' ? (
                <label className="wide">
                  Server address
                  <input
                    value={config.server}
                    placeholder="192.168.1.10:24920"
                    onChange={(event) => patchConfig({ server: event.target.value })}
                  />
                </label>
              ) : (
                <>
                  <label className="wide">
                    Bind address
                    <input
                      value={config.bind}
                      placeholder="0.0.0.0:24920"
                      onChange={(event) => patchConfig({ bind: event.target.value })}
                    />
                  </label>
                  <label>
                    Screen edge
                    <select value={config.edge} onChange={(event) => patchConfig({ edge: event.target.value })}>
                      <option value="left">Left</option>
                      <option value="right">Right</option>
                    </select>
                  </label>
                </>
              )}

              <label>
                Language
                <select value={config.language} onChange={(event) => patchConfig({ language: event.target.value })}>
                  <option value="zh">中文</option>
                  <option value="en">English</option>
                </select>
              </label>
            </div>
          )}
        </Panel>

        <Panel title="Runtime" icon={<MonitorCog size={18} />}>
          <div className="command-card">
            <span>Command preview</span>
            <code>{commandPreview}</code>
          </div>
          <div className="button-row">
            <button className="primary" disabled={busy || running} onClick={() => action('start_service')}>
              <Play size={16} /> Start
            </button>
            <button disabled={busy || !running} onClick={() => action('stop_service')}>
              <CircleStop size={16} /> Stop
            </button>
            <button disabled={busy} onClick={() => save()}>
              <Settings size={16} /> Save
            </button>
            <button disabled={busy} onClick={refresh}>
              <RefreshCw size={16} /> Refresh
            </button>
          </div>
        </Panel>

        <Panel title="Scroll tuning" icon={<Activity size={18} />}>
          {config ? (
            <div className="slider-list">
              <Range label="Scale" value={config.scrollScale} min={0.2} max={4} step={0.05} onChange={(value) => patchConfig({ scrollScale: value })} />
              <Range label="Response" value={config.scrollResponse} min={0.05} max={1} step={0.01} onChange={(value) => patchConfig({ scrollResponse: value })} />
              <Range label="Max step" value={config.scrollMaxStep} min={10} max={500} step={5} onChange={(value) => patchConfig({ scrollMaxStep: value })} />
              <Range label="Frame ms" value={config.scrollFrameMs} min={4} max={24} step={1} onChange={(value) => patchConfig({ scrollFrameMs: value })} />
            </div>
          ) : (
            <Skeleton />
          )}
        </Panel>

        <Panel title="macOS modifier mapping" icon={<ClipboardList size={18} />}>
          {config ? (
            <div className="form-grid">
              <MappingSelect label="Command" value={config.macCommandMapping} onChange={(value) => patchConfig({ macCommandMapping: value })} />
              <MappingSelect label="Control" value={config.macControlMapping} onChange={(value) => patchConfig({ macControlMapping: value })} />
              <MappingSelect label="Option" value={config.macOptionMapping} onChange={(value) => patchConfig({ macOptionMapping: value })} />
            </div>
          ) : (
            <Skeleton />
          )}
        </Panel>

        <Panel className="logs-panel" title="Logs" icon={<TerminalSquare size={18} />} action={
          <button className="ghost" onClick={() => action('clear_logs')}>Clear</button>
        }>
          <div className="logs-box">
            {logs.length ? logs.map((line, index) => <div key={`${index}-${line}`}>{line}</div>) : <span className="muted">No logs yet.</span>}
          </div>
        </Panel>
      </section>
    </main>
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
