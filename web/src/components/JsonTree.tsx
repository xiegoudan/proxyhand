import { useMemo, useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { useI18n } from '../i18n.tsx';

interface Props {
  data: unknown;
}

/** One rendered line of the tree. Collapsed branches simply stop producing lines. */
type Line =
  | { kind: 'open'; depth: number; path: string; key?: string; bracket: '[' | '{'; count: number; open: boolean; last: boolean }
  | { kind: 'close'; depth: number; bracket: ']' | '}'; last: boolean }
  | { kind: 'leaf'; depth: number; key?: string; value: unknown; last: boolean };

const INDENT = 16;
const MAX_LINES = 5000;

function isContainer(value: unknown): value is Record<string, unknown> | unknown[] {
  return value !== null && typeof value === 'object';
}

function entriesOf(value: Record<string, unknown> | unknown[]): [string, unknown][] {
  return Array.isArray(value)
    ? value.map((item, index) => [String(index), item] as [string, unknown])
    : Object.entries(value);
}

export function JsonTree({ data }: Props) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const lines = useMemo(() => flatten(data, collapsed), [data, collapsed]);

  const toggle = (path: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });

  const shown = lines.length > MAX_LINES ? lines.slice(0, MAX_LINES) : lines;
  return (
    <div className="json">
      {shown.map((line, index) => (
        <JsonLine key={index} line={line} n={index + 1} onToggle={toggle} />
      ))}
      {lines.length > MAX_LINES && <HiddenLines count={lines.length - MAX_LINES} />}
    </div>
  );
}

function HiddenLines({ count }: { count: number }) {
  const { t } = useI18n();
  return <div className="jv-hidden">{t('json.hidden', { count })}</div>;
}

function JsonLine({ line, n, onToggle }: { line: Line; n: number; onToggle: (path: string) => void }) {
  const { t } = useI18n();
  const comma = line.last ? '' : ',';
  const label = line.kind !== 'close' && line.key !== undefined && (
    <>
      <span className="jv-key">"{line.key}"</span>
      <span className="jv-punct">: </span>
    </>
  );

  return (
    <div className="jv-line">
      <span className="jv-ln">{n}</span>
      <span className="jv-text" style={{ paddingLeft: 16 + line.depth * INDENT }}>
        {line.kind === 'open' && (
          <button
            type="button"
            className="jv-caret"
            style={{ left: line.depth * INDENT }}
            onClick={() => onToggle(line.path)}
            aria-label={t('json.toggle')}
            title={t('json.toggle')}
          >
            {line.open ? <ChevronDown size={11} strokeWidth={2.6} /> : <ChevronRight size={11} strokeWidth={2.6} />}
          </button>
        )}
        {line.kind === 'leaf' ? (
          <>
            {label}
            <Value value={line.value} />
            {comma}
          </>
        ) : line.kind === 'close' ? (
          <span className="jv-punct">
            {line.bracket}
            {comma}
          </span>
        ) : (
          <>
            {label}
            <span className="jv-punct">{line.bracket}</span>
            {!line.open && (
              <>
                <span className="jv-summary">{t(line.bracket === '[' ? 'json.items' : 'json.keys', { count: line.count })}</span>
                <span className="jv-punct">
                  {line.bracket === '[' ? ']' : '}'}
                  {comma}
                </span>
              </>
            )}
          </>
        )}
      </span>
    </div>
  );
}

function Value({ value }: { value: unknown }) {
  if (value === null) return <span className="jv-null">null</span>;
  if (typeof value === 'string') return <span className="jv-str">"{value}"</span>;
  if (typeof value === 'number') return <span className="jv-num">{String(value)}</span>;
  if (typeof value === 'boolean') return <span className="jv-bool">{String(value)}</span>;
  if (Array.isArray(value)) return <span className="jv-punct">[]</span>;
  if (typeof value === 'object') return <span className="jv-punct">{'{}'}</span>;
  return <span className="jv-punct">{String(value)}</span>;
}

function flatten(data: unknown, collapsed: ReadonlySet<string>): Line[] {
  const lines: Line[] = [];

  const walk = (value: unknown, depth: number, path: string, key: string | undefined, last: boolean) => {
    if (isContainer(value)) {
      const entries = entriesOf(value);
      if (entries.length === 0) {
        lines.push({ kind: 'leaf', depth, key, value, last });
        return;
      }
      const bracket = Array.isArray(value) ? '[' : '{';
      const open = !collapsed.has(path);
      lines.push({ kind: 'open', depth, path, key, bracket, count: entries.length, open, last });
      if (open) {
        entries.forEach(([childKey, child], index) =>
          walk(child, depth + 1, `${path}.${childKey}`, Array.isArray(value) ? undefined : childKey, index === entries.length - 1),
        );
        lines.push({ kind: 'close', depth, bracket: bracket === '[' ? ']' : '}', last });
      }
      return;
    }
    lines.push({ kind: 'leaf', depth, key, value, last });
  };

  walk(data, 0, '$', undefined, true);
  return lines;
}
