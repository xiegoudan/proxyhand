import { useState } from 'react';
import { ChevronDown, ChevronRight } from 'lucide-react';
import { useI18n } from '../i18n.tsx';

interface Props {
  data: unknown;
  /** depth to expand by default */
  openDepth?: number;
}

export function JsonTree({ data, openDepth = 2 }: Props) {
  return (
    <div className="json">
      <JsonNode value={data} depth={0} openDepth={openDepth} keyName={undefined} last />
    </div>
  );
}

interface NodeProps {
  value: unknown;
  depth: number;
  openDepth: number;
  keyName: string | undefined;
  last: boolean;
}

function JsonNode({ value, depth, openDepth, keyName, last }: NodeProps) {
  const { t } = useI18n();
  const [open, setOpen] = useState(depth < openDepth);
  const comma = last ? '' : ',';
  const label = keyName !== undefined ? <span className="j-key">"{keyName}"</span> : null;

  if (value === null) {
    return (
      <div className="j-row">
        {label}
        {label && <span className="j-colon">: </span>}
        <span className="j-null">null</span>
        {comma}
      </div>
    );
  }
  if (typeof value === 'string') {
    return (
      <div className="j-row">
        {label}
        {label && <span className="j-colon">: </span>}
        <span className="j-str">"{value}"</span>
        {comma}
      </div>
    );
  }
  if (typeof value === 'number') {
    return (
      <div className="j-row">
        {label}
        {label && <span className="j-colon">: </span>}
        <span className="j-num">{String(value)}</span>
        {comma}
      </div>
    );
  }
  if (typeof value === 'boolean') {
    return (
      <div className="j-row">
        {label}
        {label && <span className="j-colon">: </span>}
        <span className="j-bool">{String(value)}</span>
        {comma}
      </div>
    );
  }

  const isArray = Array.isArray(value);
  const entries = isArray
    ? (value as unknown[]).map((item, index) => [String(index), item] as const)
    : Object.entries(value as Record<string, unknown>);
  const brackets = isArray ? ['[', ']'] : ['{', '}'];

  if (entries.length === 0) {
    return (
      <div className="j-row">
        {label}
        {label && <span className="j-colon">: </span>}
        <span className="j-brace">{brackets[0]}</span>
        <span className="j-brace">{brackets[1]}</span>
        {comma}
      </div>
    );
  }

  return (
    <div className="j-node">
      <div className="j-row">
        <button type="button" className="j-toggle" onClick={() => setOpen((v) => !v)} aria-label={t('json.toggle')}>
          {open ? <ChevronDown size={12} strokeWidth={2.4} /> : <ChevronRight size={12} strokeWidth={2.4} />}
        </button>
        {label}
        {label && <span className="j-colon">: </span>}
        <span className="j-brace">{brackets[0]}</span>
        {!open && (
          <>
            <span className="j-summary">{t(isArray ? 'json.items' : 'json.keys', { count: entries.length })}</span>
            <span className="j-brace">{brackets[1]}</span>
            {comma}
          </>
        )}
        {open && <span className="j-brace">{brackets[1]}</span>}
      </div>
      {open && (
        <div className="j-children">
          {entries.map(([key, child], index) => (
            <JsonNode
              key={key}
              value={child}
              depth={depth + 1}
              openDepth={openDepth}
              keyName={isArray ? undefined : key}
              last={index === entries.length - 1}
            />
          ))}
          <div className="j-row">
            <span className="j-brace">{brackets[1]}</span>
            {comma}
          </div>
        </div>
      )}
    </div>
  );
}
