const CACHE = 'ourspace-v1';
const PRECACHE = [
  './',
  './index.html',
  './app-interactive.js',
  './app-interactive_bg.wasm',
];

self.addEventListener('install', (e) => {
  e.waitUntil(
    caches.open(CACHE).then((c) => c.addAll(PRECACHE)).then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)))
    ).then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET') return;
  // Baked assets: cache-first (immutable content, content-addressed by build).
  if (url.pathname.includes('/assets/processed/')) {
    e.respondWith(
      caches.match(e.request).then((hit) =>
        hit || fetch(e.request).then((res) => {
          const clone = res.clone();
          caches.open(CACHE).then((c) => c.put(e.request, clone));
          return res;
        })
      )
    );
    return;
  }
  // WASM + JS + HTML: stale-while-revalidate (fast repeat, fresh on update).
  if (url.pathname.endsWith('.wasm') || url.pathname.endsWith('.js') || url.pathname.endsWith('.html')) {
    e.respondWith(
      caches.match(e.request).then((hit) => {
        const network = fetch(e.request).then((res) => {
          const clone = res.clone();
          caches.open(CACHE).then((c) => c.put(e.request, clone));
          return res;
        });
        return hit || network;
      })
    );
    return;
  }
});
