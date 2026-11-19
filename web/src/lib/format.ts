import type { Flow, HeaderPair } from '../../../shared/protocol.ts';

export function fmtBytes(n: number | undefined): string {
  if (n === undefined || n === null) return '—';
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export function fmtDuration(ms: number | undefined): string {
  if (ms === undefined) return '—';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}

export function fmtClock(ts: number): string {
  const d = new Date(ts);
  const pad = (v: number, w = 2) => String(v).padStart(w, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`;
}

export function fmtRelative(from: number, to: number): string {
  const delta = Math.max(0, to - from);
  return delta < 1000 ? `+${Math.round(delta)}ms` : `+${(delta / 1000).toFixed(2)}s`;
}

export function statusTone(status: number | null, error?: string): 'ok' | 'redirect' | 'client' | 'server' | 'pending' {
  if (error) return 'server';
  if (status === null) return 'pending';
  if (status < 300) return 'ok';
  if (status < 400) return 'redirect';
  if (status < 500) return 'client';
  return 'server';
}

export function methodTone(method: string): string {
  switch (method.toUpperCase()) {
    case 'GET':
      return 'get';
    case 'POST':
      return 'post';
    case 'PUT':
    case 'PATCH':
      return 'put';
    case 'DELETE':
      return 'del';
    case 'CONNECT':
      return 'connect';
    default:
      return 'other';
  }
}

export function headerValue(headers: HeaderPair[] | undefined, name: string): string | undefined {
  const lower = name.toLowerCase();
  return headers?.find((h) => h.name.toLowerCase() === lower)?.value;
}

export function tryJson(text: string | null | undefined): unknown | undefined {
  if (!text) return undefined;
  const trimmed = text.trim();
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return undefined;
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return undefined;
  }
}

export function isJsonLike(flow: Flow, body: 'req' | 'res'): boolean {
  const target = body === 'req' ? flow.reqBody : flow.resBody;
  const mime = target?.mime ?? '';
  if (/json/.test(mime)) return true;
  return tryJson(target?.text) !== undefined;
}

const SHELL_UNSAFE = /[^\w@%+=:,./-]/;

function shellQuote(value: string): string {
  if (value === '') return "''";
  if (!SHELL_UNSAFE.test(value)) return value;
  return `'${value.replace(/'/g, `'\\''`)}'`;
}

export function toCurl(flow: Flow): string {
  const parts = [`curl -X ${flow.method.toUpperCase()} ${shellQuote(flow.url)}`];
  const skip = new Set(['host', 'connection', 'content-length', 'accept-encoding', 'user-agent', 'proxy-connection']);
  for (const header of flow.reqHeaders ?? []) {
    if (skip.has(header.name.toLowerCase())) continue;
    parts.push(`  -H ${shellQuote(`${header.name}: ${header.value}`)}`);
  }
  if (flow.reqBody?.text) parts.push(`  --data-raw ${shellQuote(flow.reqBody.text)}`);
  else if (flow.reqBody?.base64) parts.push(`  --data-binary @- ${shellQuote(`# ${flow.reqBody.size} bytes, see captured flow`)}`);
  const host = flow.host;
  if (flow.scheme === 'https' && /(^|\.)localhost$|^127\./.test(host.split(':')[0])) parts.push('  --insecure');
  return parts.join(' \\\n');
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement('textarea');
    area.value = text;
    area.style.position = 'fixed';
    area.style.opacity = '0';
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand('copy');
    area.remove();
    return ok;
  }
}
