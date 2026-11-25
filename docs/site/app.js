// Bilingual toggle + install tabs + copy-to-clipboard. No dependencies.
(() => {
  const STORAGE = 'proxyhand:site-lang';
  const html = document.documentElement;

  // ── language ──────────────────────────────────────────────
  const applyLang = (lang) => {
    html.setAttribute('lang', lang === 'zh' ? 'zh-CN' : 'en');
    document.querySelectorAll('[data-en]').forEach((el) => {
      const text = el.getAttribute('data-' + lang);
      if (text != null) el.textContent = text;
    });
    const btn = document.getElementById('langBtn');
    if (btn) btn.textContent = lang === 'zh' ? 'EN' : '中文';
    try {
      localStorage.setItem(STORAGE, lang);
    } catch {}
  };

  let lang = 'en';
  try {
    lang = localStorage.getItem(STORAGE) || (navigator.language.startsWith('zh') ? 'zh' : 'en');
  } catch {
    lang = navigator.language && navigator.language.startsWith('zh') ? 'zh' : 'en';
  }
  applyLang(lang);

  const langBtn = document.getElementById('langBtn');
  if (langBtn) {
    langBtn.addEventListener('click', () => {
      lang = lang === 'zh' ? 'en' : 'zh';
      applyLang(lang);
    });
  }

  // ── install tabs ──────────────────────────────────────────
  document.querySelectorAll('.tab').forEach((tab) => {
    tab.addEventListener('click', () => {
      document.querySelectorAll('.tab').forEach((t) => t.classList.remove('is-active'));
      document.querySelectorAll('.panel').forEach((p) => p.classList.remove('is-active'));
      tab.classList.add('is-active');
      const panel = document.querySelector('.panel[data-panel="' + tab.dataset.tab + '"]');
      if (panel) panel.classList.add('is-active');
    });
  });

  // ── copy buttons ──────────────────────────────────────────
  document.querySelectorAll('.copy').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const text = btn.getAttribute('data-copy') || '';
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        const ta = document.createElement('textarea');
        ta.value = text;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        ta.remove();
      }
      const label = btn.getAttribute('data-' + lang) || 'Copy';
      const okLabel = lang === 'zh' ? '已复制' : 'Copied';
      btn.textContent = okLabel;
      btn.classList.add('done');
      setTimeout(() => {
        btn.textContent = label;
        btn.classList.remove('done');
      }, 1400);
    });
  });
})();
