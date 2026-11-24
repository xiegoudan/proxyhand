import { useEffect, useMemo, useRef, useState } from 'react';
import { Plus, Send, X } from 'lucide-react';
import type { Flow } from '../../../shared/protocol.ts';
import { useI18n } from '../i18n.tsx';
import { tryJson } from '../lib/format.ts';
import { parseForm, type FormField } from './FormView.tsx';
import { JsonEditor } from './JsonEditor.tsx';
import { Button } from './ui/button.tsx';
import { Checkbox } from './ui/checkbox.tsx';
import { Dialog, DialogContent, DialogTitle } from './ui/dialog.tsx';
import { Input } from './ui/input.tsx';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select.tsx';
import { Textarea } from './ui/textarea.tsx';
import { cn } from '../lib/utils.ts';

interface Props {
  flow: Flow;
  onClose: () => void;
  onSent: (id: number) => void;
}

interface HeaderRow {
  name: string;
  value: string;
  enabled: boolean;
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
  return hit && hit.enabled ? hit.value.trim() : null;
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
      .map((pair) => ({ name: pair.name, value: pair.value, enabled: true })),
  );
  const [body, setBody] = useState(initialBody);
  const [mode, setMode] = useState<'raw' | 'form'>('raw');
  const [formFields, setFormFields] = useState<FormField[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const contentType = headerValue(headers, 'content-type');
  const form = useMemo(() => parseForm(contentType ?? undefined, body), [contentType, body]);
  const isUrlencoded = form?.kind === 'urlencoded';
  const json = useMemo(() => tryJson(body), [body]);
  const jsonEditable = json !== undefined && json !== null;
  // Once the body is (or started as) JSON, keep the CodeMirror editor mounted for
  // the rest of the dialog's life. Gating on the live parse result would unmount
  // the editor (and drop the caret) the instant the user momentarily produces
  // invalid JSON while typing — which is what the editor's lint gutter is for.
  const jsonModeRef = useRef(jsonEditable);
  if (jsonEditable) jsonModeRef.current = true;
  const useJsonEditor = jsonModeRef.current && !isUrlencoded;

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
          .filter((row) => row.enabled && row.name.trim() !== '')
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
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="w-[min(960px,calc(100vw-40px))]">
        <DialogTitle className="flex items-center gap-2 px-4 pb-3 pt-4">
          {t('replay.title')}
          <span className="rounded-md bg-secondary px-1.5 py-0.5 font-mono text-[11px] font-normal text-secondary-foreground">
            {method} {flow.url}
          </span>
        </DialogTitle>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-4 pb-2">
          <div className="flex gap-2">
            <Select value={method} onValueChange={setMethod}>
              <SelectTrigger className="w-[118px] font-mono font-semibold uppercase" aria-label={t('replay.method')}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {METHODS.map((candidate) => (
                  <SelectItem key={candidate} value={candidate}>
                    {candidate}
                  </SelectItem>
                ))}
                {!METHODS.includes(method) && <SelectItem value={method}>{method}</SelectItem>}
              </SelectContent>
            </Select>
            <Input className="h-8 flex-1 font-mono text-[12.5px]" value={url} onChange={(e) => setUrl(e.target.value)} spellCheck={false} aria-label={t('replay.url')} />
          </div>

          <section className="space-y-1.5">
            <div className="flex items-center gap-2">
              <span className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">{t('replay.headers')}</span>
              <Button size="sm" variant="ghost" className="h-6 px-1.5 text-[11px]" onClick={() => setHeaders((prev) => [...prev, { name: '', value: '', enabled: true }])}>
                <Plus size={11} /> {t('replay.addHeader')}
              </Button>
            </div>
            <div className="space-y-1">
              {headers.map((row, index) => (
                <div className="flex items-center gap-1.5" key={index}>
                  <Checkbox
                    checked={row.enabled}
                    onChange={(e) => setHeader(index, { enabled: e.target.checked })}
                    aria-label={t('replay.headerEnabled')}
                  />
                  <Input
                    className={cn('h-7 w-[34%] font-mono text-[12px]', !row.enabled && 'opacity-40')}
                    value={row.name}
                    onChange={(e) => setHeader(index, { name: e.target.value })}
                    placeholder="Header"
                    spellCheck={false}
                  />
                  <Input
                    className={cn('h-7 flex-1 font-mono text-[12px]', !row.enabled && 'opacity-40')}
                    value={row.value}
                    onChange={(e) => setHeader(index, { value: e.target.value })}
                    placeholder="value"
                    spellCheck={false}
                  />
                  <Button size="iconSm" variant="ghost" onClick={() => setHeaders((prev) => prev.filter((_, i) => i !== index))} aria-label={t('replay.removeHeader')}>
                    <X size={12} />
                  </Button>
                </div>
              ))}
            </div>
          </section>

          <section className="space-y-1.5">
            <div className="flex items-center gap-2">
              <span className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">{t('replay.body')}</span>
              <span className="flex items-center gap-1">
                {jsonEditable && (
                  <>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[11px]"
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => setBody(JSON.stringify(json, null, 2))}
                    >
                      {t('replay.pretty')}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-6 px-1.5 text-[11px]"
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => setBody(JSON.stringify(json))}
                    >
                      {t('replay.minify')}
                    </Button>
                  </>
                )}
                {isUrlencoded && (
                  <span className="inline-flex items-center gap-0.5 rounded-md bg-secondary p-0.5">
                    <button type="button" className={cn('seg-btn', mode === 'form' && 'is-active')} onClick={() => switchMode('form')}>
                      {t('replay.formMode')}
                    </button>
                    <button type="button" className={cn('seg-btn', mode === 'raw' && 'is-active')} onClick={() => switchMode('raw')}>
                      {t('replay.rawMode')}
                    </button>
                  </span>
                )}
              </span>
            </div>

            {flow.reqBody?.base64 ? (
              <Textarea value={flow.reqBody.base64} readOnly rows={10} className="font-mono text-[11.5px]" />
            ) : mode === 'form' ? (
              <div className="space-y-1">
                {formFields.map((field, index) => (
                  <div className="flex items-center gap-1.5" key={index}>
                    <Input
                      className="h-7 w-[34%] font-mono text-[12px]"
                      value={field.name}
                      onChange={(e) => setFormFields((prev) => prev.map((row, i) => (i === index ? { ...row, name: e.target.value } : row)))}
                      placeholder={t('replay.fieldName')}
                      spellCheck={false}
                    />
                    <Input
                      className="h-7 flex-1 font-mono text-[12px]"
                      value={field.value ?? ''}
                      onChange={(e) => setFormFields((prev) => prev.map((row, i) => (i === index ? { ...row, value: e.target.value } : row)))}
                      placeholder={t('replay.fieldValue')}
                      spellCheck={false}
                    />
                    <Button size="iconSm" variant="ghost" onClick={() => setFormFields((prev) => prev.filter((_, i) => i !== index))} aria-label={t('replay.removeHeader')}>
                      <X size={12} />
                    </Button>
                  </div>
                ))}
                <Button size="sm" variant="ghost" className="h-6 px-1.5 text-[11px]" onClick={() => setFormFields((prev) => [...prev, { name: '', value: '' }])}>
                  <Plus size={11} /> {t('replay.addField')}
                </Button>
              </div>
            ) : useJsonEditor ? (
              <JsonEditor value={body} onChange={setBody} minHeight={200} maxHeight={360} />
            ) : (
              <Textarea value={body} onChange={(e) => setBody(e.target.value)} rows={12} spellCheck={false} className="font-mono text-[12px] leading-relaxed" />
            )}
            {flow.reqBody?.base64 && <div className="text-[11px] text-muted-foreground">{t('replay.binaryBody')}</div>}
          </section>

          {error && (
            <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-[12px] text-destructive">{error}</div>
          )}
        </div>

        <footer className="flex items-center justify-end gap-2 border-t border-border px-4 py-3">
          <Button variant="ghost" onClick={onClose}>
            {t('replay.cancel')}
          </Button>
          <Button variant="default" disabled={sending || url.trim() === ''} onClick={() => void send()}>
            <Send size={13} /> {sending ? t('replay.sending') : t('replay.send')}
          </Button>
        </footer>
      </DialogContent>
    </Dialog>
  );
}
