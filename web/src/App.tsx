import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeftRight, Check, Copy, Download, Pause, Play, ShieldCheck, Trash2, X } from 'lucide-react';
import { BrandMark } from './components/Brand.tsx';
import { Detail } from './components/Detail.tsx';
import { FlowList } from './components/FlowList.tsx';
import { LANGS, useI18n } from './i18n.tsx';
import { fmtBytes, copyText } from './lib/format.ts';
import { useCapture, type Filters } from './lib/useCapture.ts';

const KINDS: { id: Filters['kind']; key: string }[] = [
  { id: 'all', key: 'kinds.all' },
  { id: 'http', key: 'kinds.http' },
  { id: 'sse', key: 'kinds.sse' },
  { id: 'ws', key: 'kinds.ws' },
];

export function App() {
  const capture = useCapture();
  const { lang, setLang, t } = useI18n();
  const { flows, filters, setFilters, selectedId, select, detail, stats, order, toggleOrder } = capture;
  const searchRef = useRef<HTMLInputElement>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [trustOpen, setTrustOpen] = useState(false);
  const [trust, setTrust] = useState<{ caPath: string; instructions: string[] } | null>(null);

  const proxyLabel = useMemo(() => {
    const port = stats?.proxyPort ?? 7747;
    return `127.0.0.1:${port}`;
  }, [stats?.proxyPort]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const typing = target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA');
      if (event.key === '/' && !typing) {
        event.preventDefault();
        searchRef.current?.focus();
        return;
      }
      if (event.key === 'Escape' && !typing) {
        select(null);
        return;
      }
      if (event.key === 'o' && !typing && !event.metaKey && !event.ctrlKey && !event.altKey) {
        // same switch as clicking the "Time" column header
        toggleOrder();
        return;
      }
      if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && !typing && flows.length) {
        event.preventDefault();
        const index = flows.findIndex((f) => f.id === selectedId);
        const next = event.key === 'ArrowDown' ? Math.min(flows.length - 1, index + 1) : Math.max(0, index - 1);
        select(flows[next].id);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [flows, selectedId, select, toggleOrder]);

  // Trust instructions are rendered server-side and localized: refetch when the dialog opens or
  // the language changes while it is open.
  useEffect(() => {
    if (!trustOpen) return;
    let cancelled = false;
    fetch(`/api/trust?lang=${encodeURIComponent(lang)}`)
      .then((res) => (res.ok ? (res.json() as Promise<{ caPath: string; instructions: string[] }>) : null))
      .then((data) => {
        if (!cancelled && data) setTrust(data);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [trustOpen, lang]);

  const markCopied = async (what: string, text: string) => {
    if (await copyText(text)) {
      setCopied(what);
      window.setTimeout(() => setCopied(null), 1200);
    }
  };

  const openTrust = () => setTrustOpen(true);

  return (
    <div className="app">
      <header className="toolbar">
        <div className="brand">
          <BrandMark />
          <span className="brand-name">proxyhand</span>
          <span className="brand-version">v{stats?.version ?? '0.1.0'}</span>
        </div>

        <button
          type="button"
          className="endpoint"
          onClick={() => void markCopied('proxy', `http://${proxyLabel}`)}
          title={t('header.copyProxyAddress')}
        >
          <span className="endpoint-label">{t('header.proxy')}</span>
          <span className="endpoint-value">{proxyLabel}</span>
          <span className="endpoint-copy">{copied === 'proxy' ? <Check size={12} /> : <Copy size={12} />}</span>
        </button>

        <div className="filters">
          <input
            ref={searchRef}
            className="search"
            placeholder={t('header.searchPlaceholder')}
            value={filters.q}
            spellCheck={false}
            onChange={(e) => setFilters({ q: e.target.value })}
          />
          <div className="segmented">
            {KINDS.map((kind) => (
              <button
                type="button"
                key={kind.id}
                className={filters.kind === kind.id ? 'is-active' : ''}
                onClick={() => setFilters({ kind: kind.id })}
              >
                {t(kind.key)}
              </button>
            ))}
          </div>
          <select
            className="select"
            value={filters.status}
            onChange={(e) => setFilters({ status: e.target.value as Filters['status'] })}
            aria-label={t('header.statusFilter')}
          >
            <option value="all">{t('header.statusAny')}</option>
            <option value="ok">{t('header.statusSuccess')}</option>
            <option value="error">{t('header.statusErrors')}</option>
            <option value="pending">{t('header.statusPending')}</option>
          </select>
        </div>

        <div className="toolbar-actions">
          <div className="segmented lang-switch" role="group" aria-label={t('lang.label')}>
            {LANGS.map((value) => (
              <button
                type="button"
                key={value}
                className={lang === value ? 'is-active' : ''}
                onClick={() => setLang(value)}
                aria-pressed={lang === value}
                title={value === 'en' ? t('lang.switchToEn') : t('lang.switchToZh')}
              >
                {value === 'en' ? t('lang.en') : t('lang.zh')}
              </button>
            ))}
          </div>
          <button
            type="button"
            className={`icon-btn${stats?.paused ? ' is-on' : ''}`}
            onClick={() => void capture.togglePause()}
            title={t(stats?.paused ? 'header.resumeCapture' : 'header.pauseCapture')}
          >
            {stats?.paused ? <Play size={14} /> : <Pause size={14} />}
          </button>
          <button type="button" className="icon-btn" onClick={() => void capture.clear()} title={t('header.clearFlows')}>
            <Trash2 size={14} />
          </button>
          <a className="icon-btn" href="/api/export/har" title={t('header.exportHar')}>
            <Download size={14} />
          </a>
          <button
            type="button"
            className="icon-btn"
            onClick={() => openTrust()}
            title={t('header.trustInstructions')}
          >
            <ShieldCheck size={14} />
          </button>
          <span
            className={`conn${capture.connected ? ' is-up' : ''}`}
            title={t(capture.connected ? 'header.live' : 'header.disconnected')}
          />
        </div>
      </header>

      <main className="layout">
        <FlowList flows={flows} selectedId={selectedId} onSelect={select} order={order} onToggleOrder={toggleOrder} />
        <div className="detail-slot">
          {detail ? (
            <Detail flow={detail} sse={capture.selectedSse} ws={capture.selectedWs} />
          ) : (
            <div className="detail-placeholder">
              <div className="placeholder-art">
                <ArrowLeftRight size={44} strokeWidth={1.15} />
              </div>
              <h2>{t('empty.title')}</h2>
              <p>
                {t('empty.bodyBefore')}
                <kbd>↑</kbd> <kbd>↓</kbd>
                {t('empty.bodyAfter')}
              </p>
              <div className="placeholder-stats">
                <span><b>{flows.length}</b> {t('empty.flows')}</span>
                <span><b>{stats?.active ?? 0}</b> {t('empty.live')}</span>
                <span>↑{fmtBytes(stats?.bytesIn)} ↓{fmtBytes(stats?.bytesOut)}</span>
                {stats?.paused && <span className="warn">{t('common.capturePaused')}</span>}
              </div>
            </div>
          )}
        </div>
      </main>

      <footer className="statusbar">
        <span>
          CA <span className="mono">{stats?.caPath ?? '~/.proxyhand/ca.crt'}</span>
        </span>
        <span className="statusbar-sep" />
        <span>{t('statusbar.flows', { count: flows.length })}</span>
        <span className="statusbar-sep" />
        <span>↑ {fmtBytes(stats?.bytesIn)}</span>
        <span>↓ {fmtBytes(stats?.bytesOut)}</span>
        <span className="grow" />
        {stats?.paused ? <span className="warn">{t('common.capturePaused')}</span> : <span>{t('statusbar.capturing')}</span>}
        <a className="statusbar-link" href="/api/ca.crt">
          {t('statusbar.downloadCa')}
        </a>
      </footer>

      {trustOpen && (
        <div className="modal-backdrop" onClick={() => setTrustOpen(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <header className="modal-head">
              <h2>{t('trust.title')}</h2>
              <button type="button" className="icon-btn" onClick={() => setTrustOpen(false)} aria-label={t('trust.close')}>
                <X size={15} />
              </button>
            </header>
            <p className="hint">
              {t('trust.introBefore')}
              <span className="mono">{trust?.caPath ?? stats?.caPath}</span>
              {t('trust.introAfter')}
            </p>
            <p className="hint">
              {t('trust.scopeIntro')}
              <b>{t('trust.clientToProxy')}</b>
              {t('trust.scopeMid')}
              <span className="mono">--verify-tls</span>
              {t('trust.scopeMid2')}
              <span className="mono">--verify-tls --upstream-ca &lt;pem&gt;</span>
              {t('trust.scopeEnd')}
            </p>
            <ol className="trust-list">
              {(trust?.instructions ?? ['$ proxyhand --help']).map((line) => (
                <li key={line}>
                  <code>{line}</code>
                </li>
              ))}
            </ol>
            {trust && (
              <div className="modal-actions">
                <button type="button" className="ghost" onClick={() => void markCopied('ca', trust.caPath)}>
                  <Copy size={13} /> {t('trust.copyPath')}
                </button>
                <a className="ghost" href="/api/ca.crt">
                  <Download size={13} /> {t('trust.downloadCert')}
                </a>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
