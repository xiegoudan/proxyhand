import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, Download, FileJson, Terminal } from 'lucide-react';
import type { BodyView, Flow, HeaderPair, SseFrame, WsFrame } from '../../../shared/protocol.ts';
import { useI18n } from '../i18n.tsx';
import {
  copyText,
  fmtBytes,
  fmtDuration,
  fmtClock,
  fmtRelative,
  headerValue,
  methodTone,
  statusTone,
  toCurl,
  tryJson,
} from '../lib/format.ts';
import { JsonTree } from './JsonTree.tsx';

interface Props {
  flow: Flow;
  sse: SseFrame[];
  ws: WsFrame[];
}

type Tab = 'overview' | 'request' | 'response' | 'frames';
const TABS: { id: Tab; key: string }[] = [
  { id: 'overview', key: 'detail.tabs.overview' },
  { id: 'request', key: 'detail.tabs.request' },
  { id: 'response', key: 'detail.tabs.response' },
  { id: 'frames', key: 'detail.tabs.frames' },
];

export function Detail({ flow, sse, ws }: Props) {
  const { t } = useI18n();
  const [tab, setTab] = useState<Tab>('overview');
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    setTab(flow.kind === 'sse' || flow.kind === 'ws' ? 'frames' : 'overview');
  }, [flow.id, flow.kind]);

  const frameCount = sse.length + ws.length;
  const tone = statusTone(flow.status, flow.error);

  const onCopyCurl = async () => {
    const ok = await copyText(toCurl(flow));
    setCopied(ok);
    window.setTimeout(() => setCopied(false), 1400);
  };

  return (
    <section className="detail">
      <header className="detail-head">
        <div className="detail-title">
          <span className={`method-badge method-${methodTone(flow.method)}`}>{flow.method}</span>
          <span className={`status-badge status-${tone}`}>{flow.status ?? t('detail.pending')}</span>
          <span className="detail-url" title={flow.url}>
            {flow.url}
          </span>
        </div>
        <div className="detail-actions">
          <button type="button" className="ghost" onClick={onCopyCurl}>
            <Terminal size={13} /> {copied ? t('detail.copied') : t('detail.copyCurl')}
          </button>
          <a className="ghost" href={`/api/export/har?ids=${flow.id}`} download>
            <FileJson size={13} /> HAR
          </a>
        </div>
        <div className="detail-meta">
          <span>{fmtDuration(flow.durationMs)}</span>
          <span>{fmtBytes(flow.resBytes ?? flow.size)}</span>
          {flow.clientIp && <span>{flow.clientIp}</span>}
          {flow.alpn && <span>ALPN {flow.alpn}</span>}
          {flow.kind !== 'http' && <span className={`kind-pill kind-${flow.kind}`}>{flow.kind}</span>}
          {flow.open && <span className="live-dot">{t('detail.streaming')}</span>}
        </div>
      </header>

      {flow.notes?.length ? (
        <div className="notes">
          {flow.notes.map((note) => (
            <span key={note} className="note">
              {note}
            </span>
          ))}
        </div>
      ) : null}

      <nav className="tabs">
        {TABS.filter((candidate) => candidate.id !== 'frames' || frameCount > 0 || flow.kind === 'sse' || flow.kind === 'ws').map((tabDef) => (
          <button
            type="button"
            key={tabDef.id}
            className={`tab${tab === tabDef.id ? ' is-active' : ''}`}
            onClick={() => setTab(tabDef.id)}
          >
            {t(tabDef.key)}
            {tabDef.id === 'frames' && frameCount > 0 && <span className="tab-count">{frameCount}</span>}
            {tabDef.id === 'request' && flow.reqBody?.size ? (
              <span className="tab-count">{fmtBytes(flow.reqBody.size)}</span>
            ) : null}
            {tabDef.id === 'response' && flow.resBody?.size ? (
              <span className="tab-count">{fmtBytes(flow.resBody.size)}</span>
            ) : null}
          </button>
        ))}
      </nav>

      <div className="detail-body">
        {tab === 'overview' && <Overview flow={flow} />}
        {tab === 'request' && (
          <div className="pane">
            <HeadersTable title={t('detail.requestHeaders')} headers={flow.reqHeaders} tint="req" />
            <BodyBlock label={t('detail.requestBody')} file="request-body" body={flow.reqBody} />
          </div>
        )}
        {tab === 'response' && (
          <div className="pane">
            <HeadersTable title={t('detail.responseHeaders')} headers={flow.resHeaders} tint="res" />
            <BodyBlock label={t('detail.responseBody')} file="response-body" body={flow.resBody} />
          </div>
        )}
        {tab === 'frames' && (
          <div className="pane">
            {sse.length > 0 && <SseView flow={flow} frames={sse} />}
            {ws.length > 0 && <WsView frames={ws} />}
            {sse.length === 0 && ws.length === 0 && (
              <p className="hint">
                {flow.kind === 'sse' || flow.kind === 'ws' ? t('detail.waitingFrames') : t('detail.noFrames')}
              </p>
            )}
          </div>
        )}
      </div>
    </section>
  );
}

