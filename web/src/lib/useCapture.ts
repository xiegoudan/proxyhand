import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useI18n } from '../i18n.tsx';
import type {
  Flow,
  FlowListResponse,
  FlowSummary,
  ServerEvent,
  SseFrame,
  Stats,
  WsFrame,
} from '../../../shared/protocol.ts';

export interface Filters {
  q: string;
  kind: FlowKind;
  method: string;
  status: StatusFilter;
}

type FlowKind = 'all' | 'http' | 'sse' | 'ws' | 'tunnel' | 'ndjson';
type StatusFilter = 'all' | 'ok' | 'error' | 'pending';

export const emptyFilters: Filters = { q: '', kind: 'all', method: 'ALL', status: 'all' };

/**
 * Row order of the flow list. 'desc' = devtools-style, newest on top (the default, and what people
 * expect when watching a request stream); 'asc' = chronological, newest at the bottom, like a
 * terminal log. Remembered across reloads.
 */
export type Order = 'asc' | 'desc';

const ORDER_KEY = 'proxyhand:order';

function readOrder(): Order {
  try {
    // Newest-on-top unless the user explicitly flipped to chronological.
    return localStorage.getItem(ORDER_KEY) === 'asc' ? 'asc' : 'desc';
  } catch {
    return 'desc';
  }
}

function writeOrder(order: Order): void {
  try {
    localStorage.setItem(ORDER_KEY, order);
  } catch {
    /* private mode: just do not persist */
  }
}

/** Whichever copy of a flow was touched last — a live ws event beats an older list snapshot. */
function touched(flow: FlowSummary): number {
  return flow.endedAt ?? flow.startedAt;
}

function sortChronological(flows: FlowSummary[]): FlowSummary[] {
  return flows.slice().sort((a, b) => a.startedAt - b.startedAt || a.id - b.id).slice(-MAX_ROWS);
}

function matches(flow: FlowSummary, filters: Filters): boolean {
  if (filters.kind !== 'all' && flow.kind !== filters.kind) return false;
  if (filters.method !== 'ALL' && flow.method.toUpperCase() !== filters.method.toUpperCase()) return false;
  if (filters.status === 'ok' && !(flow.status !== null && flow.status < 400)) return false;
  if (filters.status === 'error' && !(flow.error || (flow.status !== null && flow.status >= 400))) return false;
  if (filters.status === 'pending' && !flow.open) return false;
  const q = filters.q.trim().toLowerCase();
  if (q && !`${flow.host}${flow.path}${flow.method}`.toLowerCase().includes(q)) return false;
  return true;
}

export interface CaptureState {
  connected: boolean;
  stats: Stats | null;
  flows: FlowSummary[];
  total: number;
  filters: Filters;
  setFilters: (next: Partial<Filters>) => void;
  /** row order of `flows`; 'desc' (default) keeps the newest flow on top */
  order: Order;
  setOrder: (next: Order) => void;
  toggleOrder: () => void;
  selectedId: number | null;
  select: (id: number | null) => void;
  detail: Flow | null;
  selectedSse: SseFrame[];
  selectedWs: WsFrame[];
  togglePause: () => Promise<void>;
  clear: () => Promise<void>;
  refresh: () => void;
}

const MAX_ROWS = 2000;

