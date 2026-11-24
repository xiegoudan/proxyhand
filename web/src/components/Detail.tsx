import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, Copy, Download, FileJson, Repeat, Terminal } from 'lucide-react';
import type { BodyView, Flow, HeaderPair, SseFrame, WsFrame } from '../../../shared/protocol.ts';
import { useI18n } from '../i18n.tsx';
import {
  copyText,
  fmtBytes,
  fmtDuration,
  fmtClock,
  fmtRelative,
  methodTone,
  statusTone,
  toCurl,
  tryJson,
} from '../lib/format.ts';
import { JsonTree } from './JsonTree.tsx';
import { FormTable, parseForm } from './FormView.tsx';
import { Badge } from './ui/badge.tsx';
import { Button, buttonVariants } from './ui/button.tsx';
import { Checkbox } from './ui/checkbox.tsx';
import { Input } from './ui/input.tsx';
import { cn } from '../lib/utils.ts';

interface Props {
  flow: Flow;
  sse: SseFrame[];
  ws: WsFrame[];
  onReplayed: (id: number) => void;
  onReplayFlow: (flow: Flow) => void;
}

type Tab = 'overview' | 'request' | 'response' | 'frames';
const TABS: { id: Tab; key: string }[] = [
  { id: 'overview', key: 'detail.tabs.overview' },
  { id: 'request', key: 'detail.tabs.request' },
  { id: 'response', key: 'detail.tabs.response' },
  { id: 'frames', key: 'detail.tabs.frames' },
];

/** Pick the most useful default tab for a flow: streams → frames, otherwise the response. */
function defaultTab(flow: Flow): Tab {
  if (flow.kind === 'sse' || flow.kind === 'ws') return 'frames';
  return 'response';
}