function Overview({ flow }: { flow: Flow }) {
  const { t } = useI18n();
  const wait = flow.ttfbMs;
  const total = flow.durationMs ?? 0;
  const rows: { label: string; value: React.ReactNode }[] = [
    { label: t('detail.overview.url'), value: <span className="mono wrap">{flow.url}</span> },
    { label: t('detail.overview.started'), value: fmtClock(flow.startedAt) },
    { label: t('detail.overview.http'), value: `HTTP/${flow.httpVersion}` },
    { label: t('detail.overview.status'), value: `${flow.status ?? '—'} ${flow.statusText ?? ''}`.trim() },
    { label: t('detail.overview.ttfb'), value: fmtDuration(wait) },
    { label: t('detail.overview.duration'), value: fmtDuration(flow.durationMs) },
    { label: t('detail.overview.requestBody'), value: fmtBytes(flow.reqBytes) },
    { label: t('detail.overview.responseBody'), value: fmtBytes(flow.resBytes ?? flow.size) },
    { label: t('detail.overview.client'), value: flow.clientIp ?? '—' },
    {
      label: t('detail.overview.tls'),
      value: flow.scheme === 'https' ? `ALPN ${flow.alpn ?? 'http/1.1'}` : t('detail.overview.plain'),
    },
  ];
  if (flow.error) rows.push({ label: t('detail.overview.error'), value: <span className="danger">{flow.error}</span> });
  return (
    <div className="pane">
      <div className="overview-grid">
        {rows.map((row) => (
          <div className="kv" key={row.label}>
            <span className="kv-label">{row.label}</span>
            <span className="kv-value">{row.value}</span>
          </div>
        ))}
      </div>
      {total > 0 && (
        <div className="timing">
          <div className="timing-label">{t('detail.timing')}</div>
          <div className="timing-bar">
            <div
              className="timing-seg timing-wait"
              style={{ width: `${Math.min(100, ((wait ?? total) / total) * 100)}%` }}
              title={t('detail.timingWait', { duration: fmtDuration(wait) })}
            />
            <div
              className="timing-seg timing-recv"
              title={t('detail.timingReceive', { duration: fmtDuration(total - (wait ?? total)) })}
            />
          </div>
          <div className="timing-legend">
            <span className="dot dot-wait" /> {t('detail.wait')} {fmtDuration(wait)}
            <span className="dot dot-recv" /> {t('detail.receive')} {fmtDuration(Math.max(0, total - (wait ?? total)))}
          </div>
        </div>
      )}
      <HeadersTable title={t('detail.requestHeaders')} headers={flow.reqHeaders} tint="req" />
      <HeadersTable title={t('detail.responseHeaders')} headers={flow.resHeaders} tint="res" />
    </div>
  );
}

