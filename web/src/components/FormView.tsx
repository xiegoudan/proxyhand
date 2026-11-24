import { Copy } from 'lucide-react';
import { copyText } from '../lib/format.ts';
import { useI18n } from '../i18n.tsx';
import { Button } from './ui/button.tsx';

export interface FormField {
  name: string;
  value?: string;
  filename?: string;
  mime?: string;
  size?: number;
}

interface Parsed {
  kind: 'urlencoded' | 'multipart';
  fields: FormField[];
}

function mediaType(contentType: string | undefined): string {
  return (contentType ?? '').split(';')[0].trim().toLowerCase();
}

function decode(input: string): string {
  try {
    return decodeURIComponent(input.replace(/\+/g, ' '));
  } catch {
    return input;
  }
}

/** Splits a urlencoded payload into fields. */
function parseUrlEncoded(text: string): FormField[] {
  return text
    .split('&')
    .filter((pair) => pair.length > 0)
    .map((pair) => {
      const at = pair.indexOf('=');
      if (at < 0) return { name: decode(pair), value: '' };
      return { name: decode(pair.slice(0, at)), value: decode(pair.slice(at + 1)) };
    });
}

/** Text-level multipart walk: enough for names, values, filenames and part headers. */
function parseMultipart(text: string, boundary: string): FormField[] {
  const fields: FormField[] = [];
  const chunks = text.split(`--${boundary}`);
  for (const chunk of chunks) {
    if (!chunk.trim() || chunk.trim() === '--') continue;
    const crlf = chunk.indexOf('\r\n\r\n') >= 0;
    const headerEnd = crlf ? chunk.indexOf('\r\n\r\n') : chunk.indexOf('\n\n');
    if (headerEnd < 0) continue;
    const head = chunk.slice(0, headerEnd);
    const body = chunk.slice(headerEnd + (crlf ? 4 : 2));
    const name = /name="([^"]*)"/i.exec(head)?.[1];
    if (name === undefined) continue;
    const filename = /filename="([^"]*)"/i.exec(head)?.[1];
    const mime = /content-type:\s*([^\r\n;]+)/i.exec(head)?.[1]?.trim();
    const value = body.replace(/\r?\n$/, '');
    fields.push({
      name,
      value: filename ? undefined : value,
      filename,
      mime,
      size: new TextEncoder().encode(value).length,
    });
  }
  return fields;
}

/** Returns undefined when the body is not a form payload we can show as fields. */
export function parseForm(contentType: string | undefined, text: string | null): Parsed | undefined {
  if (!text) return undefined;
  const type = mediaType(contentType);
  if (type === 'application/x-www-form-urlencoded') {
    return { kind: 'urlencoded', fields: parseUrlEncoded(text) };
  }
  if (type === 'multipart/form-data') {
    // BodyView only carries the media type, so fall back to the delimiter the body itself starts with.
    let boundary = /boundary=("?)([^";]+)\1/i.exec(contentType ?? '')?.[2];
    if (!boundary) {
      const first = text.slice(0, text.indexOf('\n')).trim();
      if (first.startsWith('--')) boundary = first.slice(2);
    }
    if (!boundary) return undefined;
    const fields = parseMultipart(text, boundary);
    return fields.length > 0 ? { kind: 'multipart', fields } : undefined;
  }
  return undefined;
}

export function FormTable({ fields }: { fields: FormField[] }) {
  const { t } = useI18n();
  return (
    <div className="form-table">
      {fields.map((field, index) => (
        <div className="form-row" key={`${field.name}-${index}`}>
          <span className="form-name" title={field.name}>
            {field.name}
          </span>
          <span className="form-value">
            {field.filename !== undefined ? (
              <span className="form-file">
                <span className="form-file-name">{field.filename || t('detail.unnamedFile')}</span>
                {field.mime && <span className="form-file-meta">{field.mime}</span>}
                {field.size !== undefined && <span className="form-file-meta">{field.size} B</span>}
              </span>
            ) : (
              (field.value ?? '')
            )}
          </span>
          <Button
            size="iconSm"
            variant="ghost"
            title={t('detail.copy')}
            onClick={() => void copyText(field.value ?? field.filename ?? '')}
          >
            <Copy size={12} />
          </Button>
        </div>
      ))}
    </div>
  );
}
