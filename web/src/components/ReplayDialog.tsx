import { useEffect, useMemo, useRef, useState } from 'react';
import { Plus, Send, X } from 'lucide-react';
import type { Flow } from '../../../shared/protocol.ts';
import { useI18n } from '../i18n.tsx';
import { tryJson } from '../lib/format.ts';
import { parseForm, type FormField } from './FormView.tsx';

interface Props {
  flow: Flow;
  onClose: () => void;
  onSent: (id: number) => void;
}

interface HeaderRow {
  name: string;
  value: string;
}

const METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

/** Headers the proxy or the client owns; the URL and the body decide them instead. */
const HIDDEN = new Set([
  'host',
  'content-length',
  'connection',
  'keep-alive',
  'transfer-encoding',
  'proxy-connection',
]);

function headerValue(headers: HeaderRow[], name: string): string | null {
  const hit = headers.find((row) => row.name.trim().toLowerCase() === name.toLowerCase());
  return hit ? hit.value.trim() : null;
}

/** Encode the editable form rows back into an application/x-www-form-urlencoded body. */
function encodeUrlencoded(fields: FormField[]): string {
  return fields
    .filter((field) => field.name !== '')
    .map((field) => `${encodeURIComponent(field.name)}=${encodeURIComponent(field.value ?? '')}`)
    .join('&');
}