function HeadersTable({ title, headers, tint }: { title: string; headers: HeaderPair[]; tint: 'req' | 'res' }) {
  const { t } = useI18n();
  const [filter, setFilter] = useState('');
  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return headers ?? [];
    return (headers ?? []).filter((h) => `${h.name}: ${h.value}`.toLowerCase().includes(q));
  }, [headers, filter]);
  return (
    <div className="headers">
      <div className="section-head">
        <h3>{title}</h3>
        <span className="section-count">{shown.length}</span>
        <input
          className="mini-input"
          placeholder={t('detail.headersFilter')}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          spellCheck={false}
        />
      </div>
      <div className={`header-table tint-${tint}`}>
        {shown.length === 0 && <div className="hint">{t('detail.none')}</div>}
        {shown.map((header, index) => (
          <div className="header-row" key={`${header.name}-${index}`}>
            <span className="header-name">{header.name}</span>
            <span className="header-value mono">{header.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function BodyBlock({ label, file, body }: { label: string; file: string; body: BodyView | undefined }) {
  const { t } = useI18n();
  const [mode, setMode] = useState<'pretty' | 'raw'>('pretty');
  const text = body?.text ?? null;
  const json = useMemo(() => tryJson(text), [text]);
  const download = () => {
    if (!body) return;
    const blob = body.base64
      ? new Blob([Uint8Array.from(atob(body.base64), (c) => c.charCodeAt(0))])
      : new Blob([body.text ?? '']);
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = file;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  if (!body) return null;
  return (
    <div className="body-block">
      <div className="section-head">
        <h3>{label}</h3>
        <span className="section-count">
          {fmtBytes(body.size)}
          {body.decodedSize !== undefined && body.encoding ? ` · ${body.encoding} → ${fmtBytes(body.decodedSize)}` : ''}
        </span>
        {json !== undefined && (
          <div className="segmented">
            <button type="button" className={mode === 'pretty' ? 'is-active' : ''} onClick={() => setMode('pretty')}>
              {t('detail.tree')}
            </button>
            <button type="button" className={mode === 'raw' ? 'is-active' : ''} onClick={() => setMode('raw')}>
              {t('detail.raw')}
            </button>
          </div>
        )}
        <button type="button" className="ghost small" onClick={download}>
          <Download size={12} /> {t('detail.save')}
        </button>
      </div>
      {body.truncated && (
        <div className="note warn">
          {t('detail.truncated')}
          {body.dropped ? t('detail.truncatedDropped', { size: fmtBytes(body.dropped) }) : t('detail.truncatedEnd')}
        </div>
      )}
      {body.base64 ? (
        <div className="binary">
          <span className="binary-icon">▩</span>
          {t('detail.binary')} · {fmtBytes(body.size)} {body.mime ? `· ${body.mime}` : ''}
        </div>
      ) : text === '' ? (
        <div className="hint">{t('detail.emptyBody')}</div>
      ) : json !== undefined && mode === 'pretty' ? (
        <JsonTree data={json} />
      ) : (
        <pre className="code-block">{text}</pre>
      )}
    </div>
  );
}

function SseView({ flow, frames }: { flow: Flow; frames: SseFrame[] }) {
  const { t } = useI18n();
  const [follow, setFollow] = useState(true);
  const [filter, setFilter] = useState('');
  const endRef = useRef<HTMLDivElement>(null);
  const q = filter.trim().toLowerCase();
  const shown = q ? frames.filter((f) => (f.raw ?? '').toLowerCase().includes(q)) : frames;

  useEffect(() => {
    if (follow) endRef.current?.scrollIntoView({ block: 'end' });
  }, [frames.length, follow]);

  return (
    <div className="frames">
      <div className="section-head">
        <h3>
          {t('detail.sseTitle')} <span className="section-count">{frames.length}</span>
        </h3>
        <input
          className="mini-input"
          placeholder={t('detail.filterFrames')}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <label className="check">
          <input type="checkbox" checked={follow} onChange={(e) => setFollow(e.target.checked)} /> {t('detail.follow')}
        </label>
      </div>
      <div className="sse-list">
        {shown.map((frame) => {
          const data = tryJson(frame.data);
          return (
            <div className="sse-frame" key={frame.i}>
              <div className="sse-frame-head">
                <span className="frame-index">#{frame.i}</span>
                <span className="frame-time">{fmtRelative(flow.startedAt, frame.ts)}</span>
                {frame.event && <span className="pill event">event: {frame.event}</span>}
                {frame.id && <span className="pill id">id: {frame.id}</span>}
                {frame.retry !== undefined && <span className="pill">retry: {frame.retry}</span>}
                <span className="frame-bytes">{fmtBytes(frame.bytes)}</span>
              </div>
              {frame.comment && <div className="sse-comment">{frame.comment}</div>}
              {data !== undefined ? (
                <div className="sse-data">
                  <JsonTree data={data} openDepth={3} />
                </div>
              ) : (
                <pre className="sse-data-text">{frame.data ?? ''}</pre>
              )}
            </div>
          );
        })}
        <div ref={endRef} />
      </div>
    </div>
  );
}

function WsView({ frames }: { frames: WsFrame[] }) {
  const { t } = useI18n();
  const [filter, setFilter] = useState('');
  const [follow, setFollow] = useState(true);
  const endRef = useRef<HTMLDivElement>(null);
  const q = filter.trim().toLowerCase();
  // Global arrival index: frame.i is per-direction, so interleaved frames would repeat "#1", "#2"…
  const shown = frames
    .map((frame, seq) => ({ frame, seq: seq + 1 }))
    .filter(({ frame }) => (q ? (frame.text ?? '').toLowerCase().includes(q) : true));
  const c2s = frames.filter((f) => f.dir === 'c2s').length;

  useEffect(() => {
    if (follow) endRef.current?.scrollIntoView({ block: 'end' });
  }, [frames.length, follow]);

  return (
    <div className="frames">
      <div className="section-head">
        <h3>
          {t('detail.wsTitle')} <span className="section-count">{frames.length}</span>
        </h3>
        <span className="hint">
          {t('detail.wsSent', { count: c2s })} · {t('detail.wsReceived', { count: frames.length - c2s })}
        </span>
        <input
          className="mini-input"
          placeholder={t('detail.filterFrames')}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <label className="check">
          <input type="checkbox" checked={follow} onChange={(e) => setFollow(e.target.checked)} /> {t('detail.follow')}
        </label>
      </div>
      <div className="ws-list">
        {shown.map(({ frame, seq }) => {
          const json = tryJson(frame.text);
          return (
            <div className={`ws-frame dir-${frame.dir}`} key={`${frame.dir}-${frame.i}`}>
              <div className="ws-gutter">
                <span className="ws-arrow">
                  {frame.dir === 'c2s' ? <ArrowUp size={13} strokeWidth={2.6} /> : <ArrowDown size={13} strokeWidth={2.6} />}
                </span>
                <span className="frame-index">#{seq}</span>
              </div>
              <div className="ws-body">
                <div className="ws-meta">
                  <span className={`pill op-${frame.opcodeName}`}>{frame.opcodeName}</span>
                  <span>{fmtBytes(frame.size)}</span>
                  {!frame.fin && <span className="pill">{t('detail.fragment')}</span>}
                  {frame.truncated && <span className="pill warn">{t('detail.truncatedPill')}</span>}
                </div>
                {json !== undefined ? (
                  <JsonTree data={json} openDepth={2} />
                ) : (
                  <pre className="ws-text">{frame.text ?? t('detail.binaryFrame')}</pre>
                )}
              </div>
            </div>
          );
        })}
        <div ref={endRef} />
      </div>
    </div>
  );
}
