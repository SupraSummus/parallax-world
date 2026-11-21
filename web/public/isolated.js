// A service worker that isolates the page (index.html registers it where
// it is not): each response it fetches, with the headers that let the
// page's threads share memory, which GitHub Pages does not send.
self.addEventListener('install', () => self.skipWaiting())
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()))
self.addEventListener('fetch', event => {
    // Such a request fails unless it is the page's own.
    if (event.request.cache === 'only-if-cached' && event.request.mode !== 'same-origin') return
    event.respondWith(fetch(event.request).then(response => {
        if (response.status === 0) return response
        const headers = new Headers(response.headers)
        headers.set('Cross-Origin-Opener-Policy', 'same-origin')
        headers.set('Cross-Origin-Embedder-Policy', 'require-corp')
        return new Response(response.body, { status: response.status, statusText: response.statusText, headers })
    }))
})
