// Injected in every frame of the built-in browser. Reports media candidates as JSON via window.ipc.
(() => {
  const seen = new Set();
  const EXT = /\.(m3u8|mpd|mp4|webm|m4v|mov|mkv|flv)(\?|#|$)|\.ism\/manifest/i;
  const SEG = /(^|[\/_.-])(seg|segment|chunk|frag|fragment)[-_.]?\d|\.(ts|m4s|m4f|m4a|aac|cmfv|cmfa)(\?|#|$)/i;
  const CT = /^(video\/(?!mp2t|iso\.segment)|audio\/(x-)?mpegurl|application\/(vnd\.apple\.mpegurl|x-mpegurl|dash\+xml|vnd\.ms-sstr\+xml))/i;
  const EMB = /(youtube(-nocookie)?\.com\/embed|player\.vimeo\.com\/video|dailymotion\.com\/embed|streamable\.com\/e\/)/i;

  const send = (kind, url, extra) =>
    window.ipc.postMessage(JSON.stringify(Object.assign({ kind, url, referer: location.href, ua: navigator.userAgent }, extra)));

  // Dailymotion players get the video id by script, so it only shows up in their own requests.
  const DM = /dailymotion\.com\/(?:player\/metadata\/video|cdn\/manifest\/video|embed\/video|video)\/([a-z0-9]+)/i;

  function add(u, force) {
    try { u = new URL(u, location.href).href; } catch (e) { return; }
    const c = DM.exec(u);
    if (c && u !== 'https://www.dailymotion.com/video/' + c[1]) add('https://www.dailymotion.com/video/' + c[1], true);
    if (!/^https?:/.test(u) || seen.has(u) || SEG.test(u) || (!force && !EXT.test(u))) return;
    seen.add(u);
    send('media', u);
  }
  // Content-Type catches streams whose URL has no telltale extension.
  const ct = (u, t) => { if (t && CT.test(t)) add(u, true); };

  const F = window.fetch;
  window.fetch = function (i) {
    let u;
    try { u = typeof i === 'string' ? i : (i && i.url) || String(i); add(u); } catch (e) {}
    const p = F.apply(this, arguments);
    p.then(r => { try { ct(r.url || u, r.headers.get('content-type')); } catch (e) {} }, () => {});
    return p;
  };

  const O = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (m, u) {
    try { add(String(u)); } catch (e) {}
    this.addEventListener('readystatechange', () => {
      try { if (this.readyState === 2) ct(this.responseURL, this.getResponseHeader('content-type')); } catch (e) {}
    });
    return O.apply(this, arguments);
  };

  try { new PerformanceObserver(l => l.getEntries().forEach(e => add(e.name))).observe({ type: 'resource', buffered: true }); } catch (e) {}

  // A video started playing in this frame (media events do not bubble, hence capture).
  let lastPlay = 0;
  const played = () => {
    if (Date.now() - lastPlay > 1000) { lastPlay = Date.now(); send('play', location.href); }
  };
  document.addEventListener('playing', played, true);

  const scan = () => {
    document.querySelectorAll('video').forEach(v => { if (!v.paused && v.currentTime > 0) played(); });
    document.querySelectorAll('video,source').forEach(v => add(v.currentSrc || v.src));
    document.querySelectorAll('meta[property="og:video"],meta[property="og:video:url"],meta[property="og:video:secure_url"]')
      .forEach(m => m.content && add(m.content, true));
    document.querySelectorAll('a[href]').forEach(a => add(a.href));
    document.querySelectorAll('iframe[src]').forEach(f => EMB.test(f.src) && add(f.src, true));
  };
  setInterval(scan, 1500);

  if (window.top === window) {
    // The page itself is a candidate too: yt-dlp may know the site even when no stream is visible.
    // The title names downloads of plain streams. Many sites set it after the page loaded, so it is watched too.
    let last = '', lastTitle = '';
    const reportPage = () => {
      if (location.href !== last && /^https?:/.test(location.href)) {
        last = location.href;
        lastTitle = document.title || '';
        send('page', last, { title: lastTitle });
      }
    };
    const reportTitle = () => {
      const t = document.title || '';
      if (last && t !== lastTitle) { lastTitle = t; window.ipc.postMessage(JSON.stringify({ kind: 'title', title: t })); }
    };
    reportPage();
    setInterval(reportPage, 1000);
    setInterval(reportTitle, 1000);

    // Floating button, driven by the Camallo browser process (show / hide / toast).
    let host, btn, timer;
    const make = () => {
      if (host || !document.documentElement) return;
      host = document.createElement('div');
      host.style.cssText = 'all:initial;position:fixed;top:14px;right:14px;z-index:2147483647;display:none';
      const root = host.attachShadow({ mode: 'closed' });
      btn = document.createElement('button');
      btn.textContent = 'Download video';
      btn.style.cssText = 'font:600 14px system-ui,sans-serif;padding:10px 16px;border:0;border-radius:8px;' +
        'background:#e8590c;color:#fff;cursor:pointer;box-shadow:0 2px 10px rgba(0,0,0,.4)';
      btn.onclick = () => window.ipc.postMessage(JSON.stringify({ kind: 'download' }));
      root.appendChild(btn);
      document.documentElement.appendChild(host);
    };
    window.__camallo = {
      show() { make(); if (host) host.style.display = 'block'; },
      hide() { if (host) host.style.display = 'none'; },
      toast(t) {
        make();
        if (!btn) return;
        btn.textContent = t;
        clearTimeout(timer);
        timer = setTimeout(() => { btn.textContent = 'Download video'; }, 2500);
      },
    };
  }
})();
