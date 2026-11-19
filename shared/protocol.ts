// Wire protocol shared by the proxy backend and the web UI.
// Pure types + tiny helpers only: this file is imported by both Node and the browser bundle.

export type FlowKind = 'http' | 'sse' | 'ws' | 'tunnel' | 'ndjson';

export interface HeaderPair {
  name: string;
  value: string;
}

export interface BodyView {
  /** raw (on-the-wire, i.e. possibly compressed) byte length */
  size: number;
  /** decoded byte length when a content-encoding was undone */
  decodedSize?: number;
  mime?: string;
  encoding?: string;
  /** present when the payload is textual and within the capture cap */
  text?: string | null;
  /** base64 body when binary / not representable as text */
  base64?: string | null;
  truncated?: boolean;
  /** number of bytes that were dropped because of the capture cap */
  dropped?: number;
}

export interface SseFrame {
  i: number;
  ts: number;
  /** raw frame text including the blank-line terminator */
  raw: string;
  event?: string;
  id?: string;
  retry?: number;
  data?: string;
  /** ':' comment lines / keep-alive pings */
  comment?: string;
  bytes: number;
}

export interface WsFrame {
  i: number;
  ts: number;
  dir: 'c2s' | 's2c';
  opcode: number;
  opcodeName: string;
  fin: boolean;
  size: number;
  text?: string;
  truncated?: boolean;
}

export interface CertInfo {
  subject?: string;
  issuer?: string;
  validTo?: string;
  fingerprint?: string;
}

export interface FlowSummary {
  id: number;
  seq: number;
  scheme: 'http' | 'https';
  method: string;
  url: string;
  host: string;
  port: number;
  path: string;
  status: number | null;
  statusText?: string;
  kind: FlowKind;
  contentType?: string;
  size: number;
  durationMs?: number;
  startedAt: number;
  endedAt?: number;
  error?: string;
  /** true while the response body is still streaming (SSE / long poll / websocket) */
  open: boolean;
  sseCount?: number;
  wsCount?: number;
  reqBytes?: number;
  resBytes?: number;
}

export interface Flow extends FlowSummary {
  httpVersion: string;
  reqHeaders: HeaderPair[];
  resHeaders: HeaderPair[];
  reqBody?: BodyView;
  resBody?: BodyView;
  sseFrames?: SseFrame[];
  wsFrames?: WsFrame[];
  ttfbMs?: number;
  alpn?: string;
  clientIp?: string;
  cert?: CertInfo | null;
  /** proxy-side notes: tls fallback, tunnel passthrough, decode failures ... */
  notes?: string[];
}

export interface Stats {
  total: number;
  active: number;
  paused: boolean;
  bytesIn: number;
  bytesOut: number;
  startedAt: number;
  version: string;
  proxyPort: number;
  uiPort: number;
  caPath: string;
}

export interface StateResponse extends Stats {
  filters?: unknown;
}

/** Messages pushed over the UI websocket. */
export type ServerEvent =
  | { t: 'hello'; state: Stats }
  | { t: 'flow:new'; flow: FlowSummary }
  | { t: 'flow:update'; flow: FlowSummary }
  | { t: 'sse'; id: number; frame: SseFrame }
  | { t: 'ws'; id: number; frame: WsFrame }
  | { t: 'stats'; stats: Stats }
  | { t: 'cleared' }
  | { t: 'paused'; paused: boolean };

export interface FlowListQuery {
  limit?: number;
  offset?: number;
  /** 'asc' = oldest first (default, chronological); 'desc' = newest first */
  order?: 'asc' | 'desc';
  q?: string;
  kind?: FlowKind | 'all';
  method?: string;
  status?: 'all' | 'ok' | 'error' | 'pending';
}

export interface FlowListResponse {
  total: number;
  flows: FlowSummary[];
}

export type WsClientMessage =
  | { t: 'subscribe'; id: number | null }
  | { t: 'ping' };

export const OPCODE_NAMES: Record<number, string> = {
  0x0: 'continuation',
  0x1: 'text',
  0x2: 'binary',
  0x8: 'close',
  0x9: 'ping',
  0xa: 'pong',
};
