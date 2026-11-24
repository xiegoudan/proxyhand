import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

export type Lang = 'en' | 'zh';

export const LANGS: Lang[] = ['en', 'zh'];

const LANG_KEY = 'proxyhand:lang';

/**
 * English source dictionary. Nested groups keep related copy together; `Dict` below requires the
 * Chinese dictionary to mirror every key, so a forgotten translation is a type error.
 */
const en = {
  app: {
    title: 'proxyhand — HTTP/SSE/WebSocket inspector',
  },
  lang: {
    label: 'Language',
    en: 'EN',
    zh: '中文',
    switchToEn: 'Switch to English',
    switchToZh: 'Switch to Simplified Chinese',
  },
  common: {
    capturePaused: 'capture paused',
  },
  kinds: {
    all: 'All',
    http: 'HTTP',
    sse: 'SSE',
    ws: 'WS',
  },
  header: {
    proxy: 'proxy',
    copyProxyAddress: 'copy proxy address',
    searchPlaceholder: 'Filter host, path, method…   /',
    statusFilter: 'status filter',
    statusAny: 'any status',
    statusSuccess: 'success',
    statusErrors: 'errors',
    statusPending: 'in flight',
    pauseCapture: 'pause capture',
    resumeCapture: 'resume capture',
    clearFlows: 'clear flows',
    exportHar: 'export all as HAR',
    trustInstructions: 'certificate trust instructions',
    live: 'live',
    disconnected: 'disconnected',
  },
  empty: {
    title: 'Select a flow',
    bodyBefore: 'Every request, SSE event and WebSocket frame that crosses the proxy shows up on the left. Use ',
    bodyAfter: ' to walk through them.',
    flows: 'flows',
    live: 'live',
  },
  statusbar: {
    flows: '{count} flows',
    capturing: 'capturing',
    downloadCa: 'download CA',
  },
  trust: {
    title: 'Trust the proxyhand CA',
    close: 'close',
    introBefore:
      'HTTPS interception requires the root CA to be trusted by the client you are debugging. The certificate itself is written to ',
    introAfter: '.',
    scopeIntro: 'This is the ',
    clientToProxy: 'client → proxyhand',
    scopeMid:
      ' direction only. It does not affect how proxyhand verifies upstream servers: that end trusts the OS certificate store when ',
    scopeMid2: ' is on, and a private CA needs ',
    scopeEnd: '.',
    copyPath: 'copy path',
    downloadCert: 'download ca.crt',
  },
  flowList: {
    columns: {
      status: 'Status',
      method: 'Method',
      path: 'Path',
      type: 'Type',
      size: 'Size',
      time: 'Time',
    },
    sortAscTitle: 'oldest first (click for newest first)',
    sortDescTitle: 'newest first (click for oldest first)',
    sortAscAria: 'sorted oldest first, switch to newest first',
    sortDescAria: 'sorted newest first, switch to oldest first',
    listLabel: 'captured flows',
    emptyTitle: 'No traffic yet.',
    emptyHint:
      'Point your browser or app at the proxy port, then browse. SSE and WebSocket frames appear here live.',
    events: '{count} ev',
    frames: '{count} fr',
  },
  detail: {
    tabs: {
      overview: 'Overview',
      request: 'Request',
      response: 'Response',
      frames: 'Frames',
    },
    pending: 'pending',
    copyCurl: 'Copy cURL',
    copy: 'Copy',
    copied: 'Copied',
    streaming: 'streaming',
    requestHeaders: 'Request headers',
    responseHeaders: 'Response headers',
    requestBody: 'Request body',
    responseBody: 'Response body',
    waitingFrames: 'Waiting for frames… new frames stream in live.',
    noFrames: 'This flow carries no streamed frames.',
    overview: {
      url: 'URL',
      started: 'Started',
      http: 'HTTP',
      status: 'Status',
      ttfb: 'TTFB',
      duration: 'Duration',
      requestBody: 'Request body',
      responseBody: 'Response body',
      client: 'Client',
      tls: 'TLS',
      plain: 'plain',
      error: 'Error',
    },
    timing: 'Timing',
    timingWait: 'wait {duration}',
    timingReceive: 'receive {duration}',
    wait: 'wait',
    receive: 'receive',
    headersFilter: 'filter…',
    none: 'none',
    tree: 'Tree',
    form: 'Form',
    raw: 'Raw',
    replay: 'Replay',
    close: 'Close',
    unnamedFile: '(unnamed file)',
    save: 'Save',
    truncated: 'Body truncated at the capture cap',
    truncatedEnd: '.',
    truncatedDropped: ' ({size} dropped).',
    binary: 'Binary payload',
    emptyBody: 'empty body',
    sseTitle: 'Server-Sent Events',
    wsTitle: 'WebSocket',
    filterFrames: 'filter frames…',
    follow: 'follow',
    resizeHint: 'Drag to resize',
    wsSent: '↑ {count} sent',
    wsReceived: '↓ {count} received',
    fragment: 'fragment',
    truncatedPill: 'truncated',
    binaryFrame: '(binary)',
  },
  replay: {
    title: 'Replay request',
    method: 'Method',
    url: 'URL',
    headers: 'Headers',
    addHeader: 'Add header',
    removeHeader: 'Remove header',
    headerEnabled: 'Include this header',
    body: 'Body',
    binaryBody: 'Binary body — sent as the base64 captured in the original flow',
    rawMode: 'Raw',
    formMode: 'Form',
    pretty: 'Pretty',
    minify: 'Minify',
    addField: 'Add field',
    fieldName: 'name',
    fieldValue: 'value',
    send: 'Send',
    sending: 'Sending…',
    cancel: 'Cancel',
  },
  json: {
    toggle: 'toggle',
    items: '{count} items',
    keys: '{count} keys',
    hidden: '… {count} more lines — switch to Raw to see the whole body',
  },
} as const;

