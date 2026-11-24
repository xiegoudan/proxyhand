import { useEffect, useLayoutEffect, useRef } from 'react';
import { ArrowDown, ArrowUp, Repeat } from 'lucide-react';
import type { FlowSummary } from '../../../shared/protocol.ts';
import { useI18n } from '../i18n.tsx';
import type { Order } from '../lib/useCapture.ts';
import { fmtBytes, fmtDuration, fmtClock, methodTone, statusTone } from '../lib/format.ts';

interface Props {
  flows: FlowSummary[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  order: Order;
  onToggleOrder: () => void;
  onReplay?: (id: number) => void;
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
    <div className="flow-list-wrap">
      <div className="flow-head">
        <span className="col-status">{t('flowList.columns.status')}</span>
        <span className="col-method">{t('flowList.columns.method')}</span>
        <span className="col-url">{t('flowList.columns.path')}</span>
        <span className="col-kind">{t('flowList.columns.type')}</span>
        <span className="col-size">{t('flowList.columns.size')}</span>
        <button
          type="button"
          className="col-time sort-btn"
          onClick={onToggleOrder}
          title={t(order === 'asc' ? 'flowList.sortAscTitle' : 'flowList.sortDescTitle')}
          aria-label={t(order === 'asc' ? 'flowList.sortAscAria' : 'flowList.sortDescAria')}
        >
          {t('flowList.columns.time')} {order === 'asc' ? <ArrowUp size={10} /> : <ArrowDown size={10} />}
        </button>
        <span className="col-actions" />
      </div>
      <div
        className="flow-list"
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
          <div className="flow-empty">
            <div className="flow-empty-icon">⌁</div>
            <p>{t('flowList.emptyTitle')}</p>
            <p className="hint">{t('flowList.emptyHint')}</p>
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
              className={`flow-row${flow.id === selectedId ? ' is-selected' : ''}${flow.open ? ' is-open' : ''}`}
              onClick={() => onSelect(flow.id)}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  onSelect(flow.id);
                }
              }}
              title={`${flow.method} ${flow.url}`}
            >
              <span className={`col-status status-${tone}`} title={flow.error ?? flow.statusText}>
                {flow.status ?? (flow.open ? '···' : '—')}
              </span>
              <span className={`col-method method-${methodTone(flow.method)}`}>{flow.method}</span>
              <span className="col-url">
                <span className="url-host">{flow.host}</span>
                <span className="url-path">{flow.path || '/'}</span>
              </span>
              <span className="col-kind">
                {flow.kind !== 'http' && <span className={`kind-pill kind-${flow.kind}`}>{flow.kind}</span>}
                {typeof flow.sseCount === 'number' && flow.sseCount > 0 && (
                  <span className="kind-pill kind-sse">{t('flowList.events', { count: flow.sseCount })}</span>
                )}
                {typeof flow.wsCount === 'number' && flow.wsCount > 0 && (
                  <span className="kind-pill kind-ws">{t('flowList.frames', { count: flow.wsCount })}</span>
                )}
              </span>
              <span className="col-size">{fmtBytes(flow.size || undefined)}</span>
              <span className="col-time" title={fmtClock(flow.startedAt)}>
                {flow.open ? <span className="pulse" /> : fmtDuration(flow.durationMs)}
              </span>
              <span className="col-actions">
                {onReplay && (
                  <button
                    type="button"
                    className="icon-btn tiny row-replay"
                    onClick={(event) => {
                      event.stopPropagation();
                      onReplay(flow.id);
                    }}
                    title={t('detail.replay')}
                    aria-label={t('detail.replay')}
                  >
                    <Repeat size={12} />
                  </button>
                )}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
}