export function useCapture(): CaptureState {
  const { lang } = useI18n();
  const [connected, setConnected] = useState(false);
  const [stats, setStats] = useState<Stats | null>(null);
  const [rows, setRows] = useState<FlowSummary[]>([]);
  const [order, setOrderState] = useState<Order>(readOrder);
  const [filters, setFiltersState] = useState<Filters>(emptyFilters);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<Flow | null>(null);
  const [frames, setFrames] = useState<{ id: number; sse: SseFrame[]; ws: WsFrame[] }>({ id: 0, sse: [], ws: [] });
  const socketRef = useRef<WebSocket | null>(null);
  const filtersRef = useRef(filters);
  filtersRef.current = filters;
  const selectedRef = useRef<number | null>(null);
  selectedRef.current = selectedId;
  const pendingSends = useRef<string[]>([]);

  /** `send` on a CONNECTING socket throws InvalidStateError and would kill the whole tree: queue instead. */
  const sendControl = useCallback((payload: unknown) => {
    const text = JSON.stringify(payload);
    const ws = socketRef.current;
    if (!ws || ws.readyState === WebSocket.CLOSING || ws.readyState === WebSocket.CLOSED) return;
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(text);
      return;
    }
    pendingSends.current = [...pendingSends.current.slice(-8), text];
  }, []);

  const listSeq = useRef(0);

  const fetchList = useCallback(async (active: Filters, replace = false): Promise<FlowSummary[] | null> => {
    const seq = ++listSeq.current;
    const params = new URLSearchParams({ limit: String(MAX_ROWS) });
    if (active.q.trim()) params.set('q', active.q.trim());
    if (active.kind !== 'all') params.set('kind', active.kind);
    if (active.method !== 'ALL') params.set('method', active.method);
    if (active.status !== 'all') params.set('status', active.status);
    try {
      const res = await fetch(`/api/flows?${params.toString()}`);
      if (!res.ok) return null;
      const data = (await res.json()) as FlowListResponse;
      // A newer request (filter typed while this one was in flight) wins; drop the stale answer.
      if (seq !== listSeq.current) return null;
      // Merge instead of replace: flows that arrived over the socket while the request was in
      // flight are newer than the snapshot and must not be dropped. After a clear the snapshot is
      // the whole truth, so nothing from the old list may survive (open streams included).
      setRows((prev) => {
        const byId = new Map<number, FlowSummary>();
        for (const flow of data.flows) if (matches(flow, active)) byId.set(flow.id, flow);
        if (!replace) {
          for (const row of prev) {
            if (!matches(row, active)) continue;
            const old = byId.get(row.id);
            if (!old || touched(row) >= touched(old)) byId.set(row.id, row);
          }
        }
        return sortChronological([...byId.values()]);
      });
      return data.flows;
    } catch {
      /* keep the previous list */
      return null;
    }
  }, []);

  // A clear removes the finished flows and keeps the ones still streaming, so the list is
  // reloaded rather than blanked; the selection survives when its flow is still alive.
  const afterClear = useCallback(async () => {
    const keep = selectedRef.current;
    const list = await fetchList(filtersRef.current, true);
    if (keep === null || list === null) return;
    if (list.some((flow) => flow.id === keep)) return;
    setSelectedId(null);
    setDetail(null);
    setFrames({ id: 0, sse: [], ws: [] });
  }, [fetchList]);

  const setOrder = useCallback((next: Order) => {
    setOrderState(next);
    writeOrder(next);
  }, []);

  const toggleOrder = useCallback(() => {
    setOrder(order === 'asc' ? 'desc' : 'asc');
  }, [order, setOrder]);

  const flows = useMemo(() => (order === 'desc' ? rows.slice().reverse() : rows), [rows, order]);

  useEffect(() => {
    void fetchList(filters);
  }, [fetchList, filters]);

  const setFilters = useCallback((next: Partial<Filters>) => {
    setFiltersState((prev) => ({ ...prev, ...next }));
  }, []);

  // ---- websocket: live flow list + frames for the selected flow -------------
  useEffect(() => {
    let closed = false;
    let retry: number | undefined;

    const connect = () => {
      const proto = location.protocol === 'https:' ? 'wss' : 'ws';
      const ws = new WebSocket(`${proto}://${location.host}/ws?lang=${encodeURIComponent(lang)}`);
      socketRef.current = ws;
      ws.onopen = () => {
        if (socketRef.current !== ws) return;
        setConnected(true);
        const queued = pendingSends.current;
        pendingSends.current = [];
        for (const text of queued) ws.send(text);
        sendControl({ t: 'select', id: selectedRef.current });
      };
      ws.onclose = () => {
        if (socketRef.current === ws) setConnected(false);
        if (!closed) retry = window.setTimeout(connect, 1200);
      };
      ws.onerror = () => ws.close();
      ws.onmessage = (event) => {
        if (socketRef.current !== ws) return;
        let msg: ServerEvent;
        try {
          msg = JSON.parse(String(event.data)) as ServerEvent;
        } catch {
          return;
        }
        switch (msg.t) {
          case 'hello':
            setStats(msg.state);
            break;
          case 'stats':
            setStats(msg.stats);
            break;
          case 'flow:new':
            if (!matches(msg.flow, filtersRef.current)) return;
            // rows are kept chronological; the renderer reverses them for newest-first display
            setRows((prev) => [...prev, msg.flow].slice(-MAX_ROWS));
            break;
          case 'flow:update':
            setRows((prev) => {
              const idx = prev.findIndex((f) => f.id === msg.flow.id);
              if (idx === -1) return prev;
              const next = prev.slice();
              next[idx] = msg.flow;
              return next;
            });
            setDetail((prev) => (prev && prev.id === msg.flow.id ? { ...prev, ...msg.flow } : prev));
            break;
          case 'sse':
            if (selectedRef.current !== msg.id) return;
            setFrames((prev) =>
              prev.id === msg.id ? { ...prev, sse: [...prev.sse, msg.frame] } : { id: msg.id, sse: [msg.frame], ws: [] },
            );
            setDetail((prev) => (prev && prev.id === msg.id ? addSse(prev, msg.frame) : prev));
            break;
          case 'ws':
            if (selectedRef.current !== msg.id) return;
            setFrames((prev) =>
              prev.id === msg.id ? { ...prev, ws: [...prev.ws, msg.frame] } : { id: msg.id, sse: [], ws: [msg.frame] },
            );
            setDetail((prev) => (prev && prev.id === msg.id ? addWs(prev, msg.frame) : prev));
            break;
          case 'cleared':
            void afterClear();
            break;
          case 'paused':
            setStats((prev) => (prev ? { ...prev, paused: msg.paused } : prev));
            break;
          default:
            break;
        }
      };
    };
    connect();

    return () => {
      closed = true;
      pendingSends.current = [];
      if (retry) window.clearTimeout(retry);
      socketRef.current?.close();
    };
  }, [sendControl, lang, afterClear]);

  // ---- load the selected flow's full record --------------------------------
  useEffect(() => {
    if (selectedId === null) {
      setDetail(null);
      setFrames({ id: 0, sse: [], ws: [] });
      sendControl({ t: 'select', id: null });
      return;
    }
    sendControl({ t: 'select', id: selectedId });
    let cancelled = false;
    fetch(`/api/flows/${selectedId}?lang=${encodeURIComponent(lang)}`)
      .then((res) => (res.ok ? (res.json() as Promise<Flow>) : null))
      .then((flow) => {
        if (cancelled || !flow) return;
        setDetail(flow);
        setFrames({ id: flow.id, sse: flow.sseFrames ?? [], ws: flow.wsFrames ?? [] });
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [selectedId, lang]);

  const select = useCallback((id: number | null) => setSelectedId(id), []);

  const togglePause = useCallback(async () => {
    const res = await fetch('/api/pause', { method: 'POST', body: JSON.stringify({ paused: !stats?.paused }) });
    if (res.ok) {
      const data = (await res.json()) as { paused: boolean };
      setStats((prev) => (prev ? { ...prev, paused: data.paused } : prev));
    }
  }, [stats?.paused]);

  const clear = useCallback(async () => {
    await fetch('/api/flows', { method: 'DELETE' });
    // The server broadcasts 'cleared' too; doing it here keeps the acting tab correct even
    // when the websocket happens to be reconnecting.
    await afterClear();
  }, [afterClear]);

  const refresh = useCallback(() => void fetchList(filtersRef.current), [fetchList]);

  return useMemo(
    () => ({
      connected,
      stats,
      flows,
      total: flows.length,
      filters,
      setFilters,
      order,
      setOrder,
      toggleOrder,
      selectedId,
      select,
      detail,
      selectedSse: frames.sse,
      selectedWs: frames.ws,
      togglePause,
      clear,
      refresh,
    }),
    [
      connected,
      stats,
      flows,
      filters,
      setFilters,
      order,
      setOrder,
      toggleOrder,
      selectedId,
      select,
      detail,
      frames,
      togglePause,
      clear,
      refresh,
    ],
  );
}

function addSse(flow: Flow, frame: SseFrame): Flow {
  return { ...flow, sseFrames: [...(flow.sseFrames ?? []), frame], sseCount: (flow.sseCount ?? 0) + 1 };
}

function addWs(flow: Flow, frame: WsFrame): Flow {
  return { ...flow, wsFrames: [...(flow.wsFrames ?? []), frame], wsCount: (flow.wsCount ?? 0) + 1 };
}
