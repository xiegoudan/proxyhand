// Bilingual toggle + install tabs + copy-to-clipboard + scroll reveal. No dependencies.
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

  // ── clipboard helper ──────────────────────────────────────
  const copyText = async (text) => {
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
  };

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
      await copyText(btn.getAttribute('data-copy') || '');
      const label = btn.getAttribute('data-' + lang) || 'Copy';
      btn.textContent = lang === 'zh' ? '已复制' : 'Copied';
      btn.classList.add('is-done');
      setTimeout(() => {
        btn.textContent = label;
        btn.classList.remove('is-done');
      }, 1400);
    });
  });

  // ── hero quick-copy install command ───────────────────────
  document.querySelectorAll('[data-quickcopy]').forEach((btn) => {
    btn.addEventListener('click', async () => {
      await copyText('cargo install --git https://github.com/xiegoudan/proxyhand');
      const ic = btn.querySelector('.copy-ic');
      if (ic) {
        const prev = ic.innerHTML;
        ic.innerHTML = '<path d="m5 12 5 5 9-9"/>';
        setTimeout(() => (ic.innerHTML = prev), 1300);
      }
    });
  });

  // ── scroll reveal ─────────────────────────────────────────
  const reveals = document.querySelectorAll('.reveal');
  if (reveals.length && 'IntersectionObserver' in window) {
    const io = new IntersectionObserver(
      (entries) => {
        entries.forEach((e) => {
          if (e.isIntersecting) {
            e.target.classList.add('is-visible');
            io.unobserve(e.target);
          }
        });
      },
      { rootMargin: '0px 0px -12% 0px', threshold: 0.08 }
    );
    reveals.forEach((el) => io.observe(el));
  } else {
    reveals.forEach((el) => el.classList.add('is-visible'));
  }
})();