/** Keep the key shape but widen literal values so another language can supply different copy. */
type WidenStrings<T> = {
  readonly [K in keyof T]: T[K] extends string ? string : WidenStrings<T[K]>;
};

type Dict = WidenStrings<typeof en>;

const zh: Dict = {
  app: {
    title: 'proxyhand — HTTP/SSE/WebSocket 抓包调试器',
  },
  lang: {
    label: '语言',
    en: 'EN',
    zh: '中文',
    switchToEn: '切换到英文',
    switchToZh: '切换到简体中文',
  },
  common: {
    capturePaused: '抓包已暂停',
  },
  kinds: {
    all: '全部',
    http: 'HTTP',
    sse: 'SSE',
    ws: 'WS',
  },
  header: {
    proxy: '代理',
    copyProxyAddress: '复制代理地址',
    searchPlaceholder: '筛选 host、路径、方法…   /',
    statusFilter: '状态筛选',
    statusAny: '任意状态',
    statusSuccess: '成功',
    statusErrors: '错误',
    statusPending: '进行中',
    pauseCapture: '暂停抓包',
    resumeCapture: '继续抓包',
    clearFlows: '清空请求',
    exportHar: '全部导出为 HAR',
    trustInstructions: '证书信任说明',
    live: '已连接',
    disconnected: '未连接',
  },
  empty: {
    title: '选择一个请求',
    bodyBefore: '经过代理的每个请求、SSE 事件和 WebSocket 帧都会显示在左侧。按 ',
    bodyAfter: ' 在列表中移动。',
    flows: '个请求',
    live: '个进行中',
  },
  statusbar: {
    flows: '{count} 个请求',
    capturing: '抓包中',
    downloadCa: '下载 CA',
  },
  trust: {
    title: '信任 proxyhand CA',
    close: '关闭',
    introBefore: 'HTTPS 抓包要求被调试的客户端信任根 CA。证书文件位于 ',
    introAfter: '。',
    scopeIntro: '这里只涉及',
    clientToProxy: '客户端 → proxyhand',
    scopeMid: ' 方向，不影响 proxyhand 校验上游服务器的方式：开启 ',
    scopeMid2: ' 时会信任操作系统的证书库；使用私有 CA 则需要 ',
    scopeEnd: '。',
    copyPath: '复制路径',
    downloadCert: '下载 ca.crt',
  },
  flowList: {
    columns: {
      status: '状态',
      method: '方法',
      path: '路径',
      type: '类型',
      size: '大小',
      time: '时间',
    },
    sortAscTitle: '最早在前（点击切换为最新在前）',
    sortDescTitle: '最新在前（点击切换为最早在前）',
    sortAscAria: '已按最早在前排序，点击切换为最新在前',
    sortDescAria: '已按最新在前排序，点击切换为最早在前',
    listLabel: '已抓取的请求',
    emptyTitle: '暂无流量。',
    emptyHint: '将浏览器或应用的代理指向该端口，然后开始浏览。SSE 和 WebSocket 帧会实时显示在这里。',
    events: '{count} 事件',
    frames: '{count} 帧',
  },
  detail: {
    tabs: {
      overview: '概览',
      request: '请求',
      response: '响应',
      frames: '帧',
    },
    pending: '等待中',
    copyCurl: '复制 cURL',
    copy: '复制',
    copied: '已复制',
    streaming: '流式传输中',
    requestHeaders: '请求头',
    responseHeaders: '响应头',
    requestBody: '请求体',
    responseBody: '响应体',
    waitingFrames: '等待帧… 新帧会实时流入。',
    noFrames: '此请求没有流式帧。',
    overview: {
      url: 'URL',
      started: '开始时间',
      http: 'HTTP',
      status: '状态',
      ttfb: 'TTFB',
      duration: '耗时',
      requestBody: '请求体',
      responseBody: '响应体',
      client: '客户端',
      tls: 'TLS',
      plain: '明文',
      error: '错误',
    },
    timing: '时序',
    timingWait: '等待 {duration}',
    timingReceive: '接收 {duration}',
    wait: '等待',
    receive: '接收',
    headersFilter: '筛选…',
    none: '无',
    tree: '树形',
    form: '表单',
    raw: '原始',
    replay: '重放',
    close: '关闭',
    unnamedFile: '（未命名文件）',
    save: '保存',
    truncated: '内容已在抓取上限处截断',
    truncatedEnd: '。',
    truncatedDropped: '（丢弃 {size}）。',
    binary: '二进制内容',
    emptyBody: '空内容',
    sseTitle: 'Server-Sent Events',
    wsTitle: 'WebSocket',
    filterFrames: '筛选帧…',
    follow: '跟随',
    resizeHint: '拖动可调整宽度',
    wsSent: '↑ 已发送 {count}',
    wsReceived: '↓ 已接收 {count}',
    fragment: '分片',
    truncatedPill: '已截断',
    binaryFrame: '（二进制）',
  },
  replay: {
    title: '重放请求',
    method: '方法',
    url: 'URL',
    headers: '请求头',
    addHeader: '添加',
    removeHeader: '删除',
    headerEnabled: '发送该请求头',
    body: '请求体',
    binaryBody: '二进制请求体：按原请求捕获的 base64 原样发送',
    rawMode: '原始',
    formMode: '表单',
    pretty: '格式化',
    minify: '压缩',
    addField: '添加字段',
    fieldName: '字段名',
    fieldValue: '值',
    send: '发送',
    sending: '发送中…',
    cancel: '取消',
  },
  json: {
    toggle: '展开或折叠',
    items: '{count} 项',
    keys: '{count} 个键',
    hidden: '… 还有 {count} 行，切到「原始」看完整内容',
  },
};

