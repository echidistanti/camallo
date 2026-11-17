// Run: node tests/detect_test.js  (loads src/detect.js against stubbed browser globals)
const fs = require('fs');
const assert = require('assert');
const sent = [];
const intervals = [];
globalThis.window = { ipc: { postMessage: m => sent.push(JSON.parse(m)) }, top: null };
window.top = window;
globalThis.location = { href: 'https://site.test/watch/1' };
globalThis.navigator = { userAgent: 'UA' };
let dom = { video: [], meta: [], a: [], iframe: [] };
let docHandlers = {};
globalThis.document = {
  title: 'Page One',
  addEventListener: (n, f) => { docHandlers[n] = f; },
  querySelectorAll: s =>
    s === 'video,source' ? dom.video : s.startsWith('meta') ? dom.meta : s === 'a[href]' ? dom.a : dom.iframe,
};
globalThis.setInterval = f => intervals.push(f);
globalThis.PerformanceObserver = class { observe() {} };
let fetchCt = {};
window.fetch = u => Promise.resolve({ url: u, headers: { get: () => fetchCt[u] || '' } });
globalThis.XMLHttpRequest = class {
  addEventListener(_, f) { this.h = f; }
  getResponseHeader() { return this.ct; }
};
XMLHttpRequest.prototype.open = function () {};

new Function(fs.readFileSync(__dirname + '/../src/detect.js', 'utf8'))();
const media = () => sent.filter(s => s.kind === 'media').map(s => s.url);

(async () => {
  await window.fetch('https://a.com/v/master.m3u8');
  await window.fetch('https://a.com/seg-12.ts');
  await window.fetch('https://a.com/manifest.json');
  fetchCt['https://a.com/api/play?id=1'] = 'application/vnd.apple.mpegurl';
  await window.fetch('https://a.com/api/play?id=1');
  fetchCt['https://a.com/part1.bin'] = 'video/mp2t';
  await window.fetch('https://a.com/part1.bin');
  await window.fetch('https://a.com/v/master.m3u8'); // duplicate

  await window.fetch('https://www.dailymotion.com/player/metadata/video/x8abc12?embedder=https%3A%2F%2Fsite.test');
  await window.fetch('https://www.dailymotion.com/cdn/manifest/video/x8abc12.m3u8?sec=1');
  const x = new XMLHttpRequest();
  x.open('GET', 'https://a.com/clip.mp4?t=1');
  const y = new XMLHttpRequest();
  y.ct = 'video/mp4'; y.responseURL = 'https://a.com/stream?id=2'; y.readyState = 2;
  y.open('GET', 'https://a.com/stream?id=2'); y.h();

  dom.video = [{ currentSrc: 'https://a.com/tag.webm' }];
  dom.meta = [{ content: 'https://a.com/og-video' }];
  dom.iframe = [{ src: 'https://www.youtube.com/embed/abc' }, { src: 'https://ads.test/frame' }];
  dom.a = [{ href: 'https://a.com/download.mkv' }, { href: 'https://a.com/page.html' }];
  intervals.forEach(f => f());
  document.title = 'Page One - late title'; // set after load
  intervals.forEach(f => f());
  document.title = 'Page Two';
  location.href = 'https://site.test/watch/2';
  intervals.forEach(f => f());

  docHandlers.playing();
  docHandlers.playing(); // throttled: reported once
  assert.strictEqual(sent.filter(s => s.kind === 'play').length, 1);
  assert.ok(window.__camallo && typeof window.__camallo.show === 'function', 'button hooks defined');

  assert.deepStrictEqual(media().sort(), [
    'https://a.com/api/play?id=1', 'https://a.com/clip.mp4?t=1', 'https://a.com/download.mkv',
    'https://a.com/og-video', 'https://a.com/stream?id=2', 'https://a.com/tag.webm',
    'https://a.com/v/master.m3u8', 'https://www.youtube.com/embed/abc',
    'https://www.dailymotion.com/video/x8abc12', 'https://www.dailymotion.com/cdn/manifest/video/x8abc12.m3u8?sec=1',
  ].sort());
  assert.deepStrictEqual(sent.filter(s => s.kind === 'page').map(s => s.url),
    ['https://site.test/watch/1', 'https://site.test/watch/2']);
  assert.deepStrictEqual(sent.filter(s => s.kind === 'page').map(s => s.title), ['Page One', 'Page Two']);
  // Only the late change is reported as a title; titles that came with a new page are not repeated.
  assert.deepStrictEqual(sent.filter(s => s.kind === 'title').map(s => s.title), ['Page One - late title']);
  console.log('detect ok:', media().length, 'media,', 2, 'pages');
})().catch(e => { console.error(e); process.exit(1); });
