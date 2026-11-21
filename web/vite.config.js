// The page isolated, so that its threads may share memory (web/worker.js):
// served so while developing and previewing; GitHub Pages sends no such
// headers, so there a service worker adds them (public/isolated.js).
const isolated = { 'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'require-corp' }

export default {
    server: { headers: isolated },
    preview: { headers: isolated },
}