export function ReplayDialog({ flow, onClose, onSent }: Props) {
  const { t } = useI18n();
  const initialBody = useMemo(() => {
    const text = flow.reqBody?.text ?? '';
    const json = tryJson(text);
    return json !== undefined && json !== null ? JSON.stringify(json, null, 2) : text;
  }, [flow]);
  const [method, setMethod] = useState(flow.method.toUpperCase());
  const [url, setUrl] = useState(flow.url);
  const [headers, setHeaders] = useState<HeaderRow[]>(() =>
    (flow.reqHeaders ?? [])
      .filter((pair) => !HIDDEN.has(pair.name.toLowerCase()))
      .map((pair) => ({ name: pair.name, value: pair.value })),
  );
  const [body, setBody] = useState(initialBody);
  const [mode, setMode] = useState<'raw' | 'form'>('raw');
  const [formFields, setFormFields] = useState<FormField[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const firstField = useRef<HTMLSelectElement>(null);

  const contentType = headerValue(headers, 'content-type');
  const form = useMemo(() => parseForm(contentType ?? undefined, body), [contentType, body]);
  const isUrlencoded = form?.kind === 'urlencoded';
  const json = useMemo(() => tryJson(body), [body]);

  useEffect(() => {
    firstField.current?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  // Open the form editor for urlencoded payloads; everything else starts raw.
  useEffect(() => {
    if (isUrlencoded && mode === 'raw') {
      setFormFields((form?.fields ?? []).map((field) => ({ name: field.name, value: field.value ?? '' })));
      setMode('form');
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const setHeader = (index: number, patch: Partial<HeaderRow>) =>
    setHeaders((prev) => prev.map((row, i) => (i === index ? { ...row, ...patch } : row)));

  const switchMode = (next: 'raw' | 'form') => {
    if (next === 'form' && !isUrlencoded) return;
    if (next === 'form') {
      setFormFields((form?.fields ?? []).map((field) => ({ name: field.name, value: field.value ?? '' })));
    } else {
      setBody(encodeUrlencoded(formFields));
    }
    setMode(next);
  };

  const send = async () => {
    setSending(true);
    setError(null);
    try {
      const payload: Record<string, unknown> = {
        method,
        url: url.trim(),
        headers: headers
          .filter((row) => row.name.trim() !== '')
          .map((row) => [row.name.trim(), row.value]),
      };
      const isBinary = Boolean(flow.reqBody?.base64);
      if (isBinary) {
        payload.bodyBase64 = flow.reqBody!.base64;
      } else {
        const effectiveBody = mode === 'form' ? encodeUrlencoded(formFields) : body;
        if (effectiveBody !== '') {
          payload.body = effectiveBody;
          // The body editor can re-encode a form, so keep the header truthful.
          if (mode === 'form' && contentType === null) {
            payload.headers = [...(payload.headers as string[][]), ['content-type', 'application/x-www-form-urlencoded']];
          }
        }
      }

      const res = await fetch('/api/replay', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(payload),
      });
      const data = (await res.json().catch(() => ({}))) as { id?: number; error?: string };
      if (!res.ok || typeof data.id !== 'number') {
        setError(data.error ?? `${res.status} ${res.statusText}`);
        return;
      }
      onSent(data.id);
      onClose();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="modal composer" role="dialog" aria-modal="true" aria-label={t('replay.title')}>
        <header className="modal-head">
          <h2>{t('replay.title')}</h2>
          <button type="button" className="icon-btn" onClick={onClose} aria-label={t('detail.close')}>
            <X size={15} />
          </button>
        </header>

        <div className="composer-main">
          <div className="replay-row">
            <select
              ref={firstField}
              className="replay-method"
              value={method}
              onChange={(event) => setMethod(event.target.value)}
              aria-label={t('replay.method')}
            >
              {METHODS.map((candidate) => (
                <option key={candidate} value={candidate}>
                  {candidate}
                </option>
              ))}
              {!METHODS.includes(method) && <option value={method}>{method}</option>}
            </select>
            <input
              className="replay-url"
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              spellCheck={false}
              aria-label={t('replay.url')}
            />
          </div>

          <div className="replay-section">
            <div className="replay-label">
              <span>{t('replay.headers')}</span>
              <button
                type="button"
                className="ghost tiny"
                onClick={() => setHeaders((prev) => [...prev, { name: '', value: '' }])}
              >
                <Plus size={11} /> {t('replay.addHeader')}
              </button>
            </div>
            <div className="replay-headers">
              {headers.map((row, index) => (
                <div className="replay-header-row" key={index}>
                  <input
                    className="replay-header-name"
                    value={row.name}
                    onChange={(event) => setHeader(index, { name: event.target.value })}
                    placeholder="Header"
                    spellCheck={false}
                  />
                  <input
                    className="replay-header-value"
                    value={row.value}
                    onChange={(event) => setHeader(index, { value: event.target.value })}
                    placeholder="value"
                    spellCheck={false}
                  />
                  <button
                    type="button"
                    className="icon-btn"
                    onClick={() => setHeaders((prev) => prev.filter((_, i) => i !== index))}
                    aria-label={t('replay.removeHeader')}
                  >
                    <X size={12} />
                  </button>
                </div>
              ))}
            </div>
          </div>

          <div className="replay-section replay-body-section">
            <div className="replay-label">
              <span>{t('replay.body')}</span>
              <span className="replay-toolbar">
                {json !== undefined && json !== null && (
                  <>
                    <button type="button" className="ghost tiny" onClick={() => setBody(JSON.stringify(json, null, 2))}>
                      {t('replay.pretty')}
                    </button>
                    <button type="button" className="ghost tiny" onClick={() => setBody(JSON.stringify(json))}>
                      {t('replay.minify')}
                    </button>
                  </>
                )}
                {isUrlencoded && (
                  <span className="segmented">
                    <button type="button" className={mode === 'form' ? 'is-active' : ''} onClick={() => switchMode('form')}>
                      {t('replay.formMode')}
                    </button>
                    <button type="button" className={mode === 'raw' ? 'is-active' : ''} onClick={() => switchMode('raw')}>
                      {t('replay.rawMode')}
                    </button>
                  </span>
                )}
              </span>
            </div>

            {flow.reqBody?.base64 ? (
              <textarea className="replay-body" value={flow.reqBody.base64} readOnly rows={10} />
            ) : mode === 'form' ? (
              <div className="replay-fields">
                {formFields.map((field, index) => (
                  <div className="replay-field-row" key={index}>
                    <input
                      className="replay-header-name"
                      value={field.name}
                      onChange={(event) =>
                        setFormFields((prev) => prev.map((row, i) => (i === index ? { ...row, name: event.target.value } : row)))
                      }
                      placeholder={t('replay.fieldName')}
                      spellCheck={false}
                    />
                    <input
                      className="replay-header-value"
                      value={field.value ?? ''}
                      onChange={(event) =>
                        setFormFields((prev) => prev.map((row, i) => (i === index ? { ...row, value: event.target.value } : row)))
                      }
                      placeholder={t('replay.fieldValue')}
                      spellCheck={false}
                    />
                    <button
                      type="button"
                      className="icon-btn"
                      onClick={() => setFormFields((prev) => prev.filter((_, i) => i !== index))}
                      aria-label={t('replay.removeHeader')}
                    >
                      <X size={12} />
                    </button>
                  </div>
                ))}
                <button
                  type="button"
                  className="ghost tiny form-add"
                  onClick={() => setFormFields((prev) => [...prev, { name: '', value: '' }])}
                >
                  <Plus size={11} /> {t('replay.addField')}
                </button>
              </div>
            ) : (
              <textarea
                className="replay-body"
                value={body}
                onChange={(event) => setBody(event.target.value)}
                rows={12}
                spellCheck={false}
              />
            )}
            {flow.reqBody?.base64 && <div className="replay-hint">{t('replay.binaryBody')}</div>}
          </div>

          {error && <div className="note warn replay-error">{error}</div>}
        </div>

        <footer className="modal-actions replay-foot">
          <button type="button" className="ghost" onClick={onClose}>
            {t('replay.cancel')}
          </button>
          <button type="button" className="primary" disabled={sending || url.trim() === ''} onClick={() => void send()}>
            <Send size={13} /> {sending ? t('replay.sending') : t('replay.send')}
          </button>
        </footer>
      </section>
    </div>
  );
}
