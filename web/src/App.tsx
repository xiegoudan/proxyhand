import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeftRight, Check, Copy, Download, Pause, Play, ShieldCheck, Trash2 } from 'lucide-react';
import type { Flow } from '../../shared/protocol.ts';
import { BrandMark } from './components/Brand.tsx';
import { Detail } from './components/Detail.tsx';
import { FlowList } from './components/FlowList.tsx';
import { ReplayDialog } from './components/ReplayDialog.tsx';
import { Button } from './components/ui/button.tsx';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from './components/ui/dialog.tsx';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './components/ui/select.tsx';
import { LANGS, useI18n } from './i18n.tsx';
import { fmtBytes, copyText } from './lib/format.ts';
import { useCapture, type Filters } from './lib/useCapture.ts';
import { cn } from './lib/utils.ts';

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
  const [composerFlow, setComposerFlow] = useState<Flow | null>(null);

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

  // The composer needs the full record (headers + body); a list click only has the summary.
  const openReplay = async (id: number) => {
    try {
      const res = await fetch(`/api/flows/${id}?lang=${encodeURIComponent(lang)}`);
      if (!res.ok) return;
      const flow = (await res.json()) as Flow;
      setComposerFlow(flow);
    } catch {
      /* keep the list as-is; the row click already selected the flow */
    }
  };

  return (
    <div className="flex h-screen flex-col">
      <header className="flex h-12 shrink-0 items-center gap-3 border-b border-border bg-card px-3">
        <div className="flex items-center gap-2">
          <BrandMark />
          <span className="text-[13.5px] font-semibold">proxyhand</span>
          <span className="rounded-sm bg-secondary px-1 py-px font-mono text-[10.5px] text-secondary-foreground">
            v{stats?.version ?? '0.1.0'}
          </span>
        </div>

        <button
          type="button"
          className="flex cursor-pointer items-center gap-1.5 rounded-md border border-border bg-background px-2.5 py-1 text-[12px] transition-colors hover:border-primary/50"
          onClick={() => void markCopied('proxy', `http://${proxyLabel}`)}
          title={t('header.copyProxyAddress')}
        >
          <span className="text-muted-foreground">{t('header.proxy')}</span>
          <span className="font-mono text-foreground">{proxyLabel}</span>
          <span className="text-muted-foreground">{copied === 'proxy' ? <Check size={12} /> : <Copy size={12} />}</span>
        </button>

        <div className="flex min-w-0 flex-1 items-center gap-2">
          <input
            ref={searchRef}
            className="h-8 w-full max-w-56 rounded-md border border-input bg-card px-2.5 text-[12.5px] text-foreground placeholder:text-muted-foreground/60 focus-visible:border-primary/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
            placeholder={t('header.searchPlaceholder')}
            value={filters.q}
            spellCheck={false}
            onChange={(e) => setFilters({ q: e.target.value })}
          />
          <div className="inline-flex items-center gap-0.5 rounded-md bg-secondary p-0.5">
            {KINDS.map((kind) => (
              <button
                type="button"
                key={kind.id}
                className={cn(
                  'seg-btn',
                  filters.kind === kind.id && 'is-active',
                )}
                onClick={() => setFilters({ kind: kind.id })}
              >
                {t(kind.key)}
              </button>
            ))}
          </div>
          <Select value={filters.status} onValueChange={(value) => setFilters({ status: value as Filters['status'] })}>
            <SelectTrigger className="w-[116px]" aria-label={t('header.statusFilter')}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t('header.statusAny')}</SelectItem>
              <SelectItem value="ok">{t('header.statusSuccess')}</SelectItem>
              <SelectItem value="error">{t('header.statusErrors')}</SelectItem>
              <SelectItem value="pending">{t('header.statusPending')}</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <div className="mr-1 inline-flex items-center gap-0.5 rounded-md bg-secondary p-0.5" role="group" aria-label={t('lang.label')}>
            {LANGS.map((value) => (
              <button
                type="button"
                key={value}
                className={cn('seg-btn', lang === value && 'is-active')}
                onClick={() => setLang(value)}
                aria-pressed={lang === value}
                title={value === 'en' ? t('lang.switchToEn') : t('lang.switchToZh')}
              >
                {value === 'en' ? t('lang.en') : t('lang.zh')}
              </button>
            ))}
          </div>
          <Button
            size="icon"
            variant={stats?.paused ? 'default' : 'ghost'}
            onClick={() => void capture.togglePause()}
            title={t(stats?.paused ? 'header.resumeCapture' : 'header.pauseCapture')}
          >
            {stats?.paused ? <Play size={14} /> : <Pause size={14} />}
          </Button>
          <Button size="icon" variant="ghost" onClick={() => void capture.clear()} title={t('header.clearFlows')}>
            <Trash2 size={14} />
          </Button>
          <a
            className="inline-flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
            href="/api/export/har"
            title={t('header.exportHar')}
          >
            <Download size={14} />
          </a>
          <Button size="icon" variant="ghost" onClick={openTrust} title={t('header.trustInstructions')}>
            <ShieldCheck size={14} />
          </Button>
          <span
            className={cn('ml-1 size-2 rounded-full', capture.connected ? 'bg-[var(--ok)]' : 'bg-muted-foreground/50')}
            title={t(capture.connected ? 'header.live' : 'header.disconnected')}
          />
        </div>
      </header>

      <main className="flex min-h-0 flex-1">
        <div className="flex w-[430px] shrink-0 flex-col border-r border-border bg-card/40">
          <FlowList
            flows={flows}
            selectedId={selectedId}
            onSelect={select}
            order={order}
            onToggleOrder={toggleOrder}
            onReplay={(id) => void openReplay(id)}
          />
        </div>
        <div className="flex min-w-0 flex-1 flex-col">
          {detail ? (
            <Detail
              flow={detail}
              sse={capture.selectedSse}
              ws={capture.selectedWs}
              onReplayed={(id) => {
                capture.refresh();
                capture.select(id);
              }}
              onReplayFlow={setComposerFlow}
            />
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-2 text-center">
              <div className="mb-1 flex size-16 items-center justify-center rounded-2xl border border-border bg-card text-muted-foreground/70">
                <ArrowLeftRight size={30} strokeWidth={1.15} />
              </div>
              <h2 className="text-[15px] font-semibold">{t('empty.title')}</h2>
              <p className="text-[12.5px] text-muted-foreground">
                {t('empty.bodyBefore')}
                <kbd className="rounded border border-border bg-card px-1 py-px font-mono text-[11px]">↑</kbd>{' '}
                <kbd className="rounded border border-border bg-card px-1 py-px font-mono text-[11px]">↓</kbd>
                {t('empty.bodyAfter')}
              </p>
              <div className="mt-3 flex items-center gap-4 text-[12px] text-muted-foreground">
                <span>
                  <b className="text-foreground">{flows.length}</b> {t('empty.flows')}
                </span>
                <span>
                  <b className="text-foreground">{stats?.active ?? 0}</b> {t('empty.live')}
                </span>
                <span className="font-mono">
                  ↑{fmtBytes(stats?.bytesIn)} ↓{fmtBytes(stats?.bytesOut)}
                </span>
                {stats?.paused && <span className="text-red-400">{t('common.capturePaused')}</span>}
              </div>
            </div>
          )}
        </div>
      </main>

      <footer className="flex h-7 shrink-0 items-center gap-2 border-t border-border bg-card px-3 text-[11px] text-muted-foreground">
        <span>
          CA <span className="font-mono">{stats?.caPath ?? '~/.proxyhand/ca.crt'}</span>
        </span>
        <span className="text-border">|</span>
        <span>{t('statusbar.flows', { count: flows.length })}</span>
        <span className="text-border">|</span>
        <span>↑ {fmtBytes(stats?.bytesIn)}</span>
        <span>↓ {fmtBytes(stats?.bytesOut)}</span>
        <span className="grow" />
        {stats?.paused ? (
          <span className="text-red-400">{t('common.capturePaused')}</span>
        ) : (
          <span>{t('statusbar.capturing')}</span>
        )}
        <a className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline" href="/api/ca.crt">
          {t('statusbar.downloadCa')}
        </a>
      </footer>

      {composerFlow && (
        <ReplayDialog
          flow={composerFlow}
          onClose={() => setComposerFlow(null)}
          onSent={(id) => {
            setComposerFlow(null);
            capture.refresh();
            capture.select(id);
          }}
        />
      )}

      <Dialog open={trustOpen} onOpenChange={setTrustOpen}>
        <DialogContent className="w-[min(620px,calc(100vw-40px))]">
          <DialogTitle>{t('trust.title')}</DialogTitle>
          <DialogDescription asChild>
            <div className="mt-2 space-y-2 text-[12.5px] text-muted-foreground">
              <p>
                {t('trust.introBefore')}
                <span className="font-mono text-foreground">{trust?.caPath ?? stats?.caPath}</span>
                {t('trust.introAfter')}
              </p>
              <p>
                {t('trust.scopeIntro')}
                <b className="text-foreground">{t('trust.clientToProxy')}</b>
                {t('trust.scopeMid')}
                <span className="font-mono text-foreground">--verify-tls</span>
                {t('trust.scopeMid2')}
                <span className="font-mono text-foreground">--verify-tls --upstream-ca &lt;pem&gt;</span>
                {t('trust.scopeEnd')}
              </p>
            </div>
          </DialogDescription>
          <ol className="mt-3 max-h-64 space-y-1 overflow-y-auto">
            {(trust?.instructions ?? ['$ proxyhand --help']).map((line) => (
              <li key={line} className="rounded-md bg-muted px-3 py-1.5 font-mono text-[11.5px] text-foreground">
                {line}
              </li>
            ))}
          </ol>
          <footer className="mt-4 flex justify-end gap-2 border-t border-border pt-3">
            <Button variant="ghost" onClick={() => void markCopied('ca', trust?.caPath ?? '')}>
              <Copy size={13} /> {t('trust.copyPath')}
            </Button>
            <Button variant="ghost" asChild>
              <a href="/api/ca.crt">
                <Download size={13} /> {t('trust.downloadCert')}
              </a>
            </Button>
          </footer>
        </DialogContent>
      </Dialog>
    </div>
  );
}
