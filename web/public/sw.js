// LabSentinel Service Worker（任务115）：离线 app shell。
// 规则：导航请求 network-first（离线回退缓存 index）；静态 assets cache-first；
// **/api/* 与 /ws/* 一律直连网络，绝不缓存**（事件/截图必须实时）。版本更新时 bump CACHE。
const CACHE = 'labsentinel-shell-v1'
const SHELL = ['/', '/index.html', '/manifest.webmanifest', '/icons/icon-192.png', '/icons/icon-512.png']

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()))
})

self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  )
})

self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url)
  if (e.request.method !== 'GET' || url.origin !== self.location.origin) return
  if (url.pathname.startsWith('/api/') || url.pathname.startsWith('/ws')) return // 直连，不缓存
  if (e.request.mode === 'navigate') {
    // 导航：network-first，离线回退缓存 index（app shell）
    e.respondWith(
      fetch(e.request)
        .then((r) => {
          if (!r.ok) return r // T-506（B-25）：非 2xx（500/离线网关页）不污染缓存
          const copy = r.clone()
          caches.open(CACHE).then((c) => c.put('/index.html', copy))
          return r
        })
        .catch(() => caches.match('/index.html')),
    )
    return
  }
  if (url.pathname.startsWith('/assets/')) {
    // 构建产物带 hash：cache-first
    e.respondWith(
      caches.match(e.request).then(
        (hit) =>
          hit ||
          fetch(e.request).then((r) => {
            const copy = r.clone()
            caches.open(CACHE).then((c) => c.put(e.request, copy))
            return r
          }),
      ),
    )
  }
})
