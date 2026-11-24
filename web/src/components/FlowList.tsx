import { useEffect, useLayoutEffect, useRef } from 'react';
import { ArrowDown, ArrowUp, Repeat } from 'lucide-react';
import type { FlowSummary } from '../../../shared/protocol.ts';
import { useI18n } from '../i18n.tsx';
import type { Order } from '../lib/useCapture.ts';
import { fmtBytes, fmtDuration, fmtClock, methodTone, statusTone } from '../lib/format.ts';
import { Button } from './ui/button.tsx';
import { cn } from '../lib/utils.ts';

interface Props {
  flows: FlowSummary[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  order: Order;
  onToggleOrder: () => void;
  onReplay?: (id: number) => void;
}

const COL = 'flow-grid';

/**
 * Like a browser devtools request line: the tail of the path matters far more
 * than the head, so we keep the last segment visible and let overflow be clipped
 * on the left instead of the right. `direction: rtl` + `unicode-bidi: plaintext`
 * makes ellipsis land at the start while the text still reads left-to-right.
 */
function PathCell({ path }: { path: string }) {
  const p = path || '/';
  const cut = p.lastIndexOf('/');
  const head = cut > 0 ? p.slice(0, cut + 1) : '';
  const tail = cut > 0 ? p.slice(cut + 1) : p;
  if (!head) return <span className="url-path min-w-0 truncate">{p}</span>;
  return (
    <span className="url-path min-w-0 truncate" title={p}>
      <span className="path-head">{head}</span>
      <span className="path-tail">{tail}</span>
    </span>
  );
}

export function FlowList({ flows, selectedId, onSelect, order, onToggleOrder, onReplay }: Props) {
  const { t } = useI18n();
  const listRef = useRef<HTMLDivElement>(null);
  /** true while the user is parked at the bottom of a chronological list: keep following new flows */
  const stickRef = useRef(true);

  useEffect(() => {
    if (selectedId === null) return;
    const el = listRef.current?.querySelector<HTMLElement>(`[data-flow-id="${selectedId}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  }, [selectedId]);

  // Chronological order puts new flows below the fold, so follow them unless the user scrolled up.
  useLayoutEffect(() => {
    const el = listRef.current;
    if (!el || order !== 'asc' || !stickRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [flows, order]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        className={cn(
          COL,
          'shrink-0 border-b border-border px-3 py-1.5 text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground',
        )}
      >
        <span>{t('flowList.columns.status')}</span>
        <span>{t('flowList.columns.method')}</span>
        <span>{t('flowList.columns.path')}</span>
        <span>{t('flowList.columns.type')}</span>
        <span className="text-right">{t('flowList.columns.size')}</span>
        <button
          type="button"
          className="col-time flex cursor-pointer items-center justify-end gap-0.5 text-[10.5px] font-medium uppercase tracking-wide text-muted-foreground transition-colors hover:text-foreground"
          onClick={onToggleOrder}
          title={t(order === 'asc' ? 'flowList.sortAscTitle' : 'flowList.sortDescTitle')}
          aria-label={t(order === 'asc' ? 'flowList.sortAscAria' : 'flowList.sortDescAria')}
        >
          {t('flowList.columns.time')} {order === 'asc' ? <ArrowUp size={10} /> : <ArrowDown size={10} />}
        </button>
        <span />
      </div>
      <div
        className="flow-list min-h-0 flex-1 overflow-y-auto"
        ref={listRef}
        role="listbox"
        aria-label={t('flowList.listLabel')}
        onScroll={() => {
          const el = listRef.current;
          if (!el) return;
          stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 32;
        }}
      >
        {flows.length === 0 && (
          <div className="flex flex-col items-center gap-1 py-16 text-center">
            <div className="text-3xl text-muted-foreground/50">⌁</div>
            <p className="text-sm text-foreground">{t('flowList.emptyTitle')}</p>
            <p className="text-xs text-muted-foreground">{t('flowList.emptyHint')}</p>
          </div>
        )}
        {flows.map((flow) => {
          const tone = statusTone(flow.status, flow.error);
          return (
            <div
              role="option"
              aria-selected={flow.id === selectedId}
              data-flow-id={flow.id}
              key={flow.id}
              tabIndex={0}
              className={cn(
                COL,
                'flow-row group px-3 py-1.5 text-[12px] outline-none focus-visible:ring-1 focus-visible:ring-ring/50',
                flow.id === selectedId && 'is-selected',
              )}
              onClick={() => onSelect(flow.id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onSelect(flow.id);
                }
              }}
              title={`${flow.method} ${flow.url}`}
            >
              <span className={`col-status status-${tone} font-mono`} title={flow.error ?? flow.statusText}>
                {flow.status ?? (flow.open ? '···' : '—')}
              </span>
              <span className={`col-method method-${methodTone(flow.method)} font-mono font-semibold`}>{flow.method}</span>
              <span className="col-url flex min-w-0 items-baseline gap-1">
                <span className="url-host truncate" title={flow.host}>{flow.host}</span>
                <PathCell path={flow.path} />
              </span>
              <span className="col-kind flex min-w-0 items-center gap-1">
                {flow.kind !== 'http' && <span className={`kind-pill kind-${flow.kind}`}>{flow.kind}</span>}
                {typeof flow.sseCount === 'number' && flow.sseCount > 0 && (
                  <span className="kind-pill kind-sse">{t('flowList.events', { count: flow.sseCount })}</span>
                )}
                {typeof flow.wsCount === 'number' && flow.wsCount > 0 && (
                  <span className="kind-pill kind-ws">{t('flowList.frames', { count: flow.wsCount })}</span>
                )}
              </span>
              <span className="col-size text-right font-mono text-muted-foreground">{fmtBytes(flow.size || undefined)}</span>
              <span className="col-time flex items-center justify-end gap-1 text-right font-mono text-muted-foreground" title={fmtClock(flow.startedAt)}>
                {flow.open ? <span className="pulse" /> : fmtDuration(flow.durationMs)}
              </span>
              <span className="col-actions flex justify-end">
                {onReplay && (
                  <Button
                    size="iconSm"
                    variant="ghost"
                    className="row-replay opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 group-aria-selected:opacity-100"
                    onClick={(event) => {
                      event.stopPropagation();
                      onReplay(flow.id);
                    }}
                    title={t('detail.replay')}
                    aria-label={t('detail.replay')}
                  >
                    <Repeat size={12} />
                  </Button>
                )}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