export function Detail({ flow, sse, ws, onReplayed, onReplayFlow }: Props) {
  const { t } = useI18n();
  const [tab, setTab] = useState<Tab>(() => defaultTab(flow));
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    setTab(defaultTab(flow));
  }, [flow.id, flow.kind]);

  const frameCount = sse.length + ws.length;
  const tone = statusTone(flow.status, flow.error);

  const onCopyCurl = async () => {
    const ok = await copyText(toCurl(flow));
    setCopied(ok);
    window.setTimeout(() => setCopied(false), 1400);
  };

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <header className="shrink-0 space-y-1.5 border-b border-border px-4 py-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className={`font-mono text-[12px] font-bold uppercase method-${methodTone(flow.method)}`}>{flow.method}</span>
          <span className={`status-${tone} font-mono text-[12px] font-semibold`}>{flow.status ?? t('detail.pending')}</span>
          <span className="min-w-0 truncate font-mono text-[12.5px] text-foreground" title={flow.url}>
            {flow.url}
          </span>
          <div className="ml-auto flex shrink-0 items-center gap-1">
            <Button size="sm" variant="ghost" onClick={() => onReplayFlow(flow)}>
              <Repeat size={13} /> {t('detail.replay')}
            </Button>
            <Button size="sm" variant="ghost" onClick={onCopyCurl}>
              <Terminal size={13} /> {copied ? t('detail.copied') : t('detail.copyCurl')}
            </Button>
            <a className={buttonVariants({ size: 'sm', variant: 'ghost' })} href={`/api/export/har?ids=${flow.id}`} download>
              <FileJson size={13} /> HAR
            </a>
          </div>
        </div>
        <div className="flex items-center gap-3 text-[11px] text-muted-foreground">
          <span className="font-mono">{fmtDuration(flow.durationMs)}</span>
          <span>{fmtBytes(flow.resBytes ?? flow.size)}</span>
          {flow.clientIp && <span>{flow.clientIp}</span>}
          {flow.alpn && <span>ALPN {flow.alpn}</span>}
          {flow.kind !== 'http' && <span className={`kind-pill kind-${flow.kind}`}>{flow.kind}</span>}
          {flow.open && (
            <span className="flex items-center gap-1.5">
              <span className="pulse" /> {t('detail.streaming')}
            </span>
          )}
        </div>
      </header>

      {flow.notes?.length ? (
        <div className="flex flex-wrap gap-1.5 border-b border-border px-4 py-2">
          {flow.notes.map((note) => (
            <Badge key={note} variant="redirect">
              {note}
            </Badge>
          ))}
        </div>
      ) : null}

      <nav className="shrink-0 px-4 pt-2">
        <div className="inline-flex items-center gap-0.5 rounded-lg border border-border bg-card p-0.5">
          {TABS.filter((candidate) => candidate.id !== 'frames' || frameCount > 0 || flow.kind === 'sse' || flow.kind === 'ws').map(
            (tabDef) => (
              <button
                type="button"
                key={tabDef.id}
                className={cn(
                  'inline-flex cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md px-2.5 py-1 text-[12.5px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40',
                  tab === tabDef.id ? 'bg-accent text-accent-foreground' : 'text-muted-foreground hover:text-foreground',
                )}
                onClick={() => setTab(tabDef.id)}
              >
                {t(tabDef.key)}
                {tabDef.id === 'frames' && frameCount > 0 && (
                  <Badge variant="secondary" className="px-1 py-px text-[10px]">
                    {frameCount}
                  </Badge>
                )}
                {tabDef.id === 'request' && flow.reqBody?.size ? (
                  <Badge variant="secondary" className="px-1 py-px text-[10px]">
                    {fmtBytes(flow.reqBody.size)}
                  </Badge>
                ) : null}
                {tabDef.id === 'response' && flow.resBody?.size ? (
                  <Badge variant="secondary" className="px-1 py-px text-[10px]">
                    {fmtBytes(flow.resBody.size)}
                  </Badge>
                ) : null}
              </button>
            ),
          )}
        </div>
      </nav>

      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {tab === 'overview' && <Overview flow={flow} />}
        {tab === 'request' && (
          <div className="space-y-4">
            <HeadersTable title={t('detail.requestHeaders')} headers={flow.reqHeaders} tint="req" />
            <BodyBlock label={t('detail.requestBody')} file="request-body" body={flow.reqBody} />
          </div>
        )}
        {tab === 'response' && (
          <div className="space-y-4">
            <HeadersTable title={t('detail.responseHeaders')} headers={flow.resHeaders} tint="res" />
            <BodyBlock label={t('detail.responseBody')} file="response-body" body={flow.resBody} />
          </div>
        )}
        {tab === 'frames' && (
          <div className="space-y-4">
            {sse.length > 0 && <SseView flow={flow} frames={sse} />}
            {ws.length > 0 && <WsView frames={ws} />}
            {sse.length === 0 && ws.length === 0 && (
              <p className="text-xs text-muted-foreground">
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
  const recv = Math.max(0, total - (wait ?? total));

  return (
    <div className="ov">
      {/* One line that matters most: METHOD url + status + quick metrics. */}
      <div className="ov-summary">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
          <span className={cn('font-mono text-[12px] font-bold uppercase', `method-${methodTone(flow.method)}`)}>
            {flow.method}
          </span>
          <span className={cn('font-mono text-[12px] font-semibold', `status-${statusTone(flow.status, flow.error)}`)}>
            {flow.status ?? t('detail.pending')}
          </span>
          {flow.statusText && <span className="text-[11px] text-muted-foreground">{flow.statusText}</span>}
          <span className="min-w-0 truncate font-mono text-[12px] text-foreground" title={flow.url}>
            {flow.url}
          </span>
        </div>
        <div className="ov-metrics">
          <span title={t('detail.overview.ttfb')}>
            <span className="ov-metric-label">TTFB</span> {fmtDuration(wait)}
          </span>
          <span title={t('detail.overview.duration')}>
            <span className="ov-metric-label">{t('detail.overview.duration')}</span> {fmtDuration(flow.durationMs)}
          </span>
          <span title={t('detail.overview.responseBody')}>
            <span className="ov-metric-label">{t('detail.overview.responseBody')}</span> {fmtBytes(flow.resBytes ?? flow.size)}
          </span>
        </div>
      </div>

      {flow.error && (
        <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 font-mono text-[11.5px] text-destructive">
          {flow.error}
        </div>
      )}

      {/* Timing bar — thin, with the split labelled inline. */}
      {total > 0 && (
        <div className="ov-block">
          <div className="ov-block-head">{t('detail.timing')}</div>
          <div className="timing-bar">
            <div
              className="timing-wait h-full"
              style={{ width: `${Math.min(100, ((wait ?? total) / total) * 100)}%` }}
              title={t('detail.timingWait', { duration: fmtDuration(wait) })}
            />
            <div
              className="timing-recv h-full flex-1"
              title={t('detail.timingReceive', { duration: fmtDuration(recv) })}
            />
          </div>
          <div className="mt-1.5 flex items-center gap-4 text-[11px] text-muted-foreground">
            <span>
              <span className="dot dot-wait" /> {t('detail.wait')} {fmtDuration(wait)}
            </span>
            <span>
              <span className="dot dot-recv" /> {t('detail.receive')} {fmtDuration(recv)}
            </span>
          </div>
        </div>
      )}

      {/* Compact stat grid — the facts that don't belong in the header. */}
      <div className="stat-grid">
        <StatTile label={t('detail.overview.started')} value={fmtClock(flow.startedAt)} mono />
        <StatTile label={t('detail.overview.http')} value={`HTTP/${flow.httpVersion}`} mono />
        <StatTile label={t('detail.overview.requestBody')} value={fmtBytes(flow.reqBytes)} mono />
        <StatTile label={t('detail.overview.client')} value={flow.clientIp ?? '—'} mono />
        <StatTile
          label={t('detail.overview.tls')}
          value={flow.scheme === 'https' ? `ALPN ${flow.alpn ?? 'http/1.1'}` : t('detail.overview.plain')}
          mono
        />
        {flow.kind !== 'http' && <StatTile label={t('detail.overview.status')} value={flow.kind} mono />}
      </div>

      {/* Headers side by side, devtools' "Headers" panel energy but 2 columns. */}
      <div className="ov-headers">
        <HeadersTable title={t('detail.requestHeaders')} headers={flow.reqHeaders} tint="req" />
        <HeadersTable title={t('detail.responseHeaders')} headers={flow.resHeaders} tint="res" />
      </div>
    </div>
  );
}

function StatTile({ label, value, mono }: { label: string; value: React.ReactNode; mono?: boolean }) {
  return (
    <div className="stat-tile">
      <span className="stat-label">{label}</span>
      <span className={cn('stat-value', mono && 'font-mono')}>{value}</span>
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
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <h3 className="text-[12px] font-semibold text-foreground">{title}</h3>
        <Badge variant="secondary">{shown.length}</Badge>
        <Input
          className="ml-auto h-6 w-48 text-[11px]"
          placeholder={t('detail.headersFilter')}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          spellCheck={false}
        />
      </div>
      <div className="header-table">
        {shown.length === 0 && <div className="px-3 py-4 text-center text-xs text-muted-foreground">{t('detail.none')}</div>}
        {shown.map((header, index) => (
          <div className="header-row" key={`${header.name}-${index}`}>
            <span className="header-name">{header.name}</span>
            <span className="header-value">{header.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

// Some APIs return the image as a text payload that is ALREADY a data URI
// (e.g. captcha endpoints answering `data:image/jpg;base64,/9j/...` with a
// misleading `Content-Type: image/jpeg`). Pull the first usable data:image URI
// out of such a text, whether it is the whole body, a quoted string, or a value
// nested inside a JSON object.
function dataImageFrom(text: string | null): string | null {
  if (!text) return null;
  const normalize = (uri: string): string | null => {
    const u = uri.trim().replace(/^['"]|['"]$/g, '');
    if (!/^data:image\//i.test(u)) return null;
    return u.replace(/^data:image\/jpe?g;/i, 'data:image/jpeg;').replace(/^data:image\/pjpeg;/i, 'data:image/jpeg;');
  };
  const straight = normalize(text);
  if (straight) return straight;
  const parsed = tryJson(text);
  if (parsed && typeof parsed === 'object') {
    const found: string[] = [];
    const walk = (v: unknown): void => {
      if (typeof v === 'string') found.push(v);
      else if (Array.isArray(v)) v.forEach(walk);
      else if (v && typeof v === 'object') Object.values(v as Record<string, unknown>).forEach(walk);
    };
    walk(parsed);
    for (const candidate of found) {
      const uri = normalize(candidate);
      if (uri) return uri;
    }
  }
  // Fallback: an image URI embedded in a longer plain-text payload.
  const base64 = text.match(/data:image\/[a-z0-9.+-]+(?:;[a-z0-9.+-]+=[^;,"'\s]+)*;base64,[a-z0-9+/=]+/i);
  if (base64) return normalize(base64[0]);
  const svg = text.match(/data:image\/svg\+xml(?:;charset=utf-8)?,[^"'\s<]+/i);
  if (svg) return normalize(svg[0]);
  return null;
}

function BodyBlock({ label, file, body }: { label: string; file: string; body: BodyView | undefined }) {
  const { t } = useI18n();
  const [mode, setMode] = useState<'pretty' | 'form' | 'raw'>('pretty');
  const [copied, setCopied] = useState(false);
  const text = body?.text ?? null;
  const json = useMemo(() => tryJson(text), [text]);
  const form = useMemo(() => parseForm(body?.mime, text), [body?.mime, text]);
  // An image body is shown as the image itself, not as a pile of base64. SVG arrives
  // as text (it is XML), everything else arrives as base64. But a "binary" body whose
  // bytes actually spell a `data:image/...` URI (and a text body that IS such a URI)
  // is shown directly from that URI — the bytes are not the image yet.
  const imageSrc = useMemo(() => {
    const mime = (body?.mime ?? '').toLowerCase();
    if (body?.base64) {
      let decodedText: string | null = null;
      try {
        const bin = atob(body.base64);
        if (/^\s*data:/i.test(bin)) decodedText = bin;
      } catch {
        decodedText = null;
      }
      if (decodedText) {
        const uri = dataImageFrom(decodedText);
        if (uri) return uri;
      }
      if (mime.startsWith('image/')) return 'data:' + mime + ';base64,' + body.base64;
      return null;
    }
    if (body?.text) {
      const uri = dataImageFrom(body.text);
      if (uri) return uri;
      if (mime.startsWith('image/')) return 'data:' + mime + ';utf8,' + encodeURIComponent(body.text);
    }
    return null;
  }, [body]);
  const copy = async () => {
    if (!body) return;
    // A binary body has no text to copy, so the base64 payload is what the clipboard gets.
    const ok = await copyText(text ?? body.base64 ?? '');
    setCopied(ok);
    window.setTimeout(() => setCopied(false), 1400);
  };
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
    <div className="space-y-1.5">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-[12px] font-semibold">{label}</h3>
        <span className="text-[11px] text-muted-foreground">
          {fmtBytes(body.size)}
          {body.decodedSize !== undefined && body.encoding ? ` · ${body.encoding} → ${fmtBytes(body.decodedSize)}` : ''}
        </span>
        {(json !== undefined || form !== undefined) && (
          <div className="inline-flex items-center gap-0.5 rounded-md bg-secondary p-0.5">
            {json !== undefined && (
              <button
                type="button"
                className={cn('seg-btn', mode === 'pretty' && 'is-active')}
                onClick={() => setMode('pretty')}
              >
                {t('detail.tree')}
              </button>
            )}
            {form !== undefined && (
              <button type="button" className={cn('seg-btn', mode === 'form' && 'is-active')} onClick={() => setMode('form')}>
                {t('detail.form')}
              </button>
            )}
            <button type="button" className={cn('seg-btn', mode === 'raw' && 'is-active')} onClick={() => setMode('raw')}>
              {t('detail.raw')}
            </button>
          </div>
        )}
        <div className="ml-auto flex items-center gap-1">
          <Button size="sm" variant="ghost" onClick={copy}>
            <Copy size={12} /> {copied ? t('detail.copied') : t('detail.copy')}
          </Button>
          <Button size="sm" variant="ghost" onClick={download}>
            <Download size={12} /> {t('detail.save')}
          </Button>
        </div>
      </div>
      {body.truncated && (
        <Badge variant="redirect">
          {t('detail.truncated')}
          {body.dropped ? t('detail.truncatedDropped', { size: fmtBytes(body.dropped) }) : t('detail.truncatedEnd')}
        </Badge>
      )}
      {imageSrc ? (
        <div className="image-preview">
          <img src={imageSrc} alt={label} />
          <div className="mt-1 text-[11px] text-muted-foreground">
            {body.mime} · {fmtBytes(body.size)}
            {body.decodedSize !== undefined && body.encoding
              ? ` · ${body.encoding} → ${fmtBytes(body.decodedSize)}`
              : ''}
          </div>
        </div>
      ) : body.base64 ? (
        <div className="flex items-center gap-2 rounded-lg border border-border bg-card px-3 py-4 text-[12px] text-muted-foreground">
          <span className="binary-icon">▩</span>
          {t('detail.binary')} · {fmtBytes(body.size)} {body.mime ? `· ${body.mime}` : ''}
        </div>
      ) : text === '' ? (
        <div className="px-3 py-6 text-center text-xs text-muted-foreground">{t('detail.emptyBody')}</div>
      ) : form !== undefined && (mode === 'form' || (json === undefined && mode === 'pretty')) ? (
        <FormTable fields={form.fields} />
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
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <h3 className="text-[12px] font-semibold">
          {t('detail.sseTitle')} <Badge variant="secondary">{frames.length}</Badge>
        </h3>
        <Input
          className="ml-auto h-6 w-48 text-[11px]"
          placeholder={t('detail.filterFrames')}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-muted-foreground">
          <Checkbox checked={follow} onChange={(e) => setFollow(e.target.checked)} /> {t('detail.follow')}
        </label>
      </div>
      <div className="max-h-[560px] space-y-2 overflow-y-auto pr-1">
        {shown.map((frame) => {
          const data = tryJson(frame.data);
          return (
            <div className="sse-frame" key={frame.i}>
              <div className="mb-1 flex items-center gap-2">
                <span className="frame-index">#{frame.i}</span>
                <span className="frame-time">{fmtRelative(flow.startedAt, frame.ts)}</span>
                {frame.event && <span className="pill">event: {frame.event}</span>}
                {frame.id && <span className="pill">id: {frame.id}</span>}
                {frame.retry !== undefined && <span className="pill">retry: {frame.retry}</span>}
                <span className="frame-bytes ml-auto">{fmtBytes(frame.bytes)}</span>
              </div>
              {frame.comment && <div className="sse-comment">{frame.comment}</div>}
              {data !== undefined ? <JsonTree data={data} /> : <pre className="code-block max-h-64">{frame.data ?? ''}</pre>}
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
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <h3 className="text-[12px] font-semibold">
          {t('detail.wsTitle')} <Badge variant="secondary">{frames.length}</Badge>
        </h3>
        <span className="text-[11px] text-muted-foreground">
          {t('detail.wsSent', { count: c2s })} · {t('detail.wsReceived', { count: frames.length - c2s })}
        </span>
        <Input
          className="ml-auto h-6 w-48 text-[11px]"
          placeholder={t('detail.filterFrames')}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-muted-foreground">
          <Checkbox checked={follow} onChange={(e) => setFollow(e.target.checked)} /> {t('detail.follow')}
        </label>
      </div>
      <div className="max-h-[560px] space-y-2 overflow-y-auto pr-1">
        {shown.map(({ frame, seq }) => {
          const json = tryJson(frame.text);
          return (
            <div className={`ws-frame dir-${frame.dir}`} key={`${frame.dir}-${frame.i}`}>
              <div className="flex gap-3">
                <div className="ws-gutter">
                  <span className="ws-arrow">
                    {frame.dir === 'c2s' ? <ArrowUp size={13} strokeWidth={2.6} /> : <ArrowDown size={13} strokeWidth={2.6} />}
                  </span>
                  <span className="frame-index">#{seq}</span>
                </div>
                <div className="min-w-0 flex-1">
                  <div className="mb-1 flex items-center gap-2">
                    <span className={`pill op-${frame.opcodeName}`}>{frame.opcodeName}</span>
                    <span className="text-[11px] text-muted-foreground">{fmtBytes(frame.size)}</span>
                    {!frame.fin && <span className="pill">{t('detail.fragment')}</span>}
                    {frame.truncated && <span className="pill warn">{t('detail.truncatedPill')}</span>}
                  </div>
                  {json !== undefined ? (
                    <JsonTree data={json} />
                  ) : (
                    <pre className="code-block max-h-64">{frame.text ?? t('detail.binaryFrame')}</pre>
                  )}
                </div>
              </div>
            </div>
          );
        })}
        <div ref={endRef} />
      </div>
    </div>
  );
}