/** Pick the browser's preferred UI language the first time, then remember the explicit choice. */
export function navChromeLang(): Lang {
  try {
    const stored = localStorage.getItem(LANG_KEY);
    if (stored === 'en' || stored === 'zh') return stored;
  } catch {
    /* private mode: fall through to the browser preference */
  }
  const preferred = navigator.languages?.[0] ?? navigator.language ?? '';
  return preferred.toLowerCase().startsWith('zh') ? 'zh' : 'en';
}

function flatten(source: unknown, prefix = '', out: Record<string, string> = {}): Record<string, string> {
  if (!source || typeof source !== 'object') return out;
  for (const [key, value] of Object.entries(source as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof value === 'string') out[path] = value;
    else if (value && typeof value === 'object') flatten(value, path, out);
  }
  return out;
}

const DICTS: Record<Lang, Record<string, string | undefined>> = {
  en: flatten(en),
  zh: flatten(zh),
};

function interpolate(template: string, vars?: Record<string, string | number>): string {
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (placeholder, name: string) =>
    Object.prototype.hasOwnProperty.call(vars, name) ? String(vars[name]) : placeholder,
  );
}

export interface I18nValue {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: (key: string, vars?: Record<string, string | number>) => string;
}

const I18nContext = createContext<I18nValue | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(navChromeLang);

  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    try {
      localStorage.setItem(LANG_KEY, next);
    } catch {
      /* private mode: keep the in-memory choice only */
    }
  }, []);

  const t = useCallback(
    (key: string, vars?: Record<string, string | number>) => {
      const template = DICTS[lang][key] ?? DICTS.en[key] ?? key;
      return interpolate(template, vars);
    },
    [lang],
  );

  useEffect(() => {
    document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
    document.title = t('app.title');
  }, [lang, t]);

  const value = useMemo<I18nValue>(() => ({ lang, setLang, t }), [lang, setLang, t]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  const value = useContext(I18nContext);
  if (!value) throw new Error('useI18n must be used inside <I18nProvider>');
  return value;
}
